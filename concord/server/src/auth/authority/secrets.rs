use super::{
    Actor, AuthError, AuthService, CredentialId, CredentialKind, CredentialRow, IssuedCredential,
    MAX_LEGACY_CANDIDATES, Rng, SecretCandidate, UserId, Uuid, actor_from_row, hash_irc_token,
    legacy_bot_user_hint, parse_indexed_token, rfc1459_casefold, stable_user_irc_nickname,
    verify_hash_with_permit,
};

impl AuthService {
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

    pub(super) async fn authenticate_secret(
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

    pub(super) async fn issue_secret(
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

    pub(super) async fn verify_hash(&self, token: String, hash: String) -> Result<bool, AuthError> {
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

    pub(super) async fn ensure_enabled(&self, user_id: &str) -> Result<(), AuthError> {
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

    pub(super) async fn load_actor(&self, credential_id: &str) -> Result<Actor, AuthError> {
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
