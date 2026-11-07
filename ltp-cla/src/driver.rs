//! Background runtime driver coordinating the LTP sans-I/O Engine with network I/O.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream, UdpSocket};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, trace, warn};

use cla_commons::pacing::LeakyBucket;
use hardy_async::TaskPool;
use hardy_bpa::cla::{Acceptance, ForwardBundleResult, Sink, TransferOutcome};
use hardy_bpv7::bundle::Id;
use hardy_bpv7::eid::NodeId;
use hardy_ltp::engine::{Engine, EngineConfig, Notice};
use hardy_ltp::kiss::{Command, decode_frame, encode_frame};
use hardy_ltp::segment::{Extension, Segment};
use hardy_ltp::types::{EngineId, SessionId};

use crate::cla::engine_id_to_cla_address;
use crate::config::{Config, TransportMode};
use crate::error::{Error, Result};

/// Commands sent to the driver actor from the CLA interface.
pub enum DriverCommand {
    /// Initiate block transmission for a forwarded bundle.
    Forward {
        bundle_id: Id,
        destination_engine: EngineId,
        data: Bytes,
        red_len: u64,
        reply_tx: oneshot::Sender<Result<ForwardBundleResult>>,
    },
    /// Register a mapping between a remote Engine ID, its network address, and BPA NodeId.
    RegisterPeer {
        engine_id: EngineId,
        addr: SocketAddr,
        node_id: Option<NodeId>,
    },
    /// Unregister a remote peer and remove its routing associations.
    UnregisterPeer { engine_id: EngineId },
    /// Configure or dynamically update the RTT and ACK margin for a remote peer engine.
    SetPeerTiming {
        engine_id: EngineId,
        rtt: Duration,
        ack_margin: Duration,
    },
    /// Gracefully shutdown the driver task.
    Shutdown,
}

#[derive(Debug)]
struct GreenReassembly {
    _source: EngineId,
    data: Vec<u8>,
    received_bytes: usize,
    total_len: Option<usize>,
    last_update: Instant,
}

/// The background task running the LTP engine.
pub struct Driver {
    config: Config,
    engine: Engine,
    sink: Arc<dyn Sink>,
    tasks: Arc<TaskPool>,
    cmd_rx: mpsc::Receiver<DriverCommand>,
    start_time: Instant,
    engine_to_addr: HashMap<EngineId, SocketAddr>,
    addr_to_engine: HashMap<SocketAddr, EngineId>,
    engine_to_node: HashMap<EngineId, NodeId>,
    pending_transfers: HashMap<SessionId, Id>,
    green_reassemblies: HashMap<SessionId, GreenReassembly>,
}

/// Minimum frame length required by standard packet radio TNCs (e.g. Direwolf).
/// Ensures an LTP segment is padded to at least `min_frame_len` bytes using an
/// RFC 5326 trailer extension so that it is accepted by packet radio TNCs (such as Direwolf).
/// If `min_frame_len` is 0 or if the segment is already long enough, no padding is applied.
#[must_use]
pub fn pad_segment_for_tnc(bytes: &[u8], min_frame_len: usize) -> std::borrow::Cow<'_, [u8]> {
    if min_frame_len == 0 || bytes.len() >= min_frame_len {
        return std::borrow::Cow::Borrowed(bytes);
    }

    if let Ok((mut seg, consumed)) = Segment::parse(bytes)
        && consumed == bytes.len()
        && seg.trailer_ext.len() < 15
    {
        let cur_len = bytes.len();
        let pad_len = min_frame_len.saturating_sub(cur_len + 2);
        let pad_zeros = vec![0u8; pad_len];
        seg.trailer_ext.push(Extension {
            tag: 0,
            value: &pad_zeros,
        });
        let padded = seg.encode_to_vec();
        return std::borrow::Cow::Owned(padded);
    }

    std::borrow::Cow::Borrowed(bytes)
}

#[derive(Debug, Clone)]
pub(crate) struct KissPacingConfig {
    pub min_frame_len: usize,
    pub rate_limit_bytes_per_sec: Option<u64>,
    pub burst_capacity_bytes: usize,
    pub half_duplex: bool,
    pub turnaround_mode: crate::config::TurnaroundMode,
    pub turnaround_timeout: Duration,
    pub dwell_time: Duration,
}

fn is_checkpoint_segment(bytes: &[u8]) -> bool {
    if let Some(&first) = bytes.first() {
        matches!(first & 0x0F, 1..=3)
    } else {
        false
    }
}

fn is_report_segment(bytes: &[u8]) -> bool {
    if let Some(&first) = bytes.first() {
        (first & 0x0F) == 8
    } else {
        false
    }
}

struct HalfDuplexPacer {
    enabled: bool,
    mode: crate::config::TurnaroundMode,
    turnaround_timeout: Duration,
    dwell_time: Duration,
    pause_deadline: Option<tokio::time::Instant>,
}

impl HalfDuplexPacer {
    fn new(config: &KissPacingConfig) -> Self {
        Self {
            enabled: config.half_duplex,
            mode: config.turnaround_mode,
            turnaround_timeout: config.turnaround_timeout,
            dwell_time: config.dwell_time,
            pause_deadline: None,
        }
    }

    fn is_paused(&self, now: tokio::time::Instant) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(dl) = self.pause_deadline {
            now < dl
        } else {
            false
        }
    }

    fn on_checkpoint_sent(&mut self) {
        if !self.enabled {
            return;
        }
        let duration = match self.mode {
            crate::config::TurnaroundMode::WaitForReport => self.turnaround_timeout,
            crate::config::TurnaroundMode::FixedDwell => self.dwell_time,
        };
        self.pause_deadline = Some(tokio::time::Instant::now() + duration);
        debug!(
            "Half-duplex: Checkpoint emitted, pausing transmission queue for {:?}",
            duration
        );
    }

    fn on_report_received(&mut self) {
        if !self.enabled {
            return;
        }
        if self.mode == crate::config::TurnaroundMode::WaitForReport
            && self.pause_deadline.is_some()
        {
            debug!(
                "Half-duplex: Report Segment received, unpausing transmission queue immediately"
            );
            self.pause_deadline = None;
        }
    }

    fn next_deadline(&self) -> Option<tokio::time::Instant> {
        if !self.enabled {
            None
        } else {
            self.pause_deadline
        }
    }
}

impl Driver {
    /// Spawns the driver task based on configuration.
    pub fn spawn(
        config: Config,
        sink: Arc<dyn Sink>,
        cmd_rx: mpsc::Receiver<DriverCommand>,
        addr_tx: oneshot::Sender<std::io::Result<SocketAddr>>,
        tasks: Arc<TaskPool>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let start_time = Instant::now();
            let engine_cfg = EngineConfig {
                engine_id: config.engine_id,
                max_segment_payload_len: config.max_segment_payload_len,
                checkpoint_rtx_limit: config.checkpoint_rtx_limit,
                report_rtx_limit: config.report_rtx_limit,
                rtt_micros: u64::try_from(config.rtt.as_micros()).unwrap_or(u64::MAX),
                ack_margin_micros: u64::try_from(config.ack_margin.as_micros()).unwrap_or(u64::MAX),
                allowed_client_services: Some(vec![1]),
                ..Default::default()
            };
            let mut engine = Engine::new(engine_cfg);
            for peer in &config.peers {
                if let (Some(rtt), Some(ack_margin)) = (peer.rtt, peer.ack_margin) {
                    let rtt_micros = u64::try_from(rtt.as_micros()).unwrap_or(u64::MAX);
                    let ack_margin_micros =
                        u64::try_from(ack_margin.as_micros()).unwrap_or(u64::MAX);
                    engine.set_peer_timing(peer.engine_id, rtt_micros, ack_margin_micros);
                }
            }

            let mut driver = Self {
                config,
                engine,
                sink,
                tasks,
                cmd_rx,
                start_time,
                engine_to_addr: HashMap::new(),
                addr_to_engine: HashMap::new(),
                engine_to_node: HashMap::new(),
                pending_transfers: HashMap::new(),
                green_reassemblies: HashMap::new(),
            };

            if let Err(err) = driver.run(addr_tx).await {
                error!("LTP driver exited with error: {err}");
            }

            // Clean shutdown: notify all remaining pending transfers with Failed outcome
            for (_session, bundle_id) in driver.pending_transfers.drain() {
                let _ = driver
                    .sink
                    .transfer_outcome(&bundle_id, TransferOutcome::Failed)
                    .await;
            }
        })
    }

    fn now_micros(&self) -> u64 {
        u64::try_from(self.start_time.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    async fn run(&mut self, addr_tx: oneshot::Sender<std::io::Result<SocketAddr>>) -> Result<()> {
        match self.config.transport.clone() {
            TransportMode::Udp { bind_addr } => self.run_udp(bind_addr, addr_tx).await,
            TransportMode::KissTcp {
                server_addr,
                bind_addr,
                port,
                reconnect_initial_delay,
                reconnect_max_delay,
                min_frame_len,
                rate_limit_bytes_per_sec,
                burst_capacity_bytes,
                half_duplex,
                turnaround_mode,
                turnaround_timeout,
                dwell_time,
            } => {
                let pacing = KissPacingConfig {
                    min_frame_len,
                    rate_limit_bytes_per_sec,
                    burst_capacity_bytes,
                    half_duplex,
                    turnaround_mode,
                    turnaround_timeout,
                    dwell_time,
                };
                self.run_kiss_tcp(
                    server_addr,
                    bind_addr,
                    port,
                    reconnect_initial_delay,
                    reconnect_max_delay,
                    pacing,
                    addr_tx,
                )
                .await
            }
        }
    }

    async fn run_udp(
        &mut self,
        bind_addr: SocketAddr,
        addr_tx: oneshot::Sender<std::io::Result<SocketAddr>>,
    ) -> Result<()> {
        let socket = match UdpSocket::bind(bind_addr).await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                let _ = addr_tx.send(Err(e));
                return Ok(());
            }
        };
        let local_addr = match socket.local_addr() {
            Ok(a) => a,
            Err(e) => {
                let _ = addr_tx.send(Err(e));
                return Ok(());
            }
        };
        let _ = addr_tx.send(Ok(local_addr));
        info!("LTP UDP transport listening on {local_addr}");

        let mut rx_buf = vec![0u8; 65536];
        let cancel = self.tasks.cancel_token().clone();

        loop {
            let now = self.now_micros();
            let next_timer = self.engine.next_timer_deadline();

            // Calculate duration to next deadline
            let timer_future = async {
                match next_timer {
                    Some(deadline) if deadline > now => {
                        tokio::time::sleep(Duration::from_micros(deadline - now)).await;
                    }
                    Some(_) => {
                        // Expired or due immediately
                    }
                    None => {
                        // Indefinite sleep until interrupted by other events
                        std::future::pending::<()>().await;
                    }
                }
            };

            tokio::select! {
                _ = cancel.cancelled() => {
                    debug!("LTP UDP driver loop cancelled");
                    break;
                }
                // 1. Ingress UDP packet
                recv_res = socket.recv_from(&mut rx_buf) => {
                    let (len, peer_addr) = match recv_res {
                        Ok(res) => res,
                        Err(err) => {
                            warn!("LTP UDP recv_from error (transient): {err}");
                            continue;
                        }
                    };

                    if len == 4 && rx_buf[..4] == [0, 0, 0, 0] {
                        // RFC 7122 §3.3 & §3.4: 4-octet keep-alive packet MUST be discarded
                        trace!("discarding 4-octet keep-alive datagram from {peer_addr}");
                        continue;
                    }

                    let now = self.now_micros();

                    // Resolve peer engine ID using peek_session to avoid full allocation
                    let peer_engine = if let Some(&id) = self.addr_to_engine.get(&peer_addr) {
                        if self.engine_to_addr.contains_key(&id) {
                            self.engine_to_addr.insert(id, peer_addr);
                        }
                        id
                    } else if let Ok(session) = Segment::peek_session(&rx_buf[..len]) {
                        let id = if session.originator != self.config.engine_id {
                            session.originator
                        } else if let Some(peer) = self.engine.peer_for_session(&session) {
                            peer
                        } else if self.engine_to_addr.len() == 1 {
                            *self.engine_to_addr.keys().next().unwrap()
                        } else {
                            warn!("received packet from unregistered UDP peer {peer_addr}");
                            EngineId(peer_addr.port() as u64)
                        };
                        let is_new_peer = !self.engine_to_addr.contains_key(&id);
                        if self.addr_to_engine.len() >= 1024 {
                            self.addr_to_engine.clear();
                        }
                        self.addr_to_engine.insert(peer_addr, id);
                        self.engine_to_addr.insert(id, peer_addr);

                        if is_new_peer {
                            info!("discovered new dynamic LTP peer Engine {id:?} at {peer_addr}");
                            let cla_addr = engine_id_to_cla_address(id);
                            let sink = self.sink.clone();
                            self.tasks.spawn(async move {
                                let _ = sink.add_peer(cla_addr, &[]).await;
                            });
                        }
                        id
                    } else {
                        warn!("received packet from unregistered UDP peer {peer_addr}");
                        EngineId(peer_addr.port() as u64)
                    };

                    if let Err(err) = self.engine.handle_incoming_segment(peer_engine, &rx_buf[..len], now) {
                        trace!("failed to parse LTP segment from {peer_addr}: {err}");
                    }
                }

                // 2. Timer expiration
                _ = timer_future => {
                    let now = self.now_micros();
                    self.engine.handle_timer_tick(now);
                }

                // 3. Command from CLA
                cmd = self.cmd_rx.recv() => {
                    let Some(cmd) = cmd else {
                        break;
                    };
                    if !self.handle_command(cmd).await? {
                        break;
                    }
                }
            }

            // Drain outputs and notices after any event
            self.drain_outputs_udp(&socket).await?;
            self.drain_notices().await?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_kiss_tcp(
        &mut self,
        server_addr: SocketAddr,
        bind_addr: Option<SocketAddr>,
        port: u8,
        reconnect_initial_delay: Duration,
        reconnect_max_delay: Duration,
        pacing: KissPacingConfig,
        addr_tx: oneshot::Sender<std::io::Result<SocketAddr>>,
    ) -> Result<()> {
        // Signal server address immediately so on_register does not block
        let _ = addr_tx.send(Ok(server_addr));

        let mut current_delay = reconnect_initial_delay;
        let connect_timeout = Duration::from_secs(5);
        const MAX_KISS_ACCUMULATOR: usize = 2 * 1024 * 1024; // 2 MiB
        let cancel = self.tasks.cancel_token().clone();

        'reconnect_loop: loop {
            info!("connecting LTP CLA to KISS TCP server at {server_addr}");
            let connect_fut = async {
                if let Some(baddr) = bind_addr {
                    let socket = if baddr.is_ipv4() {
                        TcpSocket::new_v4()?
                    } else {
                        TcpSocket::new_v6()?
                    };
                    socket.bind(baddr)?;
                    socket.connect(server_addr).await
                } else {
                    TcpStream::connect(server_addr).await
                }
            };

            let connect_res = tokio::time::timeout(connect_timeout, connect_fut).await;

            match connect_res {
                Ok(Ok(stream)) => {
                    info!("connected to KISS TCP server at {server_addr}");
                    // Reset backoff delay on successful connection
                    current_delay = reconnect_initial_delay;

                    // Disable Nagle's algorithm for timely transmission of small frames (ACKs/checkpoints)
                    let _ = stream.set_nodelay(true);

                    let (mut reader, mut writer) = stream.into_split();
                    let mut raw_rx_buf = vec![0u8; 4096];
                    let mut kiss_accumulator = Vec::with_capacity(8192);

                    let mut leaky_bucket = pacing
                        .rate_limit_bytes_per_sec
                        .map(|rate| LeakyBucket::new(rate, pacing.burst_capacity_bytes));
                    let mut pacer = HalfDuplexPacer::new(&pacing);

                    // Drain any output segments queued while disconnected obeying pacing and shaper
                    let mut write_err = false;
                    if !pacer.is_paused(tokio::time::Instant::now()) {
                        while let Some((_peer, bytes)) = self.engine.poll_output() {
                            let is_cp = is_checkpoint_segment(&bytes);
                            let payload = pad_segment_for_tnc(&bytes, pacing.min_frame_len);
                            let mut frame = Vec::new();
                            encode_frame(port, Command::Data, &payload, &mut frame);

                            if let Some(ref mut shaper) = leaky_bucket {
                                shaper.acquire(frame.len()).await;
                            }

                            if let Err(e) = writer.write_all(&frame).await {
                                warn!("KISS TCP initial write error to {server_addr}: {e}");
                                write_err = true;
                                break;
                            }

                            if is_cp {
                                pacer.on_checkpoint_sent();
                                break;
                            }
                        }
                    }

                    if !write_err {
                        let disconnect_reason = 'connected_loop: loop {
                            let now = self.now_micros();
                            let next_timer = self.engine.next_timer_deadline();

                            let timer_future = async {
                                match next_timer {
                                    Some(deadline) if deadline > now => {
                                        tokio::time::sleep(Duration::from_micros(deadline - now))
                                            .await;
                                    }
                                    Some(_) => {}
                                    None => {
                                        std::future::pending::<()>().await;
                                    }
                                }
                            };

                            let pacer_deadline = pacer.next_deadline();
                            let pacer_future = async {
                                match pacer_deadline {
                                    Some(dl) => tokio::time::sleep_until(dl).await,
                                    None => std::future::pending::<()>().await,
                                }
                            };

                            tokio::select! {
                                _ = cancel.cancelled() => {
                                    debug!("LTP KISS TCP connected loop cancelled");
                                    break "driver cancelled";
                                }
                                _ = pacer_future => {
                                    debug!("Half-duplex: pacer turnaround deadline elapsed, unpausing");
                                    pacer.pause_deadline = None;
                                }
                                read_res = reader.read(&mut raw_rx_buf) => {
                                    match read_res {
                                        Ok(0) => {
                                            warn!("KISS TCP server at {server_addr} closed connection (EOF)");
                                            break "server closed connection (EOF)";
                                        }
                                        Ok(n) => {
                                            if kiss_accumulator.len() + n > MAX_KISS_ACCUMULATOR {
                                                warn!(
                                                    "KISS accumulator exceeded {MAX_KISS_ACCUMULATOR} bytes, dropping corrupt buffer"
                                                );
                                                kiss_accumulator.clear();
                                            }
                                            kiss_accumulator.extend_from_slice(&raw_rx_buf[..n]);

                                            // Extract all complete KISS frames with linear O(N) cursor traversal
                                            let mut offset = 0;
                                            while let Ok(Some((frame, consumed))) = decode_frame(&kiss_accumulator[offset..]) {
                                                offset += consumed;
                                                if frame.command == Command::Data {
                                                    if is_report_segment(&frame.payload) {
                                                        pacer.on_report_received();
                                                    }
                                                    let now = self.now_micros();
                                                    // Dynamic peer identification on KISS frames (supports shared TNC / multi-station)
                                                    let peer_engine = if let Ok(session) = Segment::peek_session(&frame.payload) {
                                                        if session.originator != self.config.engine_id {
                                                            session.originator
                                                        } else if let Some(peer) = self.engine.peer_for_session(&session) {
                                                            peer
                                                        } else {
                                                            self.engine_to_addr.keys().copied().next().unwrap_or(EngineId(2))
                                                        }
                                                    } else {
                                                        self.engine_to_addr.keys().copied().next().unwrap_or(EngineId(2))
                                                    };
                                                    let _ = self.engine.handle_incoming_segment(peer_engine, &frame.payload, now);
                                                }
                                            }
                                            if offset > 0 {
                                                kiss_accumulator.drain(..offset);
                                            }
                                        }
                                        Err(e) => {
                                            warn!("KISS TCP read error from {server_addr}: {e}");
                                            break "read error";
                                        }
                                    }
                                }

                                _ = timer_future => {
                                    let now = self.now_micros();
                                    self.engine.handle_timer_tick(now);
                                }

                                cmd = self.cmd_rx.recv() => {
                                    let Some(cmd) = cmd else {
                                        break 'reconnect_loop;
                                    };
                                    if !self.handle_command(cmd).await? {
                                        break 'reconnect_loop;
                                    }
                                }
                            }

                            // Drain outputs over KISS obeying half-duplex pacing and leaky-bucket rate limiting
                            if !pacer.is_paused(tokio::time::Instant::now()) {
                                while let Some((_peer, bytes)) = self.engine.poll_output() {
                                    let is_cp = is_checkpoint_segment(&bytes);
                                    let payload = pad_segment_for_tnc(&bytes, pacing.min_frame_len);
                                    let mut frame = Vec::new();
                                    encode_frame(port, Command::Data, &payload, &mut frame);

                                    if let Some(ref mut shaper) = leaky_bucket {
                                        shaper.acquire(frame.len()).await;
                                    }

                                    if let Err(e) = writer.write_all(&frame).await {
                                        warn!("KISS TCP write error to {server_addr}: {e}");
                                        break 'connected_loop "write error";
                                    }

                                    if is_cp {
                                        pacer.on_checkpoint_sent();
                                        break;
                                    }
                                }
                            }

                            self.drain_notices().await?;
                        };
                        debug!(
                            "KISS TCP session ended ({disconnect_reason}); will reconnect with backoff"
                        );
                    }
                }
                Ok(Err(e)) => {
                    warn!(
                        "Failed to connect to KISS TCP server at {server_addr}: {e}; retrying in {current_delay:?}"
                    );
                }
                Err(_) => {
                    warn!(
                        "Connection to KISS TCP server at {server_addr} timed out; retrying in {current_delay:?}"
                    );
                }
            }

            // Disconnected state: backoff sleep while still servicing commands and timers!
            let backoff_deadline = tokio::time::sleep(current_delay);
            tokio::pin!(backoff_deadline);

            // Compute next delay for exponential backoff (multiplier 2x, saturating, capped at reconnect_max_delay)
            current_delay = current_delay.saturating_mul(2).min(reconnect_max_delay);

            loop {
                let now = self.now_micros();
                let next_timer = self.engine.next_timer_deadline();

                let timer_future = async {
                    match next_timer {
                        Some(deadline) if deadline > now => {
                            tokio::time::sleep(Duration::from_micros(deadline - now)).await;
                        }
                        Some(_) => {}
                        None => {
                            std::future::pending::<()>().await;
                        }
                    }
                };

                tokio::select! {
                    _ = cancel.cancelled() => {
                        debug!("LTP KISS TCP backoff loop cancelled");
                        break 'reconnect_loop;
                    }

                    () = &mut backoff_deadline => {
                        // Backoff period elapsed: try reconnecting
                        break;
                    }

                    _ = timer_future => {
                        let now = self.now_micros();
                        self.engine.handle_timer_tick(now);
                        self.drain_notices().await?;
                    }

                    cmd = self.cmd_rx.recv() => {
                        let Some(cmd) = cmd else {
                            break 'reconnect_loop;
                        };
                        if !self.handle_command(cmd).await? {
                            break 'reconnect_loop;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_command(&mut self, cmd: DriverCommand) -> Result<bool> {
        match cmd {
            DriverCommand::Forward {
                bundle_id,
                destination_engine,
                data,
                red_len,
                reply_tx,
            } => {
                if !self.engine_to_addr.contains_key(&destination_engine) {
                    let _ = reply_tx.send(Ok(ForwardBundleResult::NoNeighbour));
                    return Ok(true);
                }
                let now = self.now_micros();
                match self
                    .engine
                    .request_transmission(destination_engine, 1, data, red_len, now)
                {
                    Ok(session_id) => {
                        self.pending_transfers.insert(session_id, bundle_id);
                        let _ = reply_tx.send(Ok(ForwardBundleResult::Accepted));
                    }
                    Err(e) => {
                        let _ = reply_tx.send(Err(Error::Ltp(e)));
                    }
                }
                Ok(true)
            }
            DriverCommand::RegisterPeer {
                engine_id,
                addr,
                node_id,
            } => {
                self.engine_to_addr.insert(engine_id, addr);
                self.addr_to_engine.insert(addr, engine_id);
                if let Some(node) = node_id {
                    self.engine_to_node.insert(engine_id, node);
                }
                Ok(true)
            }
            DriverCommand::UnregisterPeer { engine_id } => {
                if let Some(addr) = self.engine_to_addr.remove(&engine_id) {
                    self.addr_to_engine.remove(&addr);
                }
                self.engine_to_node.remove(&engine_id);
                Ok(true)
            }
            DriverCommand::SetPeerTiming {
                engine_id,
                rtt,
                ack_margin,
            } => {
                let rtt_micros = u64::try_from(rtt.as_micros()).unwrap_or(u64::MAX);
                let ack_margin_micros = u64::try_from(ack_margin.as_micros()).unwrap_or(u64::MAX);
                self.engine
                    .set_peer_timing(engine_id, rtt_micros, ack_margin_micros);
                Ok(true)
            }
            DriverCommand::Shutdown => {
                for (_, bundle_id) in self.pending_transfers.drain() {
                    let _ = self
                        .sink
                        .transfer_outcome(&bundle_id, TransferOutcome::Failed)
                        .await;
                }
                Ok(false)
            }
        }
    }

    async fn drain_outputs_udp(&mut self, socket: &UdpSocket) -> Result<()> {
        while let Some((peer, bytes)) = self.engine.poll_output() {
            if let Some(&dest_addr) = self.engine_to_addr.get(&peer) {
                if let Err(err) = socket.send_to(&bytes, dest_addr).await {
                    warn!("failed to send LTP datagram to {dest_addr}: {err}");
                }
            } else {
                warn!("no destination socket address configured for Engine {peer:?}");
            }
        }
        Ok(())
    }

    async fn drain_notices(&mut self) -> Result<()> {
        while let Some(notice) = self.engine.poll_notice() {
            match notice {
                Notice::RedPartReceived {
                    session: _,
                    source,
                    client_service,
                    data,
                    eob: _,
                } => {
                    if client_service != 1 {
                        warn!(
                            "ignoring received red-part for unsupported client service {client_service}"
                        );
                        continue;
                    }
                    let peer_node = self.engine_to_node.get(&source).cloned();
                    let peer_addr = engine_id_to_cla_address(source);
                    let sink = self.sink.clone();

                    self.tasks.spawn(async move {
                        debug!(
                            "delivering received bundle ({} bytes) from Engine {:?}",
                            data.len(),
                            source
                        );
                        let mut bundle_stream = Bytes::from(data);
                        match sink
                            .dispatch(peer_node.as_ref(), Some(&peer_addr), &mut bundle_stream)
                            .await
                        {
                            Ok(Acceptance::Accepted) => {}
                            Ok(Acceptance::Refused) => {
                                warn!("bundle from Engine {source:?} refused by BPA");
                            }
                            Err(err) => {
                                error!("dispatch to BPA failed: {err}");
                            }
                        }
                    });
                }
                Notice::TransmissionSessionCompleted { session } => {
                    if let Some(bundle_id) = self.pending_transfers.remove(&session) {
                        debug!("transmission session {session:?} completed for bundle {bundle_id}");
                        let sink = self.sink.clone();
                        self.tasks.spawn(async move {
                            let _ = sink
                                .transfer_outcome(&bundle_id, TransferOutcome::Completed)
                                .await;
                        });
                    }
                }
                Notice::TransmissionSessionCancelled { session, reason } => {
                    if let Some(bundle_id) = self.pending_transfers.remove(&session) {
                        warn!(
                            "transmission session {session:?} cancelled ({reason:?}) for bundle {bundle_id}"
                        );
                        let sink = self.sink.clone();
                        self.tasks.spawn(async move {
                            let _ = sink
                                .transfer_outcome(&bundle_id, TransferOutcome::Failed)
                                .await;
                        });
                    }
                }
                Notice::GreenSegmentArrived {
                    session,
                    source,
                    offset,
                    data,
                    eob,
                } => {
                    let sink = self.sink.clone();
                    let peer_node = self.engine_to_node.get(&source).cloned();
                    let peer_addr = engine_id_to_cla_address(source);

                    if offset == 0 && eob {
                        // Single-segment green block (fast-path)
                        self.tasks.spawn(async move {
                            debug!(
                                "delivering single-segment green bundle ({} bytes) from Engine {:?}",
                                data.len(),
                                source
                            );
                            let mut bundle_stream = Bytes::from(data);
                            let _ = sink
                                .dispatch(peer_node.as_ref(), Some(&peer_addr), &mut bundle_stream)
                                .await;
                        });
                    } else {
                        // Multi-segment green block reassembly
                        const MAX_GREEN_REASSEMBLIES: usize = 64;
                        const MAX_GREEN_BUNDLE_SIZE: usize = 16 * 1024 * 1024; // 16 MiB max green bundle

                        let end_offset = match (offset as usize).checked_add(data.len()) {
                            Some(end) => end,
                            None => {
                                warn!(
                                    "integer overflow in Green segment offset {}; dropping",
                                    offset
                                );
                                continue;
                            }
                        };

                        if end_offset > MAX_GREEN_BUNDLE_SIZE {
                            warn!(
                                "Green segment session {:?} end offset {} exceeds max size {}; dropping",
                                session, end_offset, MAX_GREEN_BUNDLE_SIZE
                            );
                            self.green_reassemblies.remove(&session);
                            continue;
                        }

                        if !self.green_reassemblies.contains_key(&session)
                            && self.green_reassemblies.len() >= MAX_GREEN_REASSEMBLIES
                        {
                            // Evict the oldest or sweep expired reassemblies
                            let now = Instant::now();
                            self.green_reassemblies.retain(|_, r| {
                                now.duration_since(r.last_update) < Duration::from_secs(60)
                            });
                            if self.green_reassemblies.len() >= MAX_GREEN_REASSEMBLIES {
                                warn!(
                                    "too many concurrent Green reassemblies; dropping new session {:?}",
                                    session
                                );
                                continue;
                            }
                        }

                        let entry = self.green_reassemblies.entry(session).or_insert_with(|| {
                            GreenReassembly {
                                _source: source,
                                data: Vec::new(),
                                received_bytes: 0,
                                total_len: None,
                                last_update: Instant::now(),
                            }
                        });

                        entry.last_update = Instant::now();
                        if entry.data.len() < end_offset {
                            entry.data.resize(end_offset, 0);
                        }
                        entry.data[offset as usize..end_offset].copy_from_slice(&data);
                        entry.received_bytes += data.len();
                        if eob {
                            entry.total_len = Some(end_offset);
                        }

                        if let Some(total) = entry.total_len
                            && entry.received_bytes >= total
                            && entry.data.len() == total
                        {
                            let complete_data = std::mem::take(&mut entry.data);
                            self.green_reassemblies.remove(&session);

                            self.tasks.spawn(async move {
                                debug!(
                                    "delivering reassembled green bundle ({} bytes) from Engine {:?}",
                                    complete_data.len(),
                                    source
                                );
                                let mut bundle_stream = Bytes::from(complete_data);
                                let _ = sink
                                    .dispatch(
                                        peer_node.as_ref(),
                                        Some(&peer_addr),
                                        &mut bundle_stream,
                                    )
                                    .await;
                            });
                        }

                        // Periodic cleanup of stale green reassemblies (older than 30s)
                        if self.green_reassemblies.len() > 128 {
                            let now = Instant::now();
                            self.green_reassemblies.retain(|_, r| {
                                now.duration_since(r.last_update) < Duration::from_secs(30)
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnaroundMode;

    #[test]
    fn test_is_checkpoint_and_report_segment() {
        // Red Data non-CP (0)
        assert!(!is_checkpoint_segment(&[0x00, 0x11, 0x22]));
        // Red Data Checkpoints (1, 2, 3)
        assert!(is_checkpoint_segment(&[0x01, 0x11, 0x22]));
        assert!(is_checkpoint_segment(&[0x02, 0x11, 0x22]));
        assert!(is_checkpoint_segment(&[0x03, 0x11, 0x22]));
        // Red Data EORP non-CP (4)
        assert!(!is_checkpoint_segment(&[0x04, 0x11, 0x22]));
        // Report Segment (8)
        assert!(is_report_segment(&[0x08, 0x11, 0x22]));
        // Report Ack (9)
        assert!(!is_report_segment(&[0x09, 0x11, 0x22]));
        // Empty buffer
        assert!(!is_checkpoint_segment(&[]));
        assert!(!is_report_segment(&[]));
    }

    #[tokio::test]
    async fn test_leaky_bucket_pacing() {
        let mut bucket = LeakyBucket::new(10_000, 500); // 10,000 B/s, max burst 500 B
        let start = tokio::time::Instant::now();

        // Immediate consumption of initial burst
        bucket.acquire(500).await;
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(50),
            "Burst acquisition should be near instantaneous"
        );

        // Consuming 1000 more bytes at 10,000 B/s requires ~100ms
        bucket.acquire(1000).await;
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(80),
            "Paced acquisition should sleep for token replenishment"
        );
    }

    #[tokio::test]
    async fn test_half_duplex_pacer_wait_for_report() {
        let cfg = KissPacingConfig {
            min_frame_len: 0,
            rate_limit_bytes_per_sec: None,
            burst_capacity_bytes: 512,
            half_duplex: true,
            turnaround_mode: TurnaroundMode::WaitForReport,
            turnaround_timeout: Duration::from_millis(300),
            dwell_time: Duration::from_millis(100),
        };
        let mut pacer = HalfDuplexPacer::new(&cfg);

        let now = tokio::time::Instant::now();
        assert!(!pacer.is_paused(now));

        // Checkpoint emitted -> paused
        pacer.on_checkpoint_sent();
        assert!(pacer.is_paused(tokio::time::Instant::now()));

        // Report Segment received -> unpaused immediately
        pacer.on_report_received();
        assert!(!pacer.is_paused(tokio::time::Instant::now()));
    }

    #[tokio::test]
    async fn test_half_duplex_pacer_turnaround_timeout() {
        let cfg = KissPacingConfig {
            min_frame_len: 0,
            rate_limit_bytes_per_sec: None,
            burst_capacity_bytes: 512,
            half_duplex: true,
            turnaround_mode: TurnaroundMode::WaitForReport,
            turnaround_timeout: Duration::from_millis(50),
            dwell_time: Duration::from_millis(100),
        };
        let mut pacer = HalfDuplexPacer::new(&cfg);

        pacer.on_checkpoint_sent();
        assert!(pacer.is_paused(tokio::time::Instant::now()));

        // Wait past turnaround timeout
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert!(
            !pacer.is_paused(tokio::time::Instant::now()),
            "Pacer should unpause when turnaround timeout expires"
        );
    }

    #[tokio::test]
    async fn test_half_duplex_pacer_fixed_dwell() {
        let cfg = KissPacingConfig {
            min_frame_len: 0,
            rate_limit_bytes_per_sec: None,
            burst_capacity_bytes: 512,
            half_duplex: true,
            turnaround_mode: TurnaroundMode::FixedDwell,
            turnaround_timeout: Duration::from_millis(300),
            dwell_time: Duration::from_millis(60),
        };
        let mut pacer = HalfDuplexPacer::new(&cfg);

        pacer.on_checkpoint_sent();
        assert!(pacer.is_paused(tokio::time::Instant::now()));

        // In FixedDwell mode, receiving an RS does NOT cancel the dwell pause
        pacer.on_report_received();
        assert!(pacer.is_paused(tokio::time::Instant::now()));

        // Once dwell_time expires, pacer unpauses
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(
            !pacer.is_paused(tokio::time::Instant::now()),
            "Pacer should unpause after dwell time"
        );
    }
}
