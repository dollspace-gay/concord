use super::{ChatEngine, ChatEvent};

pub(super) async fn get_user_profile(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (user_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        let Some(actor) = engine.get_authenticated_actor(session_id) else {
            return std::ops::ControlFlow::Break(());
        };
        match engine.get_user_profile(&actor, &user_id).await {
            Ok((profile, stamp)) => {
                let current = match &stamp {
                    Some(stamp) => engine.authorization_stamp_is_current(&actor, stamp).await,
                    None => engine.actor_is_current(&actor).await,
                };
                if !current {
                    return std::ops::ControlFlow::Break(());
                }
                if let Some(session) = engine.get_session(session_id) {
                    let guard = match stamp {
                        Some(stamp) => {
                            crate::engine::user_session::DeliveryGuard::Stamps(vec![stamp])
                        }
                        None => crate::engine::user_session::DeliveryGuard::ActorCurrent,
                    };
                    let _ = session.send_guarded(ChatEvent::UserProfile { profile }, Some(guard));
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    })
}
