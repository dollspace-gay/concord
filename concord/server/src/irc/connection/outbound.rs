use super::{MAX_OUTBOUND_BYTES, Outbound, OutboundLine};

pub(super) fn send_line(out: &Outbound, line: &str) {
    send_line_guarded(out, line, None);
}

pub(super) fn sanitize_outbound_line(line: &str) -> String {
    line.replace(['\r', '\n', '\0'], " ")
}

pub(super) fn send_line_guarded(
    out: &Outbound,
    line: &str,
    guard: Option<crate::engine::user_session::DeliveryGuard>,
) {
    let line = line.to_string();
    let bytes = line.len();
    if out
        .queued_bytes
        .fetch_update(
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
            |current| {
                current
                    .checked_add(bytes)
                    .filter(|next| *next <= MAX_OUTBOUND_BYTES)
            },
        )
        .is_err()
    {
        out.failed.cancel();
        return;
    }
    if out.tx.try_send(OutboundLine { line, guard }).is_err() {
        out.queued_bytes
            .fetch_sub(bytes, std::sync::atomic::Ordering::AcqRel);
        out.failed.cancel();
    }
}
