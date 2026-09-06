#[cfg(feature = "storage-fault-injection")]
use super::PathBuf;

#[cfg(feature = "storage-fault-injection")]
pub(super) fn rotation_test_barrier(stage: &str) {
    let Ok(base) = std::env::var("CONCORD_ROTATION_TEST_BARRIER") else {
        return;
    };
    let marker = PathBuf::from(format!("{base}.{stage}"));
    std::fs::write(marker, b"ready\n").expect("rotation test marker must be writable");
    loop {
        std::thread::park();
    }
}

#[cfg(not(feature = "storage-fault-injection"))]
pub(super) fn rotation_test_barrier(_stage: &str) {}

#[cfg(feature = "storage-fault-injection")]
pub(super) fn restore_test_barrier(stage: &str) {
    let Ok(base) = std::env::var("CONCORD_RESTORE_TEST_BARRIER") else {
        return;
    };
    if std::env::var("CONCORD_RESTORE_TEST_STAGE").as_deref() != Ok(stage) {
        return;
    }
    let marker = PathBuf::from(format!("{base}.{stage}"));
    std::fs::write(marker, b"ready\n").expect("restore test marker must be writable");
    loop {
        std::thread::park();
    }
}

#[cfg(not(feature = "storage-fault-injection"))]
pub(super) fn restore_test_barrier(_stage: &str) {}
