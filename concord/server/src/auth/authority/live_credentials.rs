use super::{
    Actor, AtomicUsize, AuthError, AuthService, CancellationToken, CredentialId, CredentialLease,
    LiveCredential, Ordering, SqliteConnection, UserId,
};

impl AuthService {
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
}
