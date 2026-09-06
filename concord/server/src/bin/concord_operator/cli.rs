use super::{Parser, PathBuf, Subcommand};

#[derive(Parser)]
#[command(name = "concord-operator")]
pub(super) struct Cli {
    #[arg(long, default_value = "concord.toml")]
    pub(super) config: PathBuf,
    #[command(subcommand)]
    pub(super) command: Command,
}

#[derive(Subcommand)]
pub(super) enum Command {
    KeyInit {
        #[arg(long)]
        key_file: PathBuf,
    },
    SecretsMigrate,
    SecretsRotate {
        #[arg(long)]
        new_key_file: PathBuf,
    },
    MediaInventory,
    MediaRetry {
        attachment_id: String,
    },
    MediaImport {
        #[arg(long, default_value_t = 300)]
        lease_seconds: i64,
    },
    /// List durable AT publication state without contacting a provider.
    AtprotoPublicationInventory,
    /// Requeue one failed/uncertain publication after current-policy checks.
    AtprotoPublicationReconcile {
        publication_id: String,
    },
    MigrationInventory,
    /// Report the recognized source and target schema without applying changes.
    MigrationStatus,
    /// Apply all recognized migrations after a successful repair preflight.
    MigrationApply,
    MigrationRepairUserOverride {
        #[arg(long)]
        override_id: String,
        #[arg(long)]
        target_user_id: String,
        #[arg(long)]
        evidence: String,
    },
    /// List current system administrators by stable user ID.
    AdminInventory,
    /// Atomically transfer system administration between verified human users.
    AdminTransfer {
        #[arg(long)]
        from_user_id: String,
        #[arg(long)]
        to_user_id: String,
        #[arg(long)]
        reason: String,
    },
    /// Add a verified human user as an administrator for local recovery.
    AdminRecover {
        #[arg(long)]
        user_id: String,
        #[arg(long)]
        reason: String,
    },
    /// Revoke every active local credential for a verified human user.
    CredentialRevokeAll {
        #[arg(long)]
        user_id: String,
        #[arg(long)]
        reason: String,
    },
    /// Inspect bounded external-job status without printing payloads or grants.
    JobsInspect {
        #[arg(long)]
        state: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: i64,
    },
    /// Requeue one failed external job; its dispatcher revalidates source policy.
    JobRetry {
        job_id: String,
        #[arg(long)]
        reason: String,
    },
    /// Create a stopped-service database, media, configuration, and key backup.
    BackupCreate {
        #[arg(long)]
        destination: PathBuf,
    },
    /// Verify checksums, database integrity/schema, media references, and keys.
    BackupVerify {
        #[arg(long)]
        backup: PathBuf,
    },
    /// Restore a verified backup into the empty paths named by --config.
    BackupRestore {
        #[arg(long)]
        backup: PathBuf,
    },
}
