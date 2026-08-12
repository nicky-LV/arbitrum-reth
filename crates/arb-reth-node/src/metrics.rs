//! Metrics for the live sequencer-feed path.

use arb_reth_engine::ArbAppliedMessageTiming;
use reth_metrics::{
    Metrics,
    metrics::{Counter, Histogram},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const MAX_TRACKED_MESSAGES: usize = 16_384;

/// Retention window for per-block ingress stamps served over RPC: ~27 minutes at ~10 blocks/s.
/// An off-process consumer joins its own per-block send stamps against these well after
/// canonicalization, so they must outlive metric emission — but not unboundedly.
const MAX_INGRESS_BLOCKS: usize = 16_384;

/// End-to-end latency from receiving a WebSocket data frame to the corresponding block becoming
/// the canonical in-memory head.
#[derive(Metrics)]
#[metrics(scope = "arb_reth.feed")]
struct FeedLatencyMetrics {
    /// sequenced2state: time from the sequencer's own message timestamp to canonical in-memory
    /// state on this node. The sequencer stamps whole seconds, so every sample carries the
    /// sub-second remainder of its stamp as a positive bias; the minimum over a window is the
    /// tightest estimate of the true latency, and the mean sits roughly half a block-second high.
    sequenced_to_state_seconds: Histogram,
    /// sequenced2received: the sequencer-to-node half of sequenced2state, carrying the same
    /// whole-second stamping bias. sequenced2state minus this is exactly `frame_to_canonical`.
    sequenced_to_received_seconds: Histogram,
    /// received2state: time from receiving a sequencer-feed WebSocket frame to canonical in-memory
    /// state. Both edges are local monotonic instants, so this half is exact.
    frame_to_canonical_seconds: Histogram,
    /// received2push: time from receiving a sequencer-feed WebSocket frame to the executed-push
    /// notification entering the broadcast channel (pre-canonical, before the state-root wait).
    /// `frame_to_canonical` minus this is the post-execution tail the push skips.
    frame_to_push_seconds: Histogram,
    /// WebSocket text/binary conversion and JSON decoding before a message is ready for the channel.
    frame_decode_seconds: Histogram,
    /// Channel send backpressure and time waiting in the driver input channel.
    channel_wait_seconds: Histogram,
    /// Time after driver dequeue before this sequence becomes eligible for in-order application.
    sequencing_wait_seconds: Histogram,
    /// Constructing native payload attributes from the ordered message and current parent.
    payload_attributes_seconds: Histogram,
    /// Full Reth payload-job lifecycle, including the builder execution nested within it.
    payload_job_seconds: Histogram,
    /// Payload-job launch through the attributes FCU response.
    payload_job_launch_seconds: Histogram,
    /// Waiting for the launched payload job after the attributes FCU response.
    payload_job_resolve_seconds: Histogram,
    /// Payload-job lifecycle time outside measured block production.
    payload_job_overhead_seconds: Histogram,
    /// ArbOS execution, state-root calculation, and block/header construction.
    block_production_seconds: Histogram,
    /// Block-production work outside the named production sub-phases.
    block_production_unattributed_seconds: Histogram,
    /// Parent-state provider setup before block production.
    block_parent_state_seconds: Histogram,
    /// Feed-message digesting and next-block environment construction.
    block_message_preparation_seconds: Histogram,
    /// Creation of revm's journaled state over the parent provider.
    block_state_setup_seconds: Histogram,
    /// ArbOS pre-execution and transaction execution.
    block_execution_seconds: Histogram,
    /// Block-builder creation, ArbOS pre-execution changes, and base-fee setup.
    block_execution_setup_seconds: Histogram,
    /// Construction of ArbOS's mandatory internal start-block transaction.
    block_start_block_transaction_construction_seconds: Histogram,
    /// Execution of ArbOS's mandatory internal start-block transaction.
    block_start_block_transaction_seconds: Histogram,
    /// Execution of derived user and retry transactions, including retry scheduling.
    block_derived_transactions_seconds: Histogram,
    /// Derived transaction execution and commit work, excluding retry scheduling.
    block_derived_transaction_execution_seconds: Histogram,
    /// Extraction and enqueueing of retries emitted by successful derived transactions.
    block_derived_retry_scheduling_seconds: Histogram,
    /// Remainder after named derived-transaction phases, retained for exact accounting.
    block_derived_transactions_unattributed_seconds: Histogram,
    /// Remainder after named block-execution phases, retained for exact accounting.
    block_execution_unattributed_seconds: Histogram,
    /// Total generic block finalization after ArbOS transactions complete.
    block_finish_seconds: Histogram,
    /// ArbOS executor finalization, principally reading post-execution header metadata.
    block_finish_executor_seconds: Histogram,
    /// Hashing the executed bundle into the post-state representation used by the trie.
    block_finish_hashed_state_seconds: Histogram,
    /// Computing the post-state root and trie updates.
    block_finish_state_root_seconds: Histogram,
    /// Waiting for the sparse state-root task after ArbOS execution.
    block_finish_state_root_task_wait_seconds: Histogram,
    /// Transaction/receipt roots, logs bloom, and Arbitrum header/block assembly.
    block_finish_assembly_seconds: Histogram,
    /// Generic finalization work not assigned to one of the named phases.
    block_finish_unattributed_seconds: Histogram,
    /// Full engine-tree handoff until canonical state is observable.
    engine_handoff_seconds: Histogram,
    /// Sending the executed block to reth's engine tree.
    engine_insert_seconds: Histogram,
    /// Forkchoice request and response through reth's engine tree.
    engine_forkchoice_seconds: Histogram,
    /// Waiting for the canonical in-memory provider state to observe the block.
    canonicalization_wait_seconds: Histogram,
    /// In-order apply-path work not covered by the named engine phases.
    engine_apply_overhead_seconds: Histogram,
    /// Total in-order apply path from payload attributes through canonical state.
    engine_apply_total_seconds: Histogram,
    /// Samples that could not be tracked without blocking the feed or execution task.
    tracking_dropped_total: Counter,
}

struct FeedLatencyInner {
    messages: Mutex<BTreeMap<u64, FeedMessageTiming>>,
    /// Ingress stamps retained past canonicalization, keyed by produced block number. `messages`
    /// keys by sequence number and forgets on completion; this map is what lets an RPC consumer
    /// resolve a block to the wall clock at which its frame hit the websocket.
    ingress_by_block: Mutex<BTreeMap<u64, IngressStamp>>,
    metrics: OnceLock<FeedLatencyMetrics>,
}

/// The ingress edge of one tracked frame, reduced to what a cross-process join needs: the
/// wall-clock stamp (the only cross-process-comparable edge) and the feed sequence number.
#[derive(Clone, Copy)]
struct IngressStamp {
    received_wall: SystemTime,
    sequence_number: u64,
}

#[derive(Clone, Copy)]
struct FeedMessageTiming {
    frame_received_at: Instant,
    /// Wall clock taken at the same edge as `frame_received_at`, the only point where this node's
    /// clock and the sequencer's stamp can be compared. Later phases are measured monotonically
    /// from `frame_received_at`, so a clock step cannot distort them.
    frame_received_wall: SystemTime,
    /// Seconds-granularity timestamp the sequencer stamped on this message.
    sequenced_at_secs: u64,
    ready_for_channel_at: Option<Instant>,
    driver_dequeued_at: Option<Instant>,
}

/// Correlates an inbound feed message with the point at which its block is canonical in reth's
/// shared in-memory state. Contention intentionally drops a sample instead of delaying either the
/// WebSocket reader or the engine driver.
#[derive(Clone)]
pub struct FeedLatencyTracker {
    inner: Arc<FeedLatencyInner>,
}

impl FeedLatencyTracker {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(FeedLatencyInner {
                messages: Mutex::new(BTreeMap::new()),
                ingress_by_block: Mutex::new(BTreeMap::new()),
                metrics: OnceLock::new(),
            }),
        }
    }

    /// Records the instant at which a WebSocket data frame was received, before parsing it, along
    /// with the wall clock at that same edge and the sequencer's stamp for this message.
    pub(crate) fn record_frame_arrival(
        &self,
        sequence_number: u64,
        received_at: Instant,
        received_wall: SystemTime,
        sequenced_at_secs: u64,
    ) {
        let mut messages = match self.inner.messages.try_lock() {
            Ok(messages) => messages,
            Err(_) => {
                self.metrics().tracking_dropped_total.increment(1);
                return;
            }
        };

        // Keep the first receipt for a sequence. A duplicated frame must not overwrite the
        // latency start of the message that was actually queued first.
        if messages.contains_key(&sequence_number) {
            return;
        }
        if messages.len() == MAX_TRACKED_MESSAGES {
            messages.pop_first();
            self.metrics().tracking_dropped_total.increment(1);
        }
        messages.insert(
            sequence_number,
            FeedMessageTiming {
                frame_received_at: received_at,
                frame_received_wall: received_wall,
                sequenced_at_secs,
                ready_for_channel_at: None,
                driver_dequeued_at: None,
            },
        );
    }

    /// Records the instant after a WebSocket frame has been decoded and the message is ready to
    /// send through the driver channel.
    pub(crate) fn record_ready_for_channel(&self, sequence_number: u64, ready_at: Instant) {
        let mut messages = match self.inner.messages.try_lock() {
            Ok(messages) => messages,
            Err(_) => {
                self.metrics().tracking_dropped_total.increment(1);
                return;
            }
        };
        if let Some(timing) = messages.get_mut(&sequence_number) {
            timing.ready_for_channel_at = Some(ready_at);
        }
    }

    /// Records the instant at which the engine driver dequeues a message.
    pub(crate) fn record_driver_dequeue(&self, sequence_number: u64, dequeued_at: Instant) {
        let mut messages = match self.inner.messages.try_lock() {
            Ok(messages) => messages,
            Err(_) => {
                self.metrics().tracking_dropped_total.increment(1);
                return;
            }
        };
        if let Some(timing) = messages.get_mut(&sequence_number) {
            timing.driver_dequeued_at = Some(dequeued_at);
        }
    }

    /// Records the arrival of an executed-push notification for `sequence_number`, measured
    /// against the frame's ws-ingress instant. Called from a metric-only push subscriber, so the
    /// stamp includes the (µs-scale) broadcast hop — close enough to the emission edge. The
    /// timing entry is still pending here (`record_canonical` runs later and removes it); a
    /// missing entry (L1-derived catch-up, replay, evicted) is a normal skip, not an error.
    pub(crate) fn record_push(&self, sequence_number: u64) {
        let now = Instant::now();
        let frame_received_at = match self.inner.messages.try_lock() {
            Ok(messages) => messages.get(&sequence_number).map(|t| t.frame_received_at),
            // Fail-soft like every recorder: never contend with the driver.
            Err(_) => None,
        };
        if let Some(frame_received_at) = frame_received_at {
            self.metrics()
                .frame_to_push_seconds
                .record(now.saturating_duration_since(frame_received_at).as_secs_f64());
        }
    }

    /// Records the end of the measurement and each engine phase after reth has canonicalized the
    /// corresponding block.
    pub(crate) fn record_canonical(&self, sequence_number: u64, applied: ArbAppliedMessageTiming) {
        let timing = match self.inner.messages.try_lock() {
            Ok(mut messages) => messages.remove(&sequence_number),
            Err(_) => {
                self.metrics().tracking_dropped_total.increment(1);
                return;
            }
        };

        if let Some(timing) = timing {
            // Metric emission aggregates and forgets; the ingress edge itself is retained under
            // the produced block number so an RPC consumer can join per-block against it later.
            // Same fail-soft rule as above: on contention drop this sample rather than ever
            // stalling the driver behind an RPC reader.
            match self.inner.ingress_by_block.try_lock() {
                Ok(mut ingress) => {
                    // Evicting the oldest block at capacity is the steady-state retention
                    // window, not a lost sample, so it is not counted as dropped.
                    if ingress.len() == MAX_INGRESS_BLOCKS {
                        ingress.pop_first();
                    }
                    ingress.insert(
                        applied.block_number,
                        IngressStamp {
                            received_wall: timing.frame_received_wall,
                            sequence_number,
                        },
                    );
                }
                Err(_) => self.metrics().tracking_dropped_total.increment(1),
            }

            let metrics = self.metrics();
            let received_to_state = applied
                .completed_at
                .saturating_duration_since(timing.frame_received_at)
                .as_secs_f64();
            metrics.frame_to_canonical_seconds.record(received_to_state);
            // The sequencer stamp is wall-clock, so this is the one span that has to cross clock
            // domains; every later phase stays on the monotonic clock. A zero stamp means the
            // message carried none (L1-derived or synthetic), which would otherwise read as
            // decades of latency.
            if timing.sequenced_at_secs != 0
                && let Some(sequenced_to_received) =
                    sequenced_to_received_secs(timing.frame_received_wall, timing.sequenced_at_secs)
            {
                metrics
                    .sequenced_to_received_seconds
                    .record(sequenced_to_received);
                metrics
                    .sequenced_to_state_seconds
                    .record(sequenced_to_received + received_to_state);
            }
            if let Some(ready_at) = timing.ready_for_channel_at {
                metrics.frame_decode_seconds.record(
                    ready_at
                        .saturating_duration_since(timing.frame_received_at)
                        .as_secs_f64(),
                );
                if let Some(dequeued_at) = timing.driver_dequeued_at {
                    metrics.channel_wait_seconds.record(
                        dequeued_at
                            .saturating_duration_since(ready_at)
                            .as_secs_f64(),
                    );
                    metrics.sequencing_wait_seconds.record(
                        applied
                            .started_at
                            .saturating_duration_since(dequeued_at)
                            .as_secs_f64(),
                    );
                }
            }
            metrics
                .payload_attributes_seconds
                .record(applied.payload_attributes.as_secs_f64());
            metrics
                .payload_job_seconds
                .record(applied.payload_job.as_secs_f64());
            metrics
                .payload_job_launch_seconds
                .record(applied.payload_job_launch.as_secs_f64());
            metrics
                .payload_job_resolve_seconds
                .record(applied.payload_job_resolve.as_secs_f64());
            metrics
                .payload_job_overhead_seconds
                .record(applied.payload_job_overhead.as_secs_f64());
            metrics
                .block_production_seconds
                .record(applied.block_production.as_secs_f64());
            metrics
                .block_production_unattributed_seconds
                .record(applied.block_production_unattributed.as_secs_f64());
            metrics
                .block_parent_state_seconds
                .record(applied.block_parent_state.as_secs_f64());
            metrics
                .block_message_preparation_seconds
                .record(applied.block_message_preparation.as_secs_f64());
            metrics
                .block_state_setup_seconds
                .record(applied.block_state_setup.as_secs_f64());
            metrics
                .block_execution_seconds
                .record(applied.block_execution.as_secs_f64());
            metrics
                .block_execution_setup_seconds
                .record(applied.block_execution_setup.as_secs_f64());
            metrics
                .block_start_block_transaction_construction_seconds
                .record(
                    applied
                        .block_start_block_transaction_construction
                        .as_secs_f64(),
                );
            metrics
                .block_start_block_transaction_seconds
                .record(applied.block_start_block_transaction.as_secs_f64());
            metrics
                .block_derived_transactions_seconds
                .record(applied.block_derived_transactions.as_secs_f64());
            metrics
                .block_derived_transaction_execution_seconds
                .record(applied.block_derived_transaction_execution.as_secs_f64());
            metrics
                .block_derived_retry_scheduling_seconds
                .record(applied.block_derived_retry_scheduling.as_secs_f64());
            metrics
                .block_derived_transactions_unattributed_seconds
                .record(
                    applied
                        .block_derived_transactions_unattributed
                        .as_secs_f64(),
                );
            metrics
                .block_execution_unattributed_seconds
                .record(applied.block_execution_unattributed.as_secs_f64());
            metrics
                .block_finish_seconds
                .record(applied.block_finish.as_secs_f64());
            metrics
                .block_finish_executor_seconds
                .record(applied.block_finish_executor.as_secs_f64());
            metrics
                .block_finish_hashed_state_seconds
                .record(applied.block_finish_hashed_state.as_secs_f64());
            metrics
                .block_finish_state_root_seconds
                .record(applied.block_finish_state_root.as_secs_f64());
            if let Some(wait) = applied.block_finish_state_root_task_wait {
                metrics
                    .block_finish_state_root_task_wait_seconds
                    .record(wait.as_secs_f64());
            }
            metrics
                .block_finish_assembly_seconds
                .record(applied.block_finish_assembly.as_secs_f64());
            metrics
                .block_finish_unattributed_seconds
                .record(applied.block_finish_unattributed.as_secs_f64());
            metrics
                .engine_handoff_seconds
                .record(applied.engine_handoff.as_secs_f64());
            metrics
                .engine_insert_seconds
                .record(applied.engine_insert.as_secs_f64());
            metrics
                .engine_forkchoice_seconds
                .record(applied.engine_forkchoice.as_secs_f64());
            metrics
                .canonicalization_wait_seconds
                .record(applied.canonicalization_wait.as_secs_f64());
            // The builder execution is nested inside `payload_job`; forkchoice and canonical
            // observation are nested inside `engine_handoff`. Only subtract the exclusive outer
            // phases so this remainder stays additive with the end-to-end apply duration.
            let named = applied.payload_attributes + applied.payload_job + applied.engine_handoff;
            metrics
                .engine_apply_overhead_seconds
                .record(applied.total.saturating_sub(named).as_secs_f64());
            metrics
                .engine_apply_total_seconds
                .record(applied.total.as_secs_f64());
        }
    }

    /// The retained ingress edge for `block_number`: the wall clock at which the frame that
    /// produced the block hit the websocket, and its feed sequence number. `None` once the block
    /// has aged out of the retention window, or if its frame was never tracked (contention drop,
    /// L1-derived, pre-restart).
    pub fn feed_ingress(&self, block_number: u64) -> Option<(SystemTime, u64)> {
        // A plain lock is fine here: the reader is the RPC task, and the writer side try_locks
        // and drops a sample rather than ever blocking the driver. `ok()?` treats a poisoned map
        // as absent — fail-soft, matching the recording side.
        let ingress = self.inner.ingress_by_block.lock().ok()?;
        ingress
            .get(&block_number)
            .map(|stamp| (stamp.received_wall, stamp.sequence_number))
    }

    fn metrics(&self) -> &FeedLatencyMetrics {
        // The live-feed task starts only after `with_prometheus_server` has installed reth's
        // recorder, so metric handles are never initialized against the no-op recorder.
        self.inner.metrics.get_or_init(FeedLatencyMetrics::default)
    }
}

/// Seconds from the sequencer's stamp to local ingress. Signed: a negative result means this
/// node's clock trails the sequencer's, which is a real condition worth surfacing rather than
/// clamping to zero. The stamp itself is truncated to whole seconds by the sequencer, so the
/// result also carries that message's sub-second remainder as a positive bias.
/// `None` only if the system clock is set before 1970, where no sample is better than a bogus one.
fn sequenced_to_received_secs(received_wall: SystemTime, sequenced_at_secs: u64) -> Option<f64> {
    let received_epoch = received_wall.duration_since(UNIX_EPOCH).ok()?.as_secs_f64();
    Some(received_epoch - sequenced_at_secs as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn wall(secs_since_epoch: f64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs_f64(secs_since_epoch)
    }

    #[track_caller]
    fn offset(received: f64, stamp: u64) -> f64 {
        sequenced_to_received_secs(wall(received), stamp).expect("post-1970 clock")
    }

    #[test]
    fn sequenced_to_received_is_the_sub_second_offset_from_the_stamp() {
        // The sequencer stamped 1_786_294_297; the frame landed 250ms into that second.
        let offset = offset(1_786_294_297.25, 1_786_294_297);
        assert!((offset - 0.25).abs() < 1e-9, "{offset}");
    }

    #[test]
    fn sequenced_to_received_spans_whole_seconds() {
        let offset = offset(1_786_294_299.5, 1_786_294_297);
        assert!((offset - 2.5).abs() < 1e-9, "{offset}");
    }

    #[test]
    fn sequenced_to_received_is_negative_when_the_local_clock_trails() {
        let offset = offset(1_786_294_296.75, 1_786_294_297);
        assert!((offset + 0.25).abs() < 1e-9, "{offset}");
    }

    /// An applied timing with only the join key populated. The duration fields feed histograms
    /// (no-op under test) and are irrelevant to ingress retention.
    fn applied_timing(block_number: u64) -> ArbAppliedMessageTiming {
        let now = Instant::now();
        ArbAppliedMessageTiming {
            block_number,
            started_at: now,
            completed_at: now,
            payload_attributes: Duration::ZERO,
            payload_job: Duration::ZERO,
            payload_job_launch: Duration::ZERO,
            payload_job_resolve: Duration::ZERO,
            payload_job_overhead: Duration::ZERO,
            block_production: Duration::ZERO,
            block_production_unattributed: Duration::ZERO,
            block_parent_state: Duration::ZERO,
            block_message_preparation: Duration::ZERO,
            block_state_setup: Duration::ZERO,
            block_execution: Duration::ZERO,
            block_execution_setup: Duration::ZERO,
            block_start_block_transaction_construction: Duration::ZERO,
            block_start_block_transaction: Duration::ZERO,
            block_derived_transactions: Duration::ZERO,
            block_derived_transaction_execution: Duration::ZERO,
            block_derived_retry_scheduling: Duration::ZERO,
            block_derived_transactions_unattributed: Duration::ZERO,
            block_execution_unattributed: Duration::ZERO,
            block_finish: Duration::ZERO,
            block_finish_executor: Duration::ZERO,
            block_finish_hashed_state: Duration::ZERO,
            block_finish_state_root: Duration::ZERO,
            block_finish_state_root_task_wait: None,
            block_finish_state_root_task_succeeded: None,
            block_finish_assembly: Duration::ZERO,
            block_finish_unattributed: Duration::ZERO,
            engine_handoff: Duration::ZERO,
            engine_insert: Duration::ZERO,
            engine_forkchoice: Duration::ZERO,
            canonicalization_wait: Duration::ZERO,
            total: Duration::ZERO,
        }
    }

    #[test]
    fn feed_ingress_serves_the_exact_wall_stamp_recorded_at_frame_arrival() {
        let tracker = FeedLatencyTracker::new();
        let received_wall = wall(1_786_294_297.25);
        tracker.record_frame_arrival(7, Instant::now(), received_wall, 1_786_294_297);
        tracker.record_canonical(7, applied_timing(370_000_007));

        assert_eq!(tracker.feed_ingress(370_000_007), Some((received_wall, 7)));
        // A block whose frame was never tracked has no entry to serve.
        assert_eq!(tracker.feed_ingress(370_000_008), None);
    }

    #[test]
    fn feed_ingress_evicts_the_oldest_block_past_capacity() {
        let tracker = FeedLatencyTracker::new();
        let received_at = Instant::now();
        let received_wall = wall(1_786_294_297.0);
        let first_block = 100u64;
        // One entry past capacity through the public recording path. The frame-arrival map is
        // drained by each record_canonical, so only ingress_by_block accumulates.
        for seq in 0..=MAX_INGRESS_BLOCKS as u64 {
            tracker.record_frame_arrival(seq, received_at, received_wall, 1_786_294_297);
            tracker.record_canonical(seq, applied_timing(first_block + seq));
        }

        assert_eq!(
            tracker.feed_ingress(first_block),
            None,
            "oldest block must be evicted by the insert that crossed capacity"
        );
        assert!(tracker.feed_ingress(first_block + 1).is_some());
        assert!(
            tracker
                .feed_ingress(first_block + MAX_INGRESS_BLOCKS as u64)
                .is_some()
        );
    }
}
