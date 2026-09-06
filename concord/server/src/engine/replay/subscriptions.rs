use super::{
    Actor, AuthService, AuthorizationError, AuthorizationService, ConversationAction,
    ConversationId, Digest, MAX_SUBSCRIPTIONS, ReplayError, ResyncReason, Sha256, SqliteConnection,
};

pub(super) fn canonical_subscriptions(
    subscriptions: &[String],
) -> Result<Vec<ConversationId>, ReplayError> {
    if subscriptions.len() > MAX_SUBSCRIPTIONS {
        return Err(ReplayError::InvalidInput);
    }
    let mut canonical = subscriptions
        .iter()
        .map(|value| ConversationId::from_stored(value.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ReplayError::InvalidInput)?;
    canonical.sort();
    canonical.dedup();
    Ok(canonical)
}

pub(super) fn subscription_hash(subscriptions: &[ConversationId]) -> String {
    let mut hasher = Sha256::new();
    for subscription in subscriptions {
        hasher.update((subscription.as_str().len() as u64).to_be_bytes());
        hasher.update(subscription.as_str().as_bytes());
    }
    hex::encode(hasher.finalize())
}

pub(super) async fn authorize_conversation(
    authorization: &AuthorizationService,
    auth: &AuthService,
    connection: &mut SqliteConnection,
    actor: &Actor,
    conversation_id: &str,
) -> Result<(), ReplayError> {
    authorization
        .authorize_conversation_actor_in(
            connection,
            auth,
            actor,
            conversation_id,
            ConversationAction::Read,
        )
        .await
        .map_err(map_authorization_error)
}

pub(super) fn map_authorization_error(error: AuthorizationError) -> ReplayError {
    match error {
        AuthorizationError::Database(error) => ReplayError::Database(error),
        AuthorizationError::Unavailable => ReplayError::Unavailable,
        AuthorizationError::Authentication(error) => map_auth_error(error),
    }
}

pub(super) fn map_auth_error(error: crate::auth::authority::AuthError) -> ReplayError {
    match error {
        crate::auth::authority::AuthError::Database(error) => ReplayError::Database(error),
        crate::auth::authority::AuthError::VerificationBusy
        | crate::auth::authority::AuthError::HashWorker(_) => ReplayError::DependencyUnavailable,
        crate::auth::authority::AuthError::Invalid
        | crate::auth::authority::AuthError::Expired
        | crate::auth::authority::AuthError::Revoked
        | crate::auth::authority::AuthError::Disabled
        | crate::auth::authority::AuthError::Token(_) => {
            ReplayError::ResyncRequired(ResyncReason::CredentialChanged)
        }
    }
}
