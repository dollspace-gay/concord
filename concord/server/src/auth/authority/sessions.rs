use super::{
    Actor, AuthError, AuthService, CredentialKind, Utc, Uuid, create_session_token_with_id,
    rfc1459_casefold, stable_user_irc_nickname, validate_session_token,
};

impl AuthService {
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
}
