#[derive(Clone, Copy)]
pub(super) struct Migration {
    pub(super) version: i64,
    pub(super) name: &'static str,
    pub(super) sql: &'static str,
}

pub(super) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "001_initial.sql",
        sql: include_str!("../../../migrations/001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "002_servers.sql",
        sql: include_str!("../../../migrations/002_servers.sql"),
    },
    Migration {
        version: 3,
        name: "003_messaging_enhancements.sql",
        sql: include_str!("../../../migrations/003_messaging_enhancements.sql"),
    },
    Migration {
        version: 4,
        name: "004_media_files.sql",
        sql: include_str!("../../../migrations/004_media_files.sql"),
    },
    Migration {
        version: 5,
        name: "005_atproto_blob_storage.sql",
        sql: include_str!("../../../migrations/005_atproto_blob_storage.sql"),
    },
    Migration {
        version: 6,
        name: "006_server_config.sql",
        sql: include_str!("../../../migrations/006_server_config.sql"),
    },
    Migration {
        version: 7,
        name: "007_organization_permissions.sql",
        sql: include_str!("../../../migrations/007_organization_permissions.sql"),
    },
    Migration {
        version: 8,
        name: "008_user_experience.sql",
        sql: include_str!("../../../migrations/008_user_experience.sql"),
    },
    Migration {
        version: 9,
        name: "009_threads_pinning.sql",
        sql: include_str!("../../../migrations/009_threads_pinning.sql"),
    },
    Migration {
        version: 10,
        name: "010_moderation.sql",
        sql: include_str!("../../../migrations/010_moderation.sql"),
    },
    Migration {
        version: 11,
        name: "011_community.sql",
        sql: include_str!("../../../migrations/011_community.sql"),
    },
    Migration {
        version: 12,
        name: "012_integrations.sql",
        sql: include_str!("../../../migrations/012_integrations.sql"),
    },
    Migration {
        version: 13,
        name: "013_atproto_integration.sql",
        sql: include_str!("../../../migrations/013_atproto_integration.sql"),
    },
    Migration {
        version: 14,
        name: "014_user_id_to_did.sql",
        sql: include_str!("../../../migrations/014_user_id_to_did.sql"),
    },
    Migration {
        version: 15,
        name: "015_premium_for_free.sql",
        sql: include_str!("../../../migrations/015_premium_for_free.sql"),
    },
    Migration {
        version: 16,
        name: "016_fts_delete_trigger.sql",
        sql: include_str!("../../../migrations/016_fts_delete_trigger.sql"),
    },
    Migration {
        version: 17,
        name: "017_migration_foundation.sql",
        sql: include_str!("../../../migrations/017_migration_foundation.sql"),
    },
    Migration {
        version: 18,
        name: "018_session_authority.sql",
        sql: include_str!("../../../migrations/018_session_authority.sql"),
    },
    Migration {
        version: 19,
        name: "019_authorization_threads.sql",
        sql: include_str!("../../../migrations/019_authorization_threads.sql"),
    },
    Migration {
        version: 20,
        name: "020_conversations_messages.sql",
        sql: include_str!("../../../migrations/020_conversations_messages.sql"),
    },
    Migration {
        version: 21,
        name: "021_receipts_events.sql",
        sql: include_str!("../../../migrations/021_receipts_events.sql"),
    },
    Migration {
        version: 22,
        name: "022_identity_direct_presence.sql",
        sql: include_str!("../../../migrations/022_identity_direct_presence.sql"),
    },
    Migration {
        version: 23,
        name: "023_private_media.sql",
        sql: include_str!("../../../migrations/023_private_media.sql"),
    },
    Migration {
        version: 24,
        name: "024_feature_integrity.sql",
        sql: include_str!("../../../migrations/024_feature_integrity.sql"),
    },
    Migration {
        version: 25,
        name: "025_operation_generations.sql",
        sql: include_str!("../../../migrations/025_operation_generations.sql"),
    },
    Migration {
        version: 26,
        name: "026_integration_contracts.sql",
        sql: include_str!("../../../migrations/026_integration_contracts.sql"),
    },
    Migration {
        version: 27,
        name: "027_moderation_notification_integrity.sql",
        sql: include_str!("../../../migrations/027_moderation_notification_integrity.sql"),
    },
    Migration {
        version: 28,
        name: "028_server_member_nicknames.sql",
        sql: include_str!("../../../migrations/028_server_member_nicknames.sql"),
    },
    Migration {
        version: 29,
        name: "029_oauth2_lifecycle.sql",
        sql: include_str!("../../../migrations/029_oauth2_lifecycle.sql"),
    },
    Migration {
        version: 30,
        name: "030_role_projection_versions.sql",
        sql: include_str!("../../../migrations/030_role_projection_versions.sql"),
    },
    Migration {
        version: 31,
        name: "031_message_chronology_index.sql",
        sql: include_str!("../../../migrations/031_message_chronology_index.sql"),
    },
    Migration {
        version: 32,
        name: "032_operator_audit.sql",
        sql: include_str!("../../../migrations/032_operator_audit.sql"),
    },
];
