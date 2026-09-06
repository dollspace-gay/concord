-- Migration 032: durable audit records for stopped-service operator actions.

CREATE TABLE operator_audit_log (
    id            TEXT PRIMARY KEY,
    action_type   TEXT NOT NULL CHECK(action_type IN (
        'admin_transfer',
        'admin_recovery',
        'credential_revoke_all',
        'external_job_retry'
    )),
    actor_kind    TEXT NOT NULL DEFAULT 'local_stopped_operator'
        CHECK(actor_kind = 'local_stopped_operator'),
    target_type   TEXT NOT NULL CHECK(target_type IN ('user', 'external_job')),
    target_id     TEXT NOT NULL,
    reason        TEXT NOT NULL CHECK(length(trim(reason)) BETWEEN 1 AND 1000),
    details_json  TEXT NOT NULL CHECK(json_valid(details_json)),
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_operator_audit_target
    ON operator_audit_log(target_type, target_id, created_at DESC);
CREATE INDEX idx_operator_audit_action
    ON operator_audit_log(action_type, created_at DESC);

INSERT INTO schema_version(version,applied_at) VALUES (32,datetime('now'));
