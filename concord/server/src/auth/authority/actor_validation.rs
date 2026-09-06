use super::{
    Actor, AuthError, AuthService, CredentialId, SqliteConnection, compare_actor,
    load_actor_from_connection,
};

impl AuthService {
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

    pub(super) fn actor_matches(actor: &Actor, current: &Actor) -> bool {
        if current.user_id != actor.user_id
            || current.kind != actor.kind
            || current.scopes != actor.scopes
            || current.credential_version != actor.credential_version
        {
            return false;
        }
        true
    }
}
