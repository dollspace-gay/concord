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

mod actor_validation;
mod live_credentials;
mod secrets;
mod sessions;
mod token_issuance;
