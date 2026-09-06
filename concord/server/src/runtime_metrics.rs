use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const OPERATION_COUNT: usize = 13;
const BUCKETS_SECONDS: [f64; 6] = [0.001, 0.01, 0.1, 1.0, 5.0, f64::INFINITY];

#[derive(Clone, Copy, Debug)]
pub(crate) enum Operation {
    CommandAdmission = 0,
    MessageCommit = 1,
    CommandAck = 2,
    OutboundQueue = 3,
    QueueOverflow = 4,
    Resync = 5,
    Replay = 6,
    DatabaseWrite = 7,
    Upload = 8,
    JobDispatch = 9,
    ReadinessProbe = 10,
    MetricsCollection = 11,
    Migration = 12,
}

impl Operation {
    pub(crate) const ALL: [Self; OPERATION_COUNT] = [
        Self::CommandAdmission,
        Self::MessageCommit,
        Self::CommandAck,
        Self::OutboundQueue,
        Self::QueueOverflow,
        Self::Resync,
        Self::Replay,
        Self::DatabaseWrite,
        Self::Upload,
        Self::JobDispatch,
        Self::ReadinessProbe,
        Self::MetricsCollection,
        Self::Migration,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::CommandAdmission => "command_admission",
            Self::MessageCommit => "message_commit",
            Self::CommandAck => "command_ack",
            Self::OutboundQueue => "outbound_queue",
            Self::QueueOverflow => "queue_overflow",
            Self::Resync => "resync",
            Self::Replay => "replay",
            Self::DatabaseWrite => "database_write",
            Self::Upload => "upload",
            Self::JobDispatch => "job_dispatch",
            Self::ReadinessProbe => "readiness_probe",
            Self::MetricsCollection => "metrics_collection",
            Self::Migration => "migration",
        }
    }
}

struct RuntimeMetrics {
    succeeded: [AtomicU64; OPERATION_COUNT],
    failed: [AtomicU64; OPERATION_COUNT],
    duration_buckets: [[AtomicU64; BUCKETS_SECONDS.len()]; OPERATION_COUNT],
    duration_count: [AtomicU64; OPERATION_COUNT],
    duration_nanoseconds: [AtomicU64; OPERATION_COUNT],
}

impl RuntimeMetrics {
    const fn new() -> Self {
        Self {
            succeeded: [const { AtomicU64::new(0) }; OPERATION_COUNT],
            failed: [const { AtomicU64::new(0) }; OPERATION_COUNT],
            duration_buckets: [const { [const { AtomicU64::new(0) }; BUCKETS_SECONDS.len()] };
                OPERATION_COUNT],
            duration_count: [const { AtomicU64::new(0) }; OPERATION_COUNT],
            duration_nanoseconds: [const { AtomicU64::new(0) }; OPERATION_COUNT],
        }
    }
}

static METRICS: RuntimeMetrics = RuntimeMetrics::new();

pub(crate) fn record(operation: Operation, succeeded: bool, duration: Duration) {
    let index = operation as usize;
    let counter = if succeeded {
        &METRICS.succeeded[index]
    } else {
        &METRICS.failed[index]
    };
    counter.fetch_add(1, Ordering::Relaxed);
    METRICS.duration_count[index].fetch_add(1, Ordering::Relaxed);
    let nanoseconds = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
    METRICS.duration_nanoseconds[index].fetch_add(nanoseconds, Ordering::Relaxed);
    let seconds = duration.as_secs_f64();
    for (bucket, boundary) in BUCKETS_SECONDS.iter().enumerate() {
        if seconds <= *boundary {
            METRICS.duration_buckets[index][bucket].fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[doc(hidden)]
pub struct Timer {
    operation: Operation,
    started: Instant,
    succeeded: bool,
}

impl Timer {
    pub(crate) fn start(operation: Operation) -> Self {
        Self {
            operation,
            started: Instant::now(),
            succeeded: false,
        }
    }

    pub fn succeed(&mut self) {
        self.succeeded = true;
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        record(self.operation, self.succeeded, self.started.elapsed());
    }
}

pub(crate) struct Snapshot {
    pub(crate) succeeded: [u64; OPERATION_COUNT],
    pub(crate) failed: [u64; OPERATION_COUNT],
    pub(crate) duration_buckets: [[u64; BUCKETS_SECONDS.len()]; OPERATION_COUNT],
    pub(crate) duration_count: [u64; OPERATION_COUNT],
    pub(crate) duration_seconds: [f64; OPERATION_COUNT],
}

pub(crate) fn snapshot() -> Snapshot {
    Snapshot {
        succeeded: std::array::from_fn(|index| METRICS.succeeded[index].load(Ordering::Relaxed)),
        failed: std::array::from_fn(|index| METRICS.failed[index].load(Ordering::Relaxed)),
        duration_buckets: std::array::from_fn(|operation| {
            std::array::from_fn(|bucket| {
                METRICS.duration_buckets[operation][bucket].load(Ordering::Relaxed)
            })
        }),
        duration_count: std::array::from_fn(|index| {
            METRICS.duration_count[index].load(Ordering::Relaxed)
        }),
        duration_seconds: std::array::from_fn(|index| {
            METRICS.duration_nanoseconds[index].load(Ordering::Relaxed) as f64 / 1_000_000_000.0
        }),
    }
}

pub(crate) fn bucket_name(index: usize) -> &'static str {
    match index {
        0 => "0.001",
        1 => "0.01",
        2 => "0.1",
        3 => "1",
        4 => "5",
        _ => "+Inf",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_and_cumulative_histograms_record_real_outcomes() {
        let index = Operation::MessageCommit as usize;
        let before = snapshot();
        record(Operation::MessageCommit, true, Duration::from_millis(5));
        record(Operation::MessageCommit, false, Duration::from_secs(2));
        let after = snapshot();
        assert!(after.succeeded[index] > before.succeeded[index]);
        assert!(after.failed[index] > before.failed[index]);
        assert!(after.duration_count[index] >= before.duration_count[index] + 2);
        assert!(after.duration_buckets[index][5] >= before.duration_buckets[index][5] + 2);
        assert!(after.duration_seconds[index] >= before.duration_seconds[index] + 2.005);
        assert_eq!(Operation::ALL.len(), OPERATION_COUNT);
        let names = Operation::ALL.map(Operation::name);
        assert_eq!(
            names
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            OPERATION_COUNT
        );
    }
}
