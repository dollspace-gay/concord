# Executable entry points

Keep CLI parsing and composition here; substantial operator responsibilities live under concord_operator/. Fixture binaries are feature-gated test tools.

Preserve executable names, required features, output contracts, and private file permissions. Shared runtime behavior belongs in the library.

Run config_cli, operator_cli, and backup_restore integration tests. Feature builds must run sequentially because tests copy the compiled binaries.
