//! Contact coordinator managing scheduled contact windows for amateur radio CLAs.

pub mod cron;
pub mod endpoint;

use std::sync::Arc;
use std::time::Duration;

use time::OffsetDateTime;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use hardy_async::CancellationToken;
use hardy_bpv7::eid::NodeId;

use self::cron::CronExpr;
pub use self::endpoint::ContactExecutionEndpoint;

/// Contact schedule definition.
#[derive(Debug, Clone)]
pub enum ContactSchedule {
    /// Recurring contact triggered via cron expression with active duration.
    Recurring { cron: CronExpr, duration: Duration },
    /// One-shot contact defined by start and end timestamps.
    OneShot {
        start: Option<OffsetDateTime>,
        end: Option<OffsetDateTime>,
    },
}

/// A scheduled contact target.
#[derive(Debug, Clone)]
pub struct ContactTarget {
    pub callsign: String,
    pub node_id: Option<NodeId>,
    pub schedule: ContactSchedule,
    pub grace_period: Duration,
}

/// Coordinates contact schedules and drives the execution endpoint.
pub struct ContactCoordinator {
    endpoint: Arc<dyn ContactExecutionEndpoint>,
    targets: Vec<ContactTarget>,
}

impl ContactCoordinator {
    /// Creates a new contact coordinator for the given endpoint and targets.
    #[must_use]
    pub fn new(endpoint: Arc<dyn ContactExecutionEndpoint>, targets: Vec<ContactTarget>) -> Self {
        Self { endpoint, targets }
    }

    /// Spawns the coordinator loop in a background task using the given cancellation token.
    pub fn spawn(self, cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            info!(
                "starting ContactCoordinator with {} contact target(s)",
                self.targets.len()
            );

            if self.targets.is_empty() {
                debug!("no contact targets configured for coordinator");
                cancel.cancelled().await;
                return;
            }

            loop {
                let now = OffsetDateTime::now_utc();
                let mut next_target: Option<(&ContactTarget, OffsetDateTime, Duration)> = None;

                for target in &self.targets {
                    match &target.schedule {
                        ContactSchedule::Recurring { cron, duration } => {
                            if let Some(next_start) = cron.next_after(now)
                                && next_target.as_ref().is_none_or(|(_, t, _)| next_start < *t)
                            {
                                next_target = Some((target, next_start, *duration));
                            }
                        }
                        ContactSchedule::OneShot { start, end } => {
                            if let Some(s) = start
                                && *s > now
                            {
                                let dur = if let Some(e) = end {
                                    (*e - *s).try_into().unwrap_or(Duration::from_secs(60))
                                } else {
                                    Duration::from_secs(180)
                                };
                                if next_target.as_ref().is_none_or(|(_, t, _)| *s < *t) {
                                    next_target = Some((target, *s, dur));
                                }
                            }
                        }
                    }
                }

                let Some((target, next_start, duration)) = next_target else {
                    debug!("no upcoming contact windows found");
                    cancel.cancelled().await;
                    break;
                };

                let delay = (next_start - now).try_into().unwrap_or(Duration::ZERO);
                debug!(
                    "next contact with {} scheduled at {} (in {:?})",
                    target.callsign, next_start, delay
                );

                tokio::select! {
                    _ = sleep(delay) => {
                        info!(
                            "contact window opened for {}: initiating contact (duration={:?})",
                            target.callsign, duration
                        );
                        if let Err(err) = self.endpoint.start_contact(&target.callsign, target.node_id.as_ref()).await {
                            error!("failed to start contact with {}: {err}", target.callsign);
                        } else {
                            // Wait for duration of contact
                            tokio::select! {
                                _ = sleep(duration) => {
                                    info!("contact duration expired for {}: terminating contact", target.callsign);
                                    if let Err(err) = self.endpoint.end_contact(&target.callsign).await {
                                        warn!("error during contact teardown with {}: {err}", target.callsign);
                                    }
                                }
                                _ = cancel.cancelled() => {
                                    info!("cancellation requested during active contact with {}", target.callsign);
                                    let _ = self.endpoint.end_contact(&target.callsign).await;
                                    break;
                                }
                            }
                        }
                    }
                    _ = cancel.cancelled() => {
                        info!("cancellation requested, exiting ContactCoordinator");
                        break;
                    }
                }
            }
        })
    }
}
