use super::{ChatEngine, ChatEvent};

pub(super) async fn list_roles(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id,): (String,),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine.list_roles(session_id, &server_id).await {
            Ok((version, roles, member_roles)) => {
                if let Some(session) = engine.get_session(session_id) {
                    let _ = session.send(ChatEvent::RoleList {
                        server_id,
                        version,
                        roles,
                        member_roles: Some(member_roles),
                    });
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    })
}

pub(super) async fn create_role(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, name, color, permissions): (String, String, Option<String>, Option<i64>),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        let perms = permissions.unwrap_or(0);
        // Prevent non-owners from setting ADMINISTRATOR bit
        let requested = crate::engine::permissions::Permissions::from_bits_truncate(perms as u64);
        if requested.contains(crate::engine::permissions::Permissions::ADMINISTRATOR)
            && !engine.is_server_owner(
                &server_id,
                &engine
                    .get_session(session_id)
                    .and_then(|s| s.user_id.clone())
                    .unwrap_or_default(),
            )
        {
            Err("Only the server owner can grant ADMINISTRATOR permission".into())
        } else {
            match engine
                .require_permission(
                    session_id,
                    &server_id,
                    None,
                    crate::engine::permissions::Permissions::MANAGE_ROLES,
                )
                .await
            {
                Ok(_) => match engine
                    .create_role(session_id, &server_id, &name, color.as_deref(), perms)
                    .await
                {
                    Ok(_) => {
                        engine
                            .broadcast_role_snapshot(session_id, &server_id, None)
                            .await
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
    })
}

pub(super) async fn update_role(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, role_id, name, color, permissions): (String, String, String, Option<String>, i64),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        // Prevent non-owners from setting ADMINISTRATOR bit
        let requested =
            crate::engine::permissions::Permissions::from_bits_truncate(permissions as u64);
        if requested.contains(crate::engine::permissions::Permissions::ADMINISTRATOR)
            && !engine.is_server_owner(
                &server_id,
                &engine
                    .get_session(session_id)
                    .and_then(|s| s.user_id.clone())
                    .unwrap_or_default(),
            )
        {
            Err("Only the server owner can grant ADMINISTRATOR permission".into())
        } else {
            match engine
                .require_permission(
                    session_id,
                    &server_id,
                    None,
                    crate::engine::permissions::Permissions::MANAGE_ROLES,
                )
                .await
            {
                Ok(actor_uid) => {
                    // Role hierarchy: can't edit roles at or above your own
                    match engine
                        .check_role_hierarchy(&server_id, &actor_uid, &role_id)
                        .await
                    {
                        Err(e) => Err(e),
                        Ok(()) => match engine
                            .update_role(
                                session_id,
                                &server_id,
                                &role_id,
                                &name,
                                color.as_deref(),
                                permissions,
                            )
                            .await
                        {
                            Ok(_) => {
                                engine
                                    .broadcast_role_snapshot(session_id, &server_id, None)
                                    .await
                            }
                            Err(e) => Err(e),
                        },
                    }
                }
                Err(e) => Err(e),
            }
        }
    })
}

pub(super) async fn delete_role(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, role_id): (String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine
            .require_permission(
                session_id,
                &server_id,
                None,
                crate::engine::permissions::Permissions::MANAGE_ROLES,
            )
            .await
        {
            Ok(actor_uid) => {
                // Role hierarchy: can't delete roles at or above your own
                match engine
                    .check_role_hierarchy(&server_id, &actor_uid, &role_id)
                    .await
                {
                    Err(e) => Err(e),
                    Ok(()) => match engine.delete_role(session_id, &server_id, &role_id).await {
                        Ok(()) => {
                            engine
                                .broadcast_role_snapshot(session_id, &server_id, None)
                                .await
                        }
                        Err(e) => Err(e),
                    },
                }
            }
            Err(e) => Err(e),
        }
    })
}

pub(super) async fn assign_role(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, user_id, role_id): (String, String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine
            .require_permission(
                session_id,
                &server_id,
                None,
                crate::engine::permissions::Permissions::MANAGE_ROLES,
            )
            .await
        {
            Ok(_actor_uid) => match engine
                .assign_role(session_id, &server_id, &user_id, &role_id)
                .await
            {
                Ok(_) => {
                    engine
                        .broadcast_role_snapshot(session_id, &server_id, Some(&user_id))
                        .await
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        }
    })
}

pub(super) async fn remove_role(
    engine: &ChatEngine,
    session_id: crate::engine::events::ConnectionId,
    (server_id, user_id, role_id): (String, String, String),
) -> std::ops::ControlFlow<(), Result<(), String>> {
    std::ops::ControlFlow::Continue({
        match engine
            .require_permission(
                session_id,
                &server_id,
                None,
                crate::engine::permissions::Permissions::MANAGE_ROLES,
            )
            .await
        {
            Ok(_actor_uid) => match engine
                .remove_role(session_id, &server_id, &user_id, &role_id)
                .await
            {
                Ok(_) => {
                    engine
                        .broadcast_role_snapshot(session_id, &server_id, Some(&user_id))
                        .await
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        }
    })
}
