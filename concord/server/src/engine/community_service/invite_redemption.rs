use super::{Actor, CommunityError, CommunityService, RedeemedInvite, ServerId};

impl CommunityService {
    pub async fn redeem_invite(
        &self,
        actor: &Actor,
        code: &str,
    ) -> Result<RedeemedInvite, CommunityError> {
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        self.auth
            .validate_actor_in(&mut tx, actor)
            .await
            .map_err(CommunityError::from)?;
        let invite: Option<(String, String)> = sqlx::query_as(
            "SELECT id,server_id FROM invites WHERE code=? \
             AND (expires_at IS NULL OR julianday(expires_at)>julianday('now'))",
        )
        .bind(code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        let (invite_id, server_id) = invite.ok_or(CommunityError::InvalidInput(
            "invalid or expired invite code",
        ))?;
        let banned: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM bans WHERE server_id=? AND user_id=?)")
                .bind(&server_id)
                .bind(actor.user_id().as_str())
                .fetch_one(&mut *tx)
                .await
                .map_err(CommunityError::from)?;
        if banned {
            return Err(CommunityError::Forbidden);
        }
        let already_member: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id=? AND user_id=?)",
        )
        .bind(&server_id)
        .bind(actor.user_id().as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        if !already_member {
            let used = sqlx::query(
                "UPDATE invites SET use_count=use_count+1 WHERE id=? \
                 AND (max_uses IS NULL OR use_count<max_uses) \
                 AND NOT EXISTS(SELECT 1 FROM bans WHERE server_id=? AND user_id=?)",
            )
            .bind(&invite_id)
            .bind(&server_id)
            .bind(actor.user_id().as_str())
            .execute(&mut *tx)
            .await
            .map_err(CommunityError::from)?;
            if used.rows_affected() != 1 {
                return Err(CommunityError::Forbidden);
            }
            sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES(?,?,'member')")
                .bind(&server_id)
                .bind(actor.user_id().as_str())
                .execute(&mut *tx)
                .await
                .map_err(CommunityError::from)?;
        }
        tx.commit().await.map_err(CommunityError::from)?;
        Ok(RedeemedInvite {
            server_id: ServerId::from_stored(server_id)?,
        })
    }

    /// Record acceptance of the server's current rules version.
    ///
    /// The membership check and version read happen in the same admitted write
    /// transaction, so a concurrent rules edit cannot leave a stale version
    /// recorded as current.
    pub async fn accept_rules(
        &self,
        actor: &Actor,
        server_id: &ServerId,
    ) -> Result<i64, CommunityError> {
        let server_id = server_id.as_str();
        let (_permit, mut tx) = self.writes.begin().await.map_err(CommunityError::from)?;
        self.auth
            .validate_actor_in(&mut tx, actor)
            .await
            .map_err(CommunityError::from)?;
        let accepted_version: Option<i64> = sqlx::query_scalar(
            "UPDATE server_members \
             SET rules_accepted=1,accepted_rules_version=( \
                 SELECT rules_version FROM servers WHERE id=? \
             ) WHERE server_id=? AND user_id=? \
             RETURNING accepted_rules_version",
        )
        .bind(server_id)
        .bind(server_id)
        .bind(actor.user_id().as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(CommunityError::from)?;
        let accepted_version = accepted_version.ok_or(CommunityError::Forbidden)?;
        tx.commit().await.map_err(CommunityError::from)?;
        Ok(accepted_version)
    }
}
