//! Mercury TNC ARQ driver managing TCP control and data sockets.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, sleep};
use tracing::{debug, error, info, trace, warn};

use hardy_async::TaskPool;
use hardy_bpa::cla::{Acceptance, ClaAddress, Sink};
use hardy_bpv7::eid::NodeId;

use crate::config::{Config, PeerEntry};
use crate::error::{Error, Result};

/// Converts a callsign into a canonical Mercury [`ClaAddress`].
#[must_use]
pub fn callsign_to_cla_address(callsign: &str) -> ClaAddress {
    ClaAddress::Private(Bytes::copy_from_slice(callsign.to_uppercase().as_bytes()))
}

/// Commands sent to the background driver task.
pub enum DriverCommand {
    /// Initiate an ARQ connection to the remote callsign.
    Connect {
        callsign: String,
        node_id: NodeId,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Disconnect any active ARQ session.
    Disconnect { reply: oneshot::Sender<Result<()>> },
    /// Forward a bundle across the active data socket.
    ForwardBundle {
        data: Bytes,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Register a known peer mapping.
    RegisterPeer { peer: PeerEntry },
    /// Query if the CLA currently has an active ARQ connection.
    IsConnected { reply: oneshot::Sender<bool> },
    /// Terminate the driver.
    Shutdown,
}

#[derive(Debug, PartialEq, Eq)]
enum SessionState {
    Disconnected,
    Connecting {
        peer_callsign: String,
        peer_node_id: NodeId,
    },
    Connected {
        peer_callsign: String,
        peer_node_id: NodeId,
        connected_at: Instant,
        last_activity: Instant,
    },
    Disconnecting,
}

pub struct Driver {
    config: Config,
    sink: Arc<dyn Sink>,
    tasks: Arc<TaskPool>,
    cmd_rx: mpsc::Receiver<DriverCommand>,
    peers: HashMap<String, NodeId>,
}

fn is_valid_callsign(call: &str) -> bool {
    let call = call.trim();
    !call.is_empty()
        && call.len() <= 15
        && call.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

impl Driver {
    pub fn spawn(
        config: Config,
        sink: Arc<dyn Sink>,
        tasks: Arc<TaskPool>,
        cmd_rx: mpsc::Receiver<DriverCommand>,
    ) -> tokio::task::JoinHandle<()> {
        let mut peers = HashMap::new();
        for p in &config.peers {
            peers.insert(p.callsign.to_uppercase(), p.effective_node_id());
        }
        let driver = Self {
            config,
            sink,
            tasks,
            cmd_rx,
            peers,
        };
        tokio::spawn(driver.run())
    }

    async fn run(mut self) {
        info!(
            "starting Mercury ARQ CLA driver (mycall={}, ctl={}, data={})",
            self.config.my_callsign, self.config.control_addr, self.config.data_addr
        );

        let mut connect_retry_delay = Duration::from_millis(500);

        'outer: loop {
            // Connect to Mercury control and data ports
            let (mut ctl_stream, data_stream) = match self.connect_mercury_ports().await {
                Ok(streams) => {
                    connect_retry_delay = Duration::from_millis(500);
                    streams
                }
                Err(err) => {
                    warn!(
                        "failed to connect to Mercury TNC: {err}. Retrying in {:?}",
                        connect_retry_delay
                    );
                    tokio::select! {
                        _ = sleep(connect_retry_delay) => {
                            connect_retry_delay = (connect_retry_delay * 2).min(Duration::from_secs(5));
                            continue 'outer;
                        }
                        cmd = self.cmd_rx.recv() => {
                            match cmd {
                                Some(DriverCommand::Shutdown) | None => break 'outer,
                                Some(DriverCommand::Connect { reply, .. }) => {
                                    let _ = reply.send(Err(Error::Control("Mercury TNC offline".into())));
                                }
                                Some(DriverCommand::Disconnect { reply }) => {
                                    let _ = reply.send(Ok(()));
                                }
                                Some(DriverCommand::ForwardBundle { reply, .. }) => {
                                    let _ = reply.send(Err(Error::Control("Mercury TNC offline".into())));
                                }
                                Some(DriverCommand::IsConnected { reply }) => {
                                    let _ = reply.send(false);
                                }
                                Some(DriverCommand::RegisterPeer { peer }) => {
                                    self.peers.insert(peer.callsign.to_uppercase(), peer.effective_node_id());
                                }
                            }
                            continue 'outer;
                        }
                    }
                }
            };

            // Initialize Mercury TNC mode
            if let Err(err) = self.init_mercury_tnc(&mut ctl_stream).await {
                warn!("failed to initialize Mercury TNC via control port: {err}");
                continue 'outer;
            }

            info!("connected and initialized Mercury TNC control and data interfaces");

            let (ctl_reader, mut ctl_writer) = ctl_stream.into_split();
            let (data_reader, mut data_writer) = data_stream.into_split();

            let mut session = SessionState::Disconnected;
            let mut pending_connect_reply: Option<oneshot::Sender<Result<()>>> = None;
            let mut pending_disconnect_reply: Option<oneshot::Sender<Result<()>>> = None;

            let (data_rx_tx, mut data_rx_rx) = mpsc::channel::<Bytes>(64);
            let (data_tx_tx, mut data_tx_rx) =
                mpsc::channel::<(Bytes, oneshot::Sender<Result<()>>)>(64);
            let (ctl_line_tx, mut ctl_line_rx) = mpsc::channel::<String>(64);

            // Spawn background task to read CRLF lines from control socket
            let ctl_reader_task = tokio::spawn(Self::control_read_loop(ctl_reader, ctl_line_tx));
            // Spawn background task to read length-prefixed bundles from data socket
            let data_reader_task = tokio::spawn(Self::data_read_loop(data_reader, data_rx_tx));
            // Spawn background task to write framed bundles to data socket
            let data_writer_task = tokio::spawn(async move {
                while let Some((bundle, reply)) = data_tx_rx.recv().await {
                    let len = bundle.len() as u32;
                    let mut frame = BytesMut::with_capacity(4 + bundle.len());
                    frame.extend_from_slice(&len.to_be_bytes());
                    frame.extend_from_slice(&bundle);
                    match data_writer.write_all(&frame).await {
                        Ok(()) => {
                            let _ = reply.send(Ok(()));
                        }
                        Err(e) => {
                            let _ = reply.send(Err(Error::Io(e)));
                            break;
                        }
                    }
                }
            });

            let mut idle_interval = tokio::time::interval(Duration::from_millis(500));

            'session_loop: loop {
                tokio::select! {
                    _ = idle_interval.tick() => {
                        if let SessionState::Connected { peer_callsign, last_activity, .. } = &session
                            && let Some(idle_to) = self.config.idle_timeout
                            && last_activity.elapsed() >= idle_to
                        {
                            info!("idle timeout reached for peer {peer_callsign}, issuing disconnect");
                            let cla_addr = callsign_to_cla_address(peer_callsign);
                            let _ = self.sink.remove_peer(&cla_addr).await;
                            let _ = ctl_writer.write_all(b"DISCONNECT\r").await;
                            session = SessionState::Disconnecting;
                        }
                    }

                    // Line received from Mercury control socket
                    line = ctl_line_rx.recv() => {
                        let Some(line) = line else {
                            warn!("Mercury control connection closed");
                            break 'session_loop;
                        };
                        trace!("Mercury CTL << {line}");
                        self.handle_ctl_line(
                            &line,
                            &mut session,
                            &mut pending_connect_reply,
                            &mut pending_disconnect_reply,
                        ).await;
                    }

                    // Inbound bundle read from data socket
                    bundle = data_rx_rx.recv() => {
                        let Some(mut bundle) = bundle else {
                            warn!("Mercury data connection closed");
                            break 'session_loop;
                        };

                        if let SessionState::Connected { ref peer_callsign, ref peer_node_id, ref mut last_activity, .. } = session {
                            *last_activity = Instant::now();
                            let peer_addr = callsign_to_cla_address(peer_callsign);
                            let node_id = peer_node_id.clone();
                            let sink = self.sink.clone();
                            self.tasks.spawn(async move {
                                debug!("delivering received bundle ({} bytes) from peer {node_id}", bundle.len());
                                match sink.dispatch(Some(&node_id), Some(&peer_addr), &mut bundle).await {
                                    Ok(Acceptance::Accepted) => {}
                                    Ok(Acceptance::Refused) => {
                                        warn!("bundle from {node_id} refused by BPA");
                                    }
                                    Err(err) => {
                                        error!("dispatch to BPA failed: {err}");
                                    }
                                }
                            });
                        } else {
                            warn!("received data packet while session not connected; dropping");
                        }
                    }

                    // Command from BPA / Coordinator
                    cmd = self.cmd_rx.recv() => {
                        let Some(cmd) = cmd else {
                            break 'outer;
                        };

                        match cmd {
                            DriverCommand::Connect { callsign, node_id, reply } => {
                                if !is_valid_callsign(&callsign) || !is_valid_callsign(&self.config.my_callsign) {
                                    let _ = reply.send(Err(Error::Control("Invalid callsign format".into())));
                                    continue;
                                }
                                match session {
                                    SessionState::Disconnected => {
                                        let upper_call = callsign.to_uppercase();
                                        info!("dialing Mercury ARQ: CONNECT {} {}", self.config.my_callsign, upper_call);
                                        let connect_cmd = format!("CONNECT {} {}\r", self.config.my_callsign, upper_call);
                                        if let Err(err) = ctl_writer.write_all(connect_cmd.as_bytes()).await {
                                            let _ = reply.send(Err(Error::Io(err)));
                                        } else {
                                            session = SessionState::Connecting {
                                                peer_callsign: upper_call,
                                                peer_node_id: node_id,
                                            };
                                            pending_connect_reply = Some(reply);
                                        }
                                    }
                                    SessionState::Connected { ref peer_callsign, .. } if peer_callsign == &callsign.to_uppercase() => {
                                        let _ = reply.send(Ok(()));
                                    }
                                    _ => {
                                        let _ = reply.send(Err(Error::Busy));
                                    }
                                }
                            }

                            DriverCommand::Disconnect { reply } => {
                                match session {
                                    SessionState::Connected { ref peer_callsign, .. } | SessionState::Connecting { ref peer_callsign, .. } => {
                                        let cla_addr = callsign_to_cla_address(peer_callsign);
                                        let _ = self.sink.remove_peer(&cla_addr).await;
                                        if let Err(err) = ctl_writer.write_all(b"DISCONNECT\r").await {
                                            let _ = reply.send(Err(Error::Io(err)));
                                        } else {
                                            session = SessionState::Disconnecting;
                                            pending_disconnect_reply = Some(reply);
                                        }
                                    }
                                    _ => {
                                        let _ = reply.send(Ok(()));
                                    }
                                }
                            }

                            DriverCommand::ForwardBundle { data, reply } => {
                                match session {
                                    SessionState::Connected { ref mut last_activity, .. } => {
                                        *last_activity = Instant::now();
                                        if data_tx_tx.send((data, reply)).await.is_err() {
                                            break 'session_loop;
                                        }
                                    }

                                    _ => {
                                        let _ = reply.send(Err(Error::ConnectionFailed));
                                    }
                                }
                            }

                            DriverCommand::IsConnected { reply } => {
                                let is_conn = matches!(session, SessionState::Connected { .. });
                                let _ = reply.send(is_conn);
                            }

                            DriverCommand::RegisterPeer { peer } => {
                                self.peers.insert(peer.callsign.to_uppercase(), peer.effective_node_id());
                            }

                            DriverCommand::Shutdown => {
                                if let SessionState::Connected { ref peer_callsign, .. } = session {
                                    let cla_addr = callsign_to_cla_address(peer_callsign);
                                    let _ = self.sink.remove_peer(&cla_addr).await;
                                    let _ = ctl_writer.write_all(b"DISCONNECT\r").await;
                                }
                                break 'outer;
                            }
                        }
                    }
                }
            }

            ctl_reader_task.abort();
            data_reader_task.abort();
            data_writer_task.abort();
        }

        info!("Mercury ARQ CLA driver shutdown");
    }

    async fn connect_mercury_ports(&self) -> Result<(TcpStream, TcpStream)> {
        let ctl = TcpStream::connect(self.config.control_addr).await?;
        let data = TcpStream::connect(self.config.data_addr).await?;
        Ok((ctl, data))
    }

    async fn init_mercury_tnc(&self, ctl: &mut TcpStream) -> Result<()> {
        let mycall = self.config.my_callsign.trim().to_uppercase();
        if !is_valid_callsign(&mycall) {
            return Err(Error::Control(format!(
                "Invalid configured my_callsign: {mycall}"
            )));
        }
        let setup = format!(
            "MYCALL {}\rBW{}\rLISTEN ON\r",
            mycall, self.config.bandwidth
        );
        ctl.write_all(setup.as_bytes()).await?;
        ctl.flush().await?;
        Ok(())
    }

    async fn control_read_loop(
        mut reader: tokio::net::tcp::OwnedReadHalf,
        tx: mpsc::Sender<String>,
    ) {
        let mut buf = Vec::with_capacity(1024);
        let mut temp = [0u8; 256];
        const MAX_CTL_BUF: usize = 4096;

        loop {
            match reader.read(&mut temp).await {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&temp[..n]);
                    while let Some(pos) = buf.iter().position(|&b| b == b'\r' || b == b'\n') {
                        let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                        let line = String::from_utf8_lossy(&line_bytes).trim().to_string();
                        if !line.is_empty() && tx.send(line).await.is_err() {
                            return;
                        }
                    }
                    if buf.len() > MAX_CTL_BUF {
                        warn!(
                            "Mercury TNC control buffer exceeded {} bytes without newline, dropping",
                            MAX_CTL_BUF
                        );
                        buf.clear();
                    }
                }
                Err(_) => break,
            }
        }
    }

    async fn data_read_loop(mut reader: tokio::net::tcp::OwnedReadHalf, tx: mpsc::Sender<Bytes>) {
        let mut len_buf = [0u8; 4];
        const MAX_MERCURY_FRAME: usize = 16 * 1024 * 1024; // 16 MiB max bundle over RF ARQ

        loop {
            if reader.read_exact(&mut len_buf).await.is_err() {
                break;
            }
            let length = u32::from_be_bytes(len_buf) as usize;
            if length == 0 {
                continue;
            }
            if length > MAX_MERCURY_FRAME {
                warn!(
                    "Mercury TNC frame size {} exceeds maximum {}; aborting connection",
                    length, MAX_MERCURY_FRAME
                );
                break;
            }

            let mut payload = vec![0u8; length];
            if reader.read_exact(&mut payload).await.is_err() {
                break;
            }

            if tx.send(Bytes::from(payload)).await.is_err() {
                break;
            }
        }
    }

    async fn handle_ctl_line(
        &mut self,
        line: &str,
        session: &mut SessionState,
        pending_connect_reply: &mut Option<oneshot::Sender<Result<()>>>,
        pending_disconnect_reply: &mut Option<oneshot::Sender<Result<()>>>,
    ) {
        if line.starts_with("CONNECTED") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let (src, dst) = if parts.len() >= 3 {
                (parts[1].to_uppercase(), parts[2].to_uppercase())
            } else {
                ("".to_string(), "".to_string())
            };

            // Determine peer callsign
            let peer = if src == self.config.my_callsign.to_uppercase() {
                dst
            } else {
                src
            };

            let peer_node = self
                .peers
                .entry(peer.clone())
                .or_insert_with(|| {
                    let uri = format!("dtn://{}/", peer.to_lowercase());
                    uri.parse().unwrap_or_else(|_| {
                        hardy_bpv7::eid::NodeId::Dtn(hardy_bpv7::eid::DtnNodeId {
                            node_name: peer.to_lowercase().into_boxed_str(),
                        })
                    })
                })
                .clone();

            info!("Mercury ARQ CONNECTED with peer {peer} ({peer_node})");

            let cla_addr = callsign_to_cla_address(&peer);
            let _ = self
                .sink
                .add_peer(cla_addr, std::slice::from_ref(&peer_node))
                .await;

            *session = SessionState::Connected {
                peer_callsign: peer,
                peer_node_id: peer_node,
                connected_at: Instant::now(),
                last_activity: Instant::now(),
            };

            if let Some(reply) = pending_connect_reply.take() {
                let _ = reply.send(Ok(()));
            }
        } else if line == "DISCONNECTED" {
            info!("Mercury ARQ DISCONNECTED");
            if let SessionState::Connected {
                ref peer_callsign, ..
            } = *session
            {
                let cla_addr = callsign_to_cla_address(peer_callsign);
                let _ = self.sink.remove_peer(&cla_addr).await;
            }

            *session = SessionState::Disconnected;

            if let Some(reply) = pending_connect_reply.take() {
                let _ = reply.send(Err(Error::ConnectionFailed));
            }
            if let Some(reply) = pending_disconnect_reply.take() {
                let _ = reply.send(Ok(()));
            }
        } else if line == "WRONG"
            && let Some(reply) = pending_connect_reply.take()
        {
            let _ = reply.send(Err(Error::Control("Mercury rejected command".into())));
        }
    }
}
