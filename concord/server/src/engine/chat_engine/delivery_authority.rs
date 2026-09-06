use super::ChatEngine;

impl ChatEngine {
    pub async fn authorization_stamp_is_current(
        &self,
        actor: &crate::auth::authority::Actor,
        stamp: &crate::engine::authorization::AuthorizationStamp,
    ) -> bool {
        let Some(pool) = &self.db else {
            return false;
        };
        let Some(auth) = self.auth.get() else {
            return false;
        };
        if auth.validate_actor(actor).await.is_err() {
            return false;
        }
        crate::engine::authorization::AuthorizationService::new(pool.clone())
            .stamp_is_current(stamp)
            .await
            .unwrap_or(false)
    }
    /// Revalidate queued authorization evidence at the final transport boundary.
    pub async fn delivery_guard_is_current(
        &self,
        actor: &crate::auth::authority::Actor,
        guard: &crate::engine::user_session::DeliveryGuard,
    ) -> bool {
        use crate::engine::authorization::ConversationAction;
        use crate::engine::user_session::DeliveryGuard;

        match guard {
            DeliveryGuard::ActorCurrent => self.actor_is_current(actor).await,
            DeliveryGuard::Stamps(stamps) => {
                for stamp in stamps {
                    if !self.authorization_stamp_is_current(actor, stamp).await {
                        return false;
                    }
                }
                true
            }
            DeliveryGuard::Conversations(conversation_ids) => {
                let (Some(pool), Some(auth)) = (&self.db, self.auth.get()) else {
                    return false;
                };
                let service = crate::engine::authorization::AuthorizationService::new(pool.clone());
                let Ok(mut connection) = pool.acquire().await else {
                    return false;
                };
                for conversation_id in conversation_ids {
                    if service
                        .authorize_conversation_actor_in(
                            &mut connection,
                            auth,
                            actor,
                            conversation_id,
                            ConversationAction::Read,
                        )
                        .await
                        .is_err()
                    {
                        return false;
                    }
                }
                true
            }
            DeliveryGuard::Channels(channel_ids) => {
                let (Some(pool), Some(auth)) = (&self.db, self.auth.get()) else {
                    return false;
                };
                let service = crate::engine::authorization::AuthorizationService::new(pool.clone());
                let Ok(mut connection) = pool.acquire().await else {
                    return false;
                };
                for channel_id in channel_ids {
                    if service
                        .authorize_actor_in(
                            &mut connection,
                            auth,
                            actor,
                            channel_id,
                            crate::engine::authorization::ChannelAction::View,
                        )
                        .await
                        .is_err()
                    {
                        return false;
                    }
                }
                true
            }
            DeliveryGuard::ChannelActions(requirements) => {
                let (Some(pool), Some(auth)) = (&self.db, self.auth.get()) else {
                    return false;
                };
                let service = crate::engine::authorization::AuthorizationService::new(pool.clone());
                let Ok(mut connection) = pool.acquire().await else {
                    return false;
                };
                for (channel_id, action) in requirements {
                    if service
                        .authorize_actor_in(&mut connection, auth, actor, channel_id, *action)
                        .await
                        .is_err()
                    {
                        return false;
                    }
                }
                true
            }
            DeliveryGuard::ServerMembership(server_ids) => {
                let (Some(pool), Some(auth)) = (&self.db, self.auth.get()) else {
                    return false;
                };
                let Ok(mut connection) = pool.acquire().await else {
                    return false;
                };
                if auth
                    .validate_actor_in(&mut connection, actor)
                    .await
                    .is_err()
                {
                    return false;
                }
                for server_id in server_ids {
                    let current: Result<bool, _> = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM server_members sm \
                         WHERE sm.server_id=? AND sm.user_id=? \
                         AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=sm.server_id AND b.user_id=sm.user_id))",
                    )
                    .bind(server_id)
                    .bind(actor.user_id().as_str())
                    .fetch_one(&mut *connection)
                    .await;
                    if !matches!(current, Ok(true)) {
                        return false;
                    }
                }
                true
            }
            DeliveryGuard::ServerPermissions(requirements) => {
                let (Some(pool), Some(auth)) = (&self.db, self.auth.get()) else {
                    return false;
                };
                let service = crate::engine::authorization::AuthorizationService::new(pool.clone());
                let Ok(mut connection) = pool.acquire().await else {
                    return false;
                };
                for (server_id, permissions) in requirements {
                    if service
                        .require_server_actor_in(
                            &mut connection,
                            auth,
                            actor,
                            server_id,
                            *permissions,
                        )
                        .await
                        .is_err()
                    {
                        return false;
                    }
                }
                true
            }
            DeliveryGuard::BotInstallationScopes(requirements) => {
                let (Some(pool), Some(auth)) = (&self.db, self.auth.get()) else {
                    return false;
                };
                let service = crate::engine::authorization::AuthorizationService::new(pool.clone());
                for (server_id, scope) in requirements {
                    if service
                        .authorize_bot_installation_scope(auth, actor, server_id, scope)
                        .await
                        .is_err()
                    {
                        return false;
                    }
                }
                true
            }
        }
    }
    pub async fn actor_is_current(&self, actor: &crate::auth::authority::Actor) -> bool {
        let Some(auth) = self.auth.get() else {
            return false;
        };
        auth.validate_actor(actor).await.is_ok()
    }
}
