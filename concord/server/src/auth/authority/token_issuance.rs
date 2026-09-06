use super::{
    AuthError, AuthService, CredentialId, CredentialKind, IssuedCredential, PreparedBotCredential,
    SqliteConnection, UserId, Uuid,
};

impl AuthService {
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
}
