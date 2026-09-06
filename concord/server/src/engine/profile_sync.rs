use sqlx::{Row, SqlitePool};
use thiserror::Error;

use crate::auth::authority::{Actor, AuthError, AuthService};

use super::authorization::AuthorizationStamp;
use super::events::UserProfileInfo;
use super::write_admission::{WriteAdmission, WriteAdmissionError};

#[derive(Debug, Clone)]
pub struct BlueskyProfileSyncInput<'a> {
    pub did: &'a str,
    pub handle: &'a str,
    pub display_name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub avatar: Option<&'a str>,
    pub banner: Option<&'a str>,
    pub followers_count: i64,
    pub follows_count: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct AtprotoIdentity {
    pub did: String,
    pub bsky_handle: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub banner_url: Option<String>,
    pub followers_count: Option<i64>,
    pub follows_count: Option<i64>,
    pub last_sync: Option<String>,
}

#[derive(Debug, Error)]
pub enum ProfileSyncError {
    #[error("authentication required")]
    Authentication,
    #[error("Bluesky identity is unavailable")]
    IdentityUnavailable,
    #[error("Bluesky identity changed while the profile was fetched")]
    IdentityChanged,
    #[error("profile sync dependency unavailable")]
    DependencyUnavailable,
    #[error("profile sync database operation failed: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct ProfileSyncService {
    pool: SqlitePool,
    auth: AuthService,
    writes: WriteAdmission,
}

impl ProfileSyncService {
    pub fn new(pool: SqlitePool, auth: AuthService, writes: WriteAdmission) -> Self {
        Self { pool, auth, writes }
    }

    pub async fn identity_for_actor(
        &self,
        actor: &Actor,
        user_id: &str,
    ) -> Result<(Option<AtprotoIdentity>, Option<AuthorizationStamp>), ProfileSyncError> {
        let mut transaction = self.pool.begin().await?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(map_auth_error)?;
        let shared_server: Option<(String, i64)> = if actor.user_id().as_str() == user_id {
            None
        } else {
            sqlx::query_as(
                "SELECT s.id,s.authorization_version FROM servers s \
                 JOIN server_members requester ON requester.server_id=s.id AND requester.user_id=? \
                 JOIN server_members target ON target.server_id=s.id AND target.user_id=? \
                 WHERE NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=s.id AND b.user_id IN (?,?)) \
                 ORDER BY s.id LIMIT 1",
            )
            .bind(actor.user_id().as_str())
            .bind(user_id)
            .bind(actor.user_id().as_str())
            .bind(user_id)
            .fetch_optional(&mut *transaction)
            .await?
        };
        if actor.user_id().as_str() != user_id && shared_server.is_none() {
            return Ok((None, None));
        }
        let row = sqlx::query(
            "SELECT provider_id,bsky_handle,bsky_display_name,bsky_description,bsky_banner_url, \
             bsky_followers_count,bsky_follows_count,last_profile_sync FROM oauth_accounts \
             WHERE user_id=? AND provider='atproto'",
        )
        .bind(user_id)
        .fetch_optional(&mut *transaction)
        .await?;
        transaction.commit().await?;
        let identity = row.map(|row| AtprotoIdentity {
            did: row.get(0),
            bsky_handle: row.get(1),
            display_name: row.get(2),
            description: row.get(3),
            banner_url: row.get(4),
            followers_count: row.get(5),
            follows_count: row.get(6),
            last_sync: row.get(7),
        });
        let stamp = shared_server.map(|(server_id, server_version)| AuthorizationStamp {
            server_id,
            server_version,
            channel_versions: Vec::new(),
        });
        Ok((identity, stamp))
    }

    pub async fn sync_enabled(&self, actor: &Actor) -> Result<bool, ProfileSyncError> {
        let mut transaction = self.pool.begin().await?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(map_auth_error)?;
        let enabled: bool = sqlx::query_scalar(
            "SELECT atproto_sync_enabled=1 FROM users WHERE id=? AND disabled_at IS NULL",
        )
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(enabled)
    }

    pub async fn set_sync_enabled(
        &self,
        actor: &Actor,
        enabled: bool,
    ) -> Result<(), ProfileSyncError> {
        let (_permit, mut transaction) = self.writes.begin().await.map_err(map_write_error)?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(map_auth_error)?;
        let updated = sqlx::query(
            "UPDATE users SET atproto_sync_enabled=? WHERE id=? AND disabled_at IS NULL",
        )
        .bind(i64::from(enabled))
        .bind(actor.user_id().as_str())
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(ProfileSyncError::Authentication);
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Resolve the verified AT identity without assuming the local user ID is a DID.
    pub async fn verified_did(&self, actor: &Actor) -> Result<String, ProfileSyncError> {
        let mut transaction = self.pool.begin().await?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(map_auth_error)?;
        let identities: Vec<String> = sqlx::query_scalar(
            "SELECT provider_id FROM oauth_accounts \
             WHERE user_id=? AND provider='atproto' ORDER BY id LIMIT 2",
        )
        .bind(actor.user_id().as_str())
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        match identities.as_slice() {
            [did] if !did.is_empty() => Ok(did.clone()),
            _ => Err(ProfileSyncError::IdentityUnavailable),
        }
    }

    /// Commit a fetched profile only if the same credential and linked DID are current.
    pub async fn apply(
        &self,
        actor: &Actor,
        expected_did: &str,
        profile: &BlueskyProfileSyncInput<'_>,
    ) -> Result<UserProfileInfo, ProfileSyncError> {
        if profile.did != expected_did {
            return Err(ProfileSyncError::IdentityChanged);
        }
        let (_permit, mut transaction) = self.writes.begin().await.map_err(map_write_error)?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(map_auth_error)?;
        let identities: Vec<String> = sqlx::query_scalar(
            "SELECT provider_id FROM oauth_accounts \
             WHERE user_id=? AND provider='atproto' ORDER BY id LIMIT 2",
        )
        .bind(actor.user_id().as_str())
        .fetch_all(&mut *transaction)
        .await?;
        if identities.as_slice() != [expected_did] {
            return Err(ProfileSyncError::IdentityChanged);
        }

        let sync = sqlx::query(
            "UPDATE oauth_accounts SET bsky_handle=?,bsky_display_name=?,bsky_description=?, \
             bsky_banner_url=?,bsky_followers_count=?,bsky_follows_count=?,last_profile_sync=datetime('now') \
             WHERE user_id=? AND provider='atproto' AND provider_id=?",
        )
        .bind(profile.handle)
        .bind(profile.display_name)
        .bind(profile.description)
        .bind(profile.banner)
        .bind(profile.followers_count)
        .bind(profile.follows_count)
        .bind(actor.user_id().as_str())
        .bind(expected_did)
        .execute(&mut *transaction)
        .await?;
        if sync.rows_affected() != 1 {
            return Err(ProfileSyncError::IdentityChanged);
        }

        let current = sqlx::query(
            "SELECT u.avatar_url,p.banner_url,p.pronouns FROM users u \
             LEFT JOIN user_profiles p ON p.user_id=u.id WHERE u.id=? AND u.disabled_at IS NULL",
        )
        .bind(actor.user_id().as_str())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ProfileSyncError::Authentication)?;
        let current_avatar: Option<String> = current.get(0);
        let current_banner: Option<String> = current.get(1);
        let pronouns: Option<String> = current.get(2);
        let avatar = if current_avatar
            .as_deref()
            .and_then(crate::media::local_attachment_id)
            .is_some()
        {
            current_avatar
        } else {
            profile.avatar.map(str::to_owned)
        };
        let banner = if current_banner
            .as_deref()
            .and_then(crate::media::local_attachment_id)
            .is_some()
        {
            current_banner
        } else {
            profile.banner.map(str::to_owned)
        };
        sqlx::query("UPDATE users SET avatar_url=?,updated_at=datetime('now') WHERE id=?")
            .bind(&avatar)
            .bind(actor.user_id().as_str())
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO user_profiles(user_id,bio,pronouns,banner_url) VALUES(?,?,?,?) \
             ON CONFLICT(user_id) DO UPDATE SET bio=excluded.bio,pronouns=excluded.pronouns, \
             banner_url=excluded.banner_url,updated_at=datetime('now')",
        )
        .bind(actor.user_id().as_str())
        .bind(profile.description)
        .bind(&pronouns)
        .bind(&banner)
        .execute(&mut *transaction)
        .await?;

        let row = sqlx::query(
            "SELECT u.id,u.username,u.avatar_url,p.bio,p.pronouns,p.banner_url,u.created_at \
             FROM users u JOIN user_profiles p ON p.user_id=u.id WHERE u.id=?",
        )
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *transaction)
        .await?;
        let updated = UserProfileInfo {
            user_id: row.get(0),
            username: row.get(1),
            avatar_url: row.get(2),
            bio: row.get(3),
            pronouns: row.get(4),
            banner_url: row.get(5),
            created_at: row.get(6),
        };
        transaction.commit().await?;
        Ok(updated)
    }
}

fn map_auth_error(error: AuthError) -> ProfileSyncError {
    match error {
        AuthError::Database(_) | AuthError::VerificationBusy | AuthError::HashWorker(_) => {
            ProfileSyncError::DependencyUnavailable
        }
        AuthError::Invalid
        | AuthError::Expired
        | AuthError::Revoked
        | AuthError::Disabled
        | AuthError::Token(_) => ProfileSyncError::Authentication,
    }
}

fn map_write_error(error: WriteAdmissionError) -> ProfileSyncError {
    match error {
        WriteAdmissionError::Unavailable => ProfileSyncError::DependencyUnavailable,
        WriteAdmissionError::Database(error) => ProfileSyncError::Database(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};

    async fn fixture() -> (SqlitePool, AuthService, Actor, ProfileSyncService) {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username) VALUES('local-user','alice')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO oauth_accounts(id,user_id,provider,provider_id) \
             VALUES('at-account','local-user','atproto','did:plc:alice')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = AuthService::new(pool.clone(), "profile-sync-secret".into(), 1);
        let actor = auth.issue_web_session("local-user").await.unwrap().1;
        let service = ProfileSyncService::new(
            pool.clone(),
            auth.clone(),
            WriteAdmission::new(pool.clone()),
        );
        (pool, auth, actor, service)
    }

    fn remote<'a>() -> BlueskyProfileSyncInput<'a> {
        BlueskyProfileSyncInput {
            did: "did:plc:alice",
            handle: "alice.test",
            display_name: Some("Alice"),
            description: Some("Remote biography"),
            avatar: Some("https://cdn.test/avatar.jpg"),
            banner: Some("https://cdn.test/banner.jpg"),
            followers_count: 4,
            follows_count: 3,
        }
    }

    #[tokio::test]
    async fn stable_local_id_sync_preserves_managed_media_and_pronouns_atomically() {
        let (pool, _, actor, service) = fixture().await;
        let managed = "/api/uploads/10000000-0000-4000-8000-000000000001";
        sqlx::query("UPDATE users SET avatar_url=? WHERE id='local-user'")
            .bind(managed)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO user_profiles(user_id,bio,pronouns,banner_url) VALUES('local-user','Local bio','she/her',?)",
        )
        .bind(managed)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(service.verified_did(&actor).await.unwrap(), "did:plc:alice");
        let profile = service
            .apply(&actor, "did:plc:alice", &remote())
            .await
            .unwrap();
        assert_eq!(profile.user_id, "local-user");
        assert_eq!(profile.avatar_url.as_deref(), Some(managed));
        assert_eq!(profile.banner_url.as_deref(), Some(managed));
        assert_eq!(profile.bio.as_deref(), Some("Remote biography"));
        assert_eq!(profile.pronouns.as_deref(), Some("she/her"));
        let sync: (String, String, i64, i64) = sqlx::query_as(
            "SELECT bsky_handle,bsky_display_name,bsky_followers_count,bsky_follows_count \
             FROM oauth_accounts WHERE id='at-account'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(sync, ("alice.test".into(), "Alice".into(), 4, 3));
    }

    #[tokio::test]
    async fn revoked_credential_or_changed_identity_cannot_commit_a_held_fetch() {
        let (pool, auth, actor, service) = fixture().await;
        let did = service.verified_did(&actor).await.unwrap();
        auth.revoke_credential(actor.credential_id()).await.unwrap();
        assert!(matches!(
            service.apply(&actor, &did, &remote()).await,
            Err(ProfileSyncError::Authentication)
        ));
        assert_eq!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT bsky_handle FROM oauth_accounts WHERE id='at-account'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn profile_write_fault_rolls_back_sync_and_local_profile() {
        let (pool, _, actor, service) = fixture().await;
        sqlx::query("CREATE TRIGGER fail_profile_sync BEFORE INSERT ON user_profiles BEGIN SELECT RAISE(ABORT,'injected profile failure'); END")
            .execute(&pool).await.unwrap();
        assert!(matches!(
            service.apply(&actor, "did:plc:alice", &remote()).await,
            Err(ProfileSyncError::Database(_))
        ));
        let values: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT bsky_handle,(SELECT avatar_url FROM users WHERE id='local-user') \
             FROM oauth_accounts WHERE id='at-account'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(values, (None, None));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM user_profiles")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }
}
