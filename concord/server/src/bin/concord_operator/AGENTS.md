# Operator recovery implementation

The parent binary composes CLI commands. Administration, external jobs, backups, restores, filesystem activation, and key changes have separate modules.

Retain stopped-service exclusion, verified human/reason checks, audit records, durable staging markers, and resumable activation. Test-only barriers stay feature-gated. Never expose keys in command output.

Run operator_cli and backup_restore with default and all features; run packaging-restore when backup, restore, or installation behavior changes.
