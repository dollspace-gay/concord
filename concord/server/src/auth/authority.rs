use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Utc;
use dashmap::DashMap;
use rand::Rng;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqliteConnection, SqlitePool};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::token::{
    create_session_token_with_id, hash_irc_token, validate_session_token, verify_irc_token,
};

const MAX_LEGACY_CANDIDATES: usize = 32;
const DEFAULT_HASH_CONCURRENCY: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UserId(String);

impl UserId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_stored(value: impl Into<String>) -> Result<Self, AuthError> {
        let value = value.into();
        if value.is_empty() {
            return Err(AuthError::Invalid);
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CredentialId(String);

impl CredentialId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_stored(value: impl Into<String>) -> Result<Self, AuthError> {
        let value = value.into();
        if value.is_empty() {
            return Err(AuthError::Invalid);
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialKind {
    WebSession,
    IrcToken,
    BotToken,
}

impl CredentialKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::WebSession => "web_session",
            Self::IrcToken => "irc_token",
            Self::BotToken => "bot_token",
        }
    }

    fn token_prefix(self) -> Option<&'static str> {
        match self {
            Self::WebSession => None,
            Self::IrcToken => Some("cc_irc_"),
            Self::BotToken => Some("cc_bot_"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialScopes(Arc<[String]>);

impl CredentialScopes {
    pub fn parse(value: &str) -> Self {
        let scopes = value
            .split([',', ' '])
            .filter(|scope| !scope.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        Self(scopes.into())
    }

    pub fn contains(&self, scope: &str) -> bool {
        self.0.iter().any(|candidate| candidate == scope)
    }

    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Actor {
    user_id: UserId,
    credential_id: CredentialId,
    kind: CredentialKind,
    scopes: CredentialScopes,
    expires_at: Option<i64>,
    credential_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedCredential {
    pub token_id: String,
    pub secret: String,
    pub credential_id: CredentialId,
}

/// A one-time bot credential whose expensive secret hash is ready to be
/// inserted as part of a caller-owned transaction.
pub(crate) struct PreparedBotCredential {
    pub token_id: String,
    pub secret: String,
    pub credential_id: CredentialId,
    secret_hash: String,
    name: String,
    scopes: String,
}

impl PreparedBotCredential {
    pub(crate) fn credential_id(&self) -> &CredentialId {
        &self.credential_id
    }

    pub(crate) fn secret(&self) -> &str {
        &self.secret
    }

    fn into_issued(self) -> IssuedCredential {
        IssuedCredential {
            token_id: self.token_id,
            secret: self.secret,
            credential_id: self.credential_id,
        }
    }
}

impl Actor {
    pub fn user_id(&self) -> &UserId {
        &self.user_id
    }
    pub fn credential_id(&self) -> &CredentialId {
        &self.credential_id
    }
    pub fn kind(&self) -> CredentialKind {
        self.kind
    }
    pub fn scopes(&self) -> &CredentialScopes {
        &self.scopes
    }
    pub fn expires_at(&self) -> Option<i64> {
        self.expires_at
    }
    pub fn credential_version(&self) -> i64 {
        self.credential_version
    }
}

#[derive(Debug)]
pub enum AuthError {
    Invalid,
    Expired,
    Revoked,
    Disabled,
    VerificationBusy,
    Database(sqlx::Error),
    Token(jsonwebtoken::errors::Error),
    HashWorker(String),
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Invalid => "invalid credential",
            Self::Expired => "expired credential",
            Self::Revoked => "revoked credential",
            Self::Disabled => "disabled account",
            Self::VerificationBusy => "credential verifier is busy",
            Self::Database(_) => "credential database operation failed",
            Self::Token(_) => "invalid session token",
            Self::HashWorker(_) => "credential verification worker failed",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for AuthError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Token(error) => Some(error),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for AuthError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

#[derive(Clone)]
pub struct AuthService {
    inner: Arc<AuthServiceInner>,
}

struct AuthServiceInner {
    pool: SqlitePool,
    jwt_secret: Arc<str>,
    session_expiry_hours: i64,
    hash_workers: Arc<Semaphore>,
    live_credentials: DashMap<CredentialId, LiveCredential>,
}

struct LiveCredential {
    cancelled: CancellationToken,
    connections: AtomicUsize,
}

pub struct CredentialLease {
    service: AuthService,
    credential_id: CredentialId,
    cancelled: CancellationToken,
}

pub async fn wait_for_expiry(expires_at: Option<i64>) {
    match expires_at {
        Some(expires_at) => {
            let remaining = expires_at.saturating_sub(Utc::now().timestamp()).max(0) as u64;
            tokio::time::sleep(std::time::Duration::from_secs(remaining)).await;
        }
        None => std::future::pending::<()>().await,
    }
}

impl CredentialLease {
    pub fn cancelled(&self) -> impl std::future::Future<Output = ()> + '_ {
        self.cancelled.cancelled()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancelled.clone()
    }
}

impl Drop for CredentialLease {
    fn drop(&mut self) {
        if let dashmap::mapref::entry::Entry::Occupied(entry) = self
            .service
            .inner
            .live_credentials
            .entry(self.credential_id.clone())
        {
            if entry.get().connections.load(Ordering::Relaxed) <= 1 {
                entry.remove();
            } else {
                entry.get().connections.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
}

#[derive(FromRow)]
struct CredentialRow {
    id: String,
    user_id: String,
    kind: String,
    scopes: String,
    expires_at: Option<i64>,
    revoked_at: Option<i64>,
    version: i64,
    disabled_at: Option<String>,
}

#[derive(FromRow)]
struct SecretCandidate {
    id: String,
    secret_hash: String,
}

impl AuthService {
    pub fn new(pool: SqlitePool, jwt_secret: String, session_expiry_hours: i64) -> Self {
        Self::with_hash_concurrency(
            pool,
            jwt_secret,
            session_expiry_hours,
            DEFAULT_HASH_CONCURRENCY,
        )
    }

    pub fn with_hash_concurrency(
        pool: SqlitePool,
        jwt_secret: String,
        session_expiry_hours: i64,
        hash_concurrency: usize,
    ) -> Self {
        Self {
            inner: Arc::new(AuthServiceInner {
                pool,
                jwt_secret: jwt_secret.into(),
                session_expiry_hours,
                hash_workers: Arc::new(Semaphore::new(hash_concurrency.max(1))),
                live_credentials: DashMap::new(),
            }),
        }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.inner.pool
    }

    pub async fn issue_web_session(&self, user_id: &str) -> Result<(String, Actor), AuthError> {
        self.ensure_enabled(user_id).await?;
        let now = Utc::now().timestamp();
        let expires_at = now + self.inner.session_expiry_hours * 3600;
        let credential_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO auth_credentials (id,user_id,kind,token_id,scopes,expires_at) \
             VALUES (?,?,'web_session',?,'web',?)",
        )
        .bind(&credential_id)
        .bind(user_id)
        .bind(&credential_id)
        .bind(expires_at)
        .execute(&self.inner.pool)
        .await?;

        let token = match create_session_token_with_id(
            user_id,
            &self.inner.jwt_secret,
            now,
            expires_at,
            &credential_id,
        ) {
            Ok(token) => token,
            Err(error) => {
                let _ = sqlx::query("DELETE FROM auth_credentials WHERE id = ?")
                    .bind(&credential_id)
                    .execute(&self.inner.pool)
                    .await;
                return Err(AuthError::Token(error));
            }
        };
        let actor = self.load_actor(&credential_id).await?;
        Ok((token, actor))
    }

    pub async fn authenticate_web_session(&self, token: &str) -> Result<Actor, AuthError> {
        let claims =
            validate_session_token(token, &self.inner.jwt_secret).map_err(AuthError::Token)?;
        if claims.jti.is_empty() {
            return Err(AuthError::Invalid);
        }
        let actor = self.load_actor(&claims.jti).await?;
        if actor.kind() != CredentialKind::WebSession || actor.user_id().as_str() != claims.sub {
            return Err(AuthError::Invalid);
        }
        Ok(actor)
    }

    pub async fn authenticate_irc(&self, token: &str, nickname: &str) -> Result<Actor, AuthError> {
        self.authenticate_secret(CredentialKind::IrcToken, token, Some(nickname))
            .await
    }

    /// Return the durable server-assigned IRC nickname for this exact actor.
    /// Client supplied NICK/authcid values never select another user's identity.
    pub async fn canonical_irc_nickname(&self, actor: &Actor) -> Result<String, AuthError> {
        self.validate_actor(actor).await?;
        if actor.kind() != CredentialKind::IrcToken {
            return Err(AuthError::Invalid);
        }
        if let Some(nickname) =
            sqlx::query_scalar("SELECT nickname FROM irc_nick_reservations WHERE user_id=?")
                .bind(actor.user_id().as_str())
                .fetch_optional(&self.inner.pool)
                .await?
        {
            return Ok(nickname);
        }
        let nickname = stable_user_irc_nickname(actor.user_id().as_str());
        let folded = rfc1459_casefold(&nickname);
        sqlx::query(
            "INSERT INTO irc_nick_reservations(nick_casefold,nickname,user_id) VALUES(?,?,?) \
             ON CONFLICT(user_id) DO NOTHING",
        )
        .bind(folded)
        .bind(&nickname)
        .bind(actor.user_id().as_str())
        .execute(&self.inner.pool)
        .await?;
        sqlx::query_scalar("SELECT nickname FROM irc_nick_reservations WHERE user_id=?")
            .bind(actor.user_id().as_str())
            .fetch_one(&self.inner.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn authenticate_bot(&self, token: &str) -> Result<Actor, AuthError> {
        self.authenticate_secret(CredentialKind::BotToken, token, None)
            .await
    }

    pub async fn issue_irc_token(
        &self,
        user_id: &UserId,
        label: Option<&str>,
    ) -> Result<IssuedCredential, AuthError> {
        self.issue_secret(
            CredentialKind::IrcToken,
            user_id,
            "irc",
            label.unwrap_or("IRC token"),
        )
        .await
    }

    pub async fn issue_bot_token(
        &self,
        user_id: &UserId,
        name: &str,
        scopes: &str,
    ) -> Result<IssuedCredential, AuthError> {
        self.ensure_enabled(user_id.as_str()).await?;
        let prepared = self.prepare_bot_token(name, scopes).await?;
        let mut transaction = self.inner.pool.begin().await?;
        self.insert_prepared_bot_in(&mut transaction, user_id, &prepared)
            .await?;
        transaction.commit().await?;
        Ok(prepared.into_issued())
    }

    pub(crate) async fn prepare_bot_token(
        &self,
        name: &str,
        scopes: &str,
    ) -> Result<PreparedBotCredential, AuthError> {
        let token_id = Uuid::new_v4().to_string();
        let credential_id = CredentialId(Uuid::new_v4().to_string());
        let secret = Self::generate_indexed_token(CredentialKind::BotToken, &token_id)?;
        let secret_hash = self.hash_secret(secret.clone()).await?;
        Ok(PreparedBotCredential {
            token_id,
            secret,
            credential_id,
            secret_hash,
            name: name.to_owned(),
            scopes: scopes.to_owned(),
        })
    }

    pub(crate) async fn insert_prepared_bot_in(
        &self,
        connection: &mut SqliteConnection,
        user_id: &UserId,
        prepared: &PreparedBotCredential,
    ) -> Result<(), AuthError> {
        let disabled: Option<Option<String>> =
            sqlx::query_scalar("SELECT disabled_at FROM users WHERE id=?")
                .bind(user_id.as_str())
                .fetch_optional(&mut *connection)
                .await?;
        if !matches!(disabled, Some(None)) {
            return Err(match disabled {
                Some(Some(_)) => AuthError::Disabled,
                _ => AuthError::Invalid,
            });
        }
        sqlx::query(
            "INSERT INTO auth_credentials \
             (id,user_id,kind,token_id,secret_hash,scopes) VALUES (?,?,?,?,?,?)",
        )
        .bind(prepared.credential_id.as_str())
        .bind(user_id.as_str())
        .bind(CredentialKind::BotToken.as_str())
        .bind(&prepared.token_id)
        .bind(&prepared.secret_hash)
        .bind(&prepared.scopes)
        .execute(&mut *connection)
        .await?;
        sqlx::query(
            "INSERT INTO bot_tokens \
             (id,user_id,token_hash,name,scopes,credential_id,token_id) VALUES (?,?,?,?,?,?,?)",
        )
        .bind(&prepared.token_id)
        .bind(user_id.as_str())
        .bind(&prepared.secret_hash)
        .bind(&prepared.name)
        .bind(&prepared.scopes)
        .bind(prepared.credential_id.as_str())
        .bind(&prepared.token_id)
        .execute(connection)
        .await?;
        Ok(())
    }

    pub async fn validate_actor(&self, actor: &Actor) -> Result<(), AuthError> {
        let current = self.load_actor(actor.credential_id.as_str()).await?;
        compare_actor(actor, &current)
    }

    /// Revalidates an actor on the caller's connection so authorization and a
    /// mutation can share one transaction snapshot.
    pub async fn validate_actor_in(
        &self,
        connection: &mut SqliteConnection,
        actor: &Actor,
    ) -> Result<(), AuthError> {
        let current = load_actor_from_connection(connection, actor.credential_id.as_str()).await?;
        compare_actor(actor, &current)
    }

    pub async fn actor_in(
        &self,
        connection: &mut SqliteConnection,
        credential_id: &CredentialId,
    ) -> Result<Actor, AuthError> {
        load_actor_from_connection(connection, credential_id.as_str()).await
    }

    fn actor_matches(actor: &Actor, current: &Actor) -> bool {
        if current.user_id != actor.user_id
            || current.kind != actor.kind
            || current.scopes != actor.scopes
            || current.credential_version != actor.credential_version
        {
            return false;
        }
        true
    }

    pub async fn register_live(&self, actor: &Actor) -> Result<CredentialLease, AuthError> {
        let cancelled = match self
            .inner
            .live_credentials
            .entry(actor.credential_id.clone())
        {
            dashmap::mapref::entry::Entry::Occupied(entry) => {
                entry.get().connections.fetch_add(1, Ordering::Relaxed);
                entry.get().cancelled.clone()
            }
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                let cancelled = CancellationToken::new();
                entry.insert(LiveCredential {
                    cancelled: cancelled.clone(),
                    connections: AtomicUsize::new(1),
                });
                cancelled
            }
        };
        let lease = CredentialLease {
            service: self.clone(),
            credential_id: actor.credential_id.clone(),
            cancelled,
        };
        self.validate_actor(actor).await?;
        Ok(lease)
    }

    pub async fn revoke_credential(&self, credential_id: &CredentialId) -> Result<bool, AuthError> {
        let mut transaction = self.inner.pool.begin().await?;
        let revoked = self
            .revoke_credential_in(&mut transaction, credential_id)
            .await?;
        transaction.commit().await?;
        if revoked {
            self.cancel_live_credential(credential_id);
        }
        Ok(revoked)
    }

    pub(crate) async fn revoke_credential_in(
        &self,
        connection: &mut SqliteConnection,
        credential_id: &CredentialId,
    ) -> Result<bool, AuthError> {
        let result = sqlx::query(
            "UPDATE auth_credentials SET revoked_at=unixepoch(), version=version+1 \
             WHERE id=? AND revoked_at IS NULL",
        )
        .bind(credential_id.as_str())
        .execute(connection)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub(crate) fn cancel_live_credential(&self, credential_id: &CredentialId) {
        if let Some((_, live)) = self.inner.live_credentials.remove(credential_id) {
            live.cancelled.cancel();
        }
    }

    pub async fn revoke_all_for_user(&self, user_id: &UserId) -> Result<u64, AuthError> {
        let ids: Vec<String> = sqlx::query_scalar(
            "UPDATE auth_credentials SET revoked_at=unixepoch(), version=version+1 \
             WHERE user_id=? AND revoked_at IS NULL RETURNING id",
        )
        .bind(user_id.as_str())
        .fetch_all(&self.inner.pool)
        .await?;
        let revoked = ids.len() as u64;
        for id in ids {
            let id = CredentialId(id);
            if let Some((_, live)) = self.inner.live_credentials.remove(&id) {
                live.cancelled.cancel();
            }
        }
        Ok(revoked)
    }

    pub async fn revoke_irc_token(
        &self,
        token_id: &str,
        user_id: &UserId,
    ) -> Result<bool, AuthError> {
        let credential_id: Option<String> =
            sqlx::query_scalar("SELECT credential_id FROM irc_tokens WHERE id=? AND user_id=?")
                .bind(token_id)
                .bind(user_id.as_str())
                .fetch_optional(&self.inner.pool)
                .await?
                .flatten();
        let Some(credential_id) = credential_id else {
            return Ok(false);
        };
        let credential_id = CredentialId(credential_id);
        self.revoke_credential(&credential_id).await?;
        sqlx::query("DELETE FROM irc_tokens WHERE id=? AND user_id=?")
            .bind(token_id)
            .bind(user_id.as_str())
            .execute(&self.inner.pool)
            .await?;
        Ok(true)
    }

    pub async fn revoke_bot_token(&self, token_id: &str) -> Result<bool, AuthError> {
        let credential_id: Option<String> =
            sqlx::query_scalar("SELECT credential_id FROM bot_tokens WHERE id=?")
                .bind(token_id)
                .fetch_optional(&self.inner.pool)
                .await?
                .flatten();
        let Some(credential_id) = credential_id else {
            return Ok(false);
        };
        let credential_id = CredentialId(credential_id);
        self.revoke_credential(&credential_id).await?;
        sqlx::query("DELETE FROM bot_tokens WHERE id=?")
            .bind(token_id)
            .execute(&self.inner.pool)
            .await?;
        Ok(true)
    }

    pub fn generate_indexed_token(
        kind: CredentialKind,
        token_id: &str,
    ) -> Result<String, AuthError> {
        let prefix = kind.token_prefix().ok_or(AuthError::Invalid)?;
        let mut secret = [0_u8; 32];
        rand::rng().fill_bytes(&mut secret);
        let encoded = secret
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok(format!("{prefix}{token_id}_{encoded}"))
    }

    async fn authenticate_secret(
        &self,
        kind: CredentialKind,
        token: &str,
        nickname: Option<&str>,
    ) -> Result<Actor, AuthError> {
        let candidates = if let Some(token_id) = parse_indexed_token(kind, token) {
            sqlx::query_as::<_, SecretCandidate>(
                "SELECT id,secret_hash FROM auth_credentials WHERE kind=? AND token_id=?",
            )
            .bind(kind.as_str())
            .bind(token_id)
            .fetch_all(&self.inner.pool)
            .await?
        } else {
            let rows = if let Some(nickname) = nickname {
                sqlx::query_as::<_, SecretCandidate>(
                    "SELECT DISTINCT c.id,c.secret_hash FROM auth_credentials c JOIN users u ON u.id=c.user_id \
                     LEFT JOIN user_nicknames n ON n.user_id=u.id \
                     WHERE c.kind=? AND c.token_id IS NULL AND (u.username=? OR n.nickname=?) LIMIT 33",
                )
                .bind(kind.as_str())
                .bind(nickname)
                .bind(nickname)
                .fetch_all(&self.inner.pool)
                .await?
            } else if let Some(user_id) = legacy_bot_user_hint(token) {
                sqlx::query_as::<_, SecretCandidate>(
                    "SELECT id,secret_hash FROM auth_credentials \
                     WHERE kind=? AND token_id IS NULL AND user_id=? LIMIT 33",
                )
                .bind(kind.as_str())
                .bind(user_id)
                .fetch_all(&self.inner.pool)
                .await?
            } else {
                sqlx::query_as::<_, SecretCandidate>(
                    "SELECT id,secret_hash FROM auth_credentials WHERE kind=? AND token_id IS NULL LIMIT 33",
                )
                .bind(kind.as_str())
                .fetch_all(&self.inner.pool)
                .await?
            };
            if rows.len() > MAX_LEGACY_CANDIDATES {
                return Err(AuthError::VerificationBusy);
            }
            rows
        };

        for candidate in candidates {
            if self
                .verify_hash(token.to_owned(), candidate.secret_hash)
                .await?
            {
                sqlx::query("UPDATE auth_credentials SET last_used_at=unixepoch() WHERE id=?")
                    .bind(&candidate.id)
                    .execute(&self.inner.pool)
                    .await?;
                return self.load_actor(&candidate.id).await;
            }
        }
        Err(AuthError::Invalid)
    }

    async fn issue_secret(
        &self,
        kind: CredentialKind,
        user_id: &UserId,
        scopes: &str,
        label: &str,
    ) -> Result<IssuedCredential, AuthError> {
        self.ensure_enabled(user_id.as_str()).await?;
        let token_id = Uuid::new_v4().to_string();
        let credential_id = CredentialId(Uuid::new_v4().to_string());
        let secret = Self::generate_indexed_token(kind, &token_id)?;
        let secret_hash = self.hash_secret(secret.clone()).await?;
        let mut transaction = self.inner.pool.begin().await?;
        sqlx::query(
            "INSERT INTO auth_credentials \
             (id,user_id,kind,token_id,secret_hash,scopes) VALUES (?,?,?,?,?,?)",
        )
        .bind(credential_id.as_str())
        .bind(user_id.as_str())
        .bind(kind.as_str())
        .bind(&token_id)
        .bind(&secret_hash)
        .bind(scopes)
        .execute(&mut *transaction)
        .await?;
        match kind {
            CredentialKind::IrcToken => {
                let nickname = stable_user_irc_nickname(user_id.as_str());
                sqlx::query(
                    "INSERT INTO irc_nick_reservations(nick_casefold,nickname,user_id) VALUES(?,?,?) \
                     ON CONFLICT(user_id) DO NOTHING",
                )
                .bind(rfc1459_casefold(&nickname))
                .bind(nickname)
                .bind(user_id.as_str())
                .execute(&mut *transaction)
                .await?;
                sqlx::query(
                    "INSERT INTO irc_tokens \
                     (id,user_id,token_hash,label,credential_id,token_id) VALUES (?,?,?,?,?,?)",
                )
                .bind(&token_id)
                .bind(user_id.as_str())
                .bind(&secret_hash)
                .bind(label)
                .bind(credential_id.as_str())
                .bind(&token_id)
                .execute(&mut *transaction)
                .await?;
            }
            CredentialKind::BotToken => {
                sqlx::query(
                    "INSERT INTO bot_tokens \
                     (id,user_id,token_hash,name,scopes,credential_id,token_id) VALUES (?,?,?,?,?,?,?)",
                )
                .bind(&token_id)
                .bind(user_id.as_str())
                .bind(&secret_hash)
                .bind(label)
                .bind(scopes)
                .bind(credential_id.as_str())
                .bind(&token_id)
                .execute(&mut *transaction)
                .await?;
            }
            CredentialKind::WebSession => return Err(AuthError::Invalid),
        }
        transaction.commit().await?;
        Ok(IssuedCredential {
            token_id,
            secret,
            credential_id,
        })
    }

    async fn verify_hash(&self, token: String, hash: String) -> Result<bool, AuthError> {
        let permit = self
            .inner
            .hash_workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| AuthError::VerificationBusy)?;
        tokio::task::spawn_blocking(move || verify_hash_with_permit(permit, token, hash))
            .await
            .map_err(|error| AuthError::HashWorker(error.to_string()))
    }

    pub(crate) async fn verify_secret_hash(
        &self,
        token: String,
        hash: String,
    ) -> Result<bool, AuthError> {
        self.verify_hash(token, hash).await
    }

    pub(crate) async fn hash_secret(&self, token: String) -> Result<String, AuthError> {
        let permit = self
            .inner
            .hash_workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| AuthError::VerificationBusy)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            hash_irc_token(&token).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| AuthError::HashWorker(error.to_string()))?
        .map_err(AuthError::HashWorker)
    }

    async fn ensure_enabled(&self, user_id: &str) -> Result<(), AuthError> {
        let disabled: Option<Option<String>> =
            sqlx::query_scalar("SELECT disabled_at FROM users WHERE id=?")
                .bind(user_id)
                .fetch_optional(&self.inner.pool)
                .await?;
        match disabled {
            Some(None) => Ok(()),
            Some(Some(_)) => Err(AuthError::Disabled),
            None => Err(AuthError::Invalid),
        }
    }

    async fn load_actor(&self, credential_id: &str) -> Result<Actor, AuthError> {
        let row = sqlx::query_as::<_, CredentialRow>(
            "SELECT c.id,c.user_id,c.kind,c.scopes,c.expires_at,c.revoked_at,c.version,u.disabled_at \
             FROM auth_credentials c JOIN users u ON u.id=c.user_id WHERE c.id=?",
        )
        .bind(credential_id)
        .fetch_optional(&self.inner.pool)
        .await?
        .ok_or(AuthError::Invalid)?;
        actor_from_row(row)
    }
}

pub fn rfc1459_casefold(nickname: &str) -> String {
    nickname
        .chars()
        .map(|character| match character {
            'A'..='Z' => character.to_ascii_lowercase(),
            '[' => '{',
            ']' => '}',
            '\\' => '|',
            '^' => '~',
            _ => character,
        })
        .collect()
}

fn stable_user_irc_nickname(user_id: &str) -> String {
    let digest = Sha256::digest(user_id.as_bytes());
    format!("u-{}", hex::encode(&digest[..15]))
}

fn compare_actor(actor: &Actor, current: &Actor) -> Result<(), AuthError> {
    AuthService::actor_matches(actor, current)
        .then_some(())
        .ok_or(AuthError::Revoked)
}

async fn load_actor_from_connection(
    connection: &mut SqliteConnection,
    credential_id: &str,
) -> Result<Actor, AuthError> {
    let row = sqlx::query_as::<_, CredentialRow>(
        "SELECT c.id,c.user_id,c.kind,c.scopes,c.expires_at,c.revoked_at,c.version,u.disabled_at \
         FROM auth_credentials c JOIN users u ON u.id=c.user_id WHERE c.id=?",
    )
    .bind(credential_id)
    .fetch_optional(connection)
    .await?
    .ok_or(AuthError::Invalid)?;
    actor_from_row(row)
}

fn actor_from_row(row: CredentialRow) -> Result<Actor, AuthError> {
    if row.disabled_at.is_some() {
        return Err(AuthError::Disabled);
    }
    if row.revoked_at.is_some() {
        return Err(AuthError::Revoked);
    }
    if row
        .expires_at
        .is_some_and(|expires| expires <= Utc::now().timestamp())
    {
        return Err(AuthError::Expired);
    }
    let kind = match row.kind.as_str() {
        "web_session" => CredentialKind::WebSession,
        "irc_token" => CredentialKind::IrcToken,
        "bot_token" => CredentialKind::BotToken,
        _ => return Err(AuthError::Invalid),
    };
    Ok(Actor {
        user_id: UserId(row.user_id),
        credential_id: CredentialId(row.id),
        kind,
        scopes: CredentialScopes::parse(&row.scopes),
        expires_at: row.expires_at,
        credential_version: row.version,
    })
}

fn verify_hash_with_permit(_permit: OwnedSemaphorePermit, token: String, hash: String) -> bool {
    verify_irc_token(&token, &hash)
}

fn parse_indexed_token(kind: CredentialKind, token: &str) -> Option<&str> {
    let rest = token.strip_prefix(kind.token_prefix()?)?;
    let (token_id, secret) = rest.split_once('_')?;
    (!token_id.is_empty()
        && secret.len() == 64
        && secret.bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then_some(token_id)
}

fn legacy_bot_user_hint(token: &str) -> Option<&str> {
    let (user_id, secret) = token.strip_prefix("bot_")?.split_once('.')?;
    (!user_id.is_empty() && !secret.is_empty()).then_some(user_id)
}
