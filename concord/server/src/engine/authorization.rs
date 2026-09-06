use std::fmt;

use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection, SqlitePool};

use crate::auth::authority::{Actor, AuthService, CredentialKind};
use crate::db::models::ChannelRow;
use crate::db::models::MessageRow;
use crate::db::models::ServerMemberRow;
use crate::engine::permissions::{
    ChannelOverride, OverrideTargetType, Permissions, ServerRole, compute_effective_permissions,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelAction {
    View,
    ReadHistory,
    Send,
    Manage,
    ManageMessages,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationAction {
    View,
    Read,
    Send,
    ManageMessages,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationStamp {
    pub server_id: String,
    pub server_version: i64,
    pub channel_versions: Vec<(String, i64)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::pool::{create_pool, run_migrations};
    use crate::engine::permissions::DEFAULT_EVERYONE;

    async fn fixture() -> (SqlitePool, AuthorizationService) {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        for (id, name) in [
            ("owner", "owner"),
            ("member", "member"),
            ("outsider", "outsider"),
        ] {
            sqlx::query("INSERT INTO users(id,username) VALUES(?,?)")
                .bind(id)
                .bind(name)
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO servers(id,name,owner_id) VALUES('server','Server','owner')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO server_members(server_id,user_id,role) VALUES('server','owner','owner'),('server','member','member')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO roles(id,server_id,name,permissions,is_default) VALUES('everyone','server','@everyone',?,1)")
            .bind(DEFAULT_EVERYONE.bits() as i64).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name) VALUES('public','server','#public')")
            .execute(&pool)
            .await
            .unwrap();
        (pool.clone(), AuthorizationService::new(pool))
    }

    #[tokio::test]
    async fn nonmember_and_active_ban_are_denied_before_default_role() {
        let (pool, service) = fixture().await;
        assert!(matches!(
            service
                .authorize_channel("outsider", "public", ChannelAction::View)
                .await,
            Err(AuthorizationError::Unavailable)
        ));
        sqlx::query("INSERT INTO bans(id,server_id,user_id,banned_by) VALUES('ban','server','member','owner')")
            .execute(&pool).await.unwrap();
        assert!(matches!(
            service
                .authorize_channel("member", "public", ChannelAction::View)
                .await,
            Err(AuthorizationError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn history_allow_cannot_bypass_view_deny() {
        let (pool, service) = fixture().await;
        sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,allow_bits,deny_bits) VALUES('deny','public','role','everyone',?,?)")
            .bind(Permissions::READ_MESSAGE_HISTORY.bits() as i64)
            .bind(Permissions::VIEW_CHANNELS.bits() as i64)
            .execute(&pool).await.unwrap();
        assert!(matches!(
            service
                .authorize_channel("member", "public", ChannelAction::ReadHistory)
                .await,
            Err(AuthorizationError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn channel_subscription_is_not_a_private_visibility_grant() {
        let (pool, service) = fixture().await;
        sqlx::query("INSERT INTO channels(id,server_id,name,is_private) VALUES('private','server','#private',1)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channel_members(channel_id,user_id) VALUES('private','member')")
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            service
                .authorize_channel("member", "private", ChannelAction::View)
                .await,
            Err(AuthorizationError::Unavailable)
        ));
        sqlx::query("INSERT INTO channel_visibility_grants(channel_id,target_type,target_id) VALUES('private','user','member')")
            .execute(&pool).await.unwrap();
        service
            .authorize_channel("member", "private", ChannelAction::View)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn public_thread_cannot_exceed_parent_visibility() {
        let (pool, service) = fixture().await;
        sqlx::query("INSERT INTO channels(id,server_id,name,channel_type,parent_channel_id) VALUES('thread','server','#thread','public_thread','public')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,deny_bits) VALUES('parent-deny','public','role','everyone',?)")
            .bind(Permissions::VIEW_CHANNELS.bits() as i64).execute(&pool).await.unwrap();
        assert!(matches!(
            service
                .authorize_channel("member", "thread", ChannelAction::View)
                .await,
            Err(AuthorizationError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn thread_cannot_exceed_parent_repair_guard() {
        let (pool, service) = fixture().await;
        sqlx::query("UPDATE channels SET visibility_repair_required=1 WHERE id='public'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channels(id,server_id,name,channel_type,parent_channel_id) VALUES('thread','server','#thread','public_thread','public')")
            .execute(&pool).await.unwrap();
        assert!(matches!(
            service
                .authorize_channel("member", "thread", ChannelAction::View)
                .await,
            Err(AuthorizationError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn visibility_mutations_advance_channel_and_server_versions() {
        let (pool, _) = fixture().await;
        sqlx::query("INSERT INTO channels(id,server_id,name,channel_type,parent_channel_id) VALUES('thread','server','#thread','private_thread','public')")
            .execute(&pool).await.unwrap();

        let versions = async || {
            (
                sqlx::query_scalar::<_, i64>(
                    "SELECT authorization_version FROM channels WHERE id='thread'",
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
                sqlx::query_scalar::<_, i64>(
                    "SELECT authorization_version FROM servers WHERE id='server'",
                )
                .fetch_one(&pool)
                .await
                .unwrap(),
            )
        };

        let mut previous = versions().await;
        for statement in [
            "INSERT INTO channel_visibility_grants(channel_id,target_type,target_id) VALUES('thread','user','member')",
            "UPDATE channel_visibility_grants SET target_id='owner' WHERE channel_id='thread'",
            "DELETE FROM channel_visibility_grants WHERE channel_id='thread'",
            "INSERT INTO thread_members(thread_id,user_id) VALUES('thread','member')",
            "UPDATE thread_members SET user_id='owner' WHERE thread_id='thread'",
            "DELETE FROM thread_members WHERE thread_id='thread'",
            "UPDATE channels SET visibility_repair_required=1 WHERE id='thread'",
            "UPDATE channels SET is_private=1 WHERE id='thread'",
            "UPDATE channels SET channel_type='public_thread' WHERE id='thread'",
            "UPDATE channels SET parent_channel_id=NULL WHERE id='thread'",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
            let current = versions().await;
            assert!(
                current.0 > previous.0,
                "channel version unchanged: {statement}"
            );
            assert!(
                current.1 > previous.1,
                "server version unchanged: {statement}"
            );
            previous = current;
        }

        let server_before: i64 =
            sqlx::query_scalar("SELECT authorization_version FROM servers WHERE id='server'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query("UPDATE servers SET owner_id='member' WHERE id='server'")
            .execute(&pool)
            .await
            .unwrap();
        let server_after: i64 =
            sqlx::query_scalar("SELECT authorization_version FROM servers WHERE id='server'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(server_after > server_before);
    }

    #[tokio::test]
    async fn sql_failure_is_not_replaced_with_default_permissions() {
        let (pool, service) = fixture().await;
        sqlx::query("DROP TABLE roles")
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            service
                .authorize_channel("member", "public", ChannelAction::View)
                .await,
            Err(AuthorizationError::Database(_))
        ));
    }

    #[tokio::test]
    async fn search_excludes_channels_without_history_permission() {
        let (pool, service) = fixture().await;
        sqlx::query("INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content) VALUES('legacy-id','server','public','owner','owner','classified needle')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO channel_permission_overrides(id,channel_id,target_type,target_id,deny_bits) VALUES('deny-history','public','role','everyone',?)")
            .bind(Permissions::READ_MESSAGE_HISTORY.bits() as i64)
            .execute(&pool).await.unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("member").await.unwrap();

        let (rows, total, stamp) = service
            .search_messages(
                &auth,
                &actor,
                MessageSearch {
                    server_id: "server",
                    query: Some("needle"),
                    requested_channel_id: None,
                    sender: None,
                    has_attachment: false,
                    has_link: false,
                    before: None,
                    after: None,
                    after_inclusive: false,
                    limit: 50,
                    offset: 0,
                    cursor_created_at: None,
                    cursor_message_id: None,
                },
            )
            .await
            .unwrap();
        assert!(rows.is_empty());
        assert_eq!(total, 0);
        assert!(service.stamp_is_current(&stamp).await.unwrap());
        assert!(matches!(
            service
                .search_messages(
                    &auth,
                    &actor,
                    MessageSearch {
                        server_id: "server",
                        query: Some("needle"),
                        requested_channel_id: Some("public"),
                        sender: None,
                        has_attachment: false,
                        has_link: false,
                        before: None,
                        after: None,
                        after_inclusive: false,
                        limit: 50,
                        offset: 0,
                        cursor_created_at: None,
                        cursor_message_id: None,
                    },
                )
                .await,
            Err(AuthorizationError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn bot_authority_intersects_credential_installation_and_exact_webhook_scope() {
        let (pool, service) = fixture().await;
        sqlx::query("INSERT INTO users(id,username,is_bot) VALUES('bot','bot',1)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO server_members(server_id,user_id,role) VALUES('server','bot','member')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = AuthService::new(pool.clone(), "bot-secret".into(), 1);
        let bot_id = crate::auth::authority::UserId::from_stored("bot").unwrap();

        sqlx::query("INSERT INTO bot_installations(id,bot_user_id,server_id,installed_by,granted_scopes,state) VALUES('install','bot','server','owner','messages','active')")
            .execute(&pool).await.unwrap();
        let transport_only = auth
            .issue_bot_token(&bot_id, "transport", "bot")
            .await
            .unwrap();
        let actor = auth.authenticate_bot(&transport_only.secret).await.unwrap();
        assert!(
            service
                .authorize_actor(&auth, &actor, "public", ChannelAction::Send)
                .await
                .is_err()
        );

        let usable = auth
            .issue_bot_token(&bot_id, "usable", "bot messages")
            .await
            .unwrap();
        let actor = auth.authenticate_bot(&usable.secret).await.unwrap();
        service
            .authorize_actor(&auth, &actor, "public", ChannelAction::Send)
            .await
            .unwrap();
        sqlx::query("UPDATE bot_installations SET state='revoked',revoked_at=datetime('now'),authorization_version=authorization_version+1 WHERE id='install'")
            .execute(&pool).await.unwrap();
        assert!(
            service
                .authorize_actor(&auth, &actor, "public", ChannelAction::Send)
                .await
                .is_err()
        );

        sqlx::query("UPDATE bot_installations SET state='active',revoked_at=NULL,granted_scopes='webhook:channel:public',authorization_version=authorization_version+1 WHERE id='install'")
            .execute(&pool).await.unwrap();
        let exact = auth
            .issue_bot_token(&bot_id, "webhook", "bot webhook:channel:public")
            .await
            .unwrap();
        let exact = auth.authenticate_bot(&exact.secret).await.unwrap();
        service
            .authorize_actor(&auth, &exact, "public", ChannelAction::Send)
            .await
            .unwrap();
        assert!(
            service
                .authorize_actor(&auth, &exact, "public", ChannelAction::View)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn bot_grant_shrink_invalidates_a_held_authorization_stamp() {
        let (pool, service) = fixture().await;
        sqlx::query("INSERT INTO users(id,username,is_bot) VALUES('bot','bot',1)")
            .execute(&pool)
            .await
            .unwrap();
        crate::db::queries::bots::add_bot_to_server_with_grants(
            &pool,
            "server",
            "bot",
            "owner",
            "messages commands",
        )
        .await
        .unwrap();

        // Model a response that passed authorization and is waiting at the
        // transport boundary while its bot installation grant is reduced.
        let mut connection = pool.acquire().await.unwrap();
        let held = service
            .authorization_stamp(&mut connection, "server", &["public".to_owned()])
            .await
            .unwrap();
        drop(connection);
        assert!(service.stamp_is_current(&held).await.unwrap());

        crate::db::queries::bots::add_bot_to_server_with_grants(
            &pool, "server", "bot", "owner", "messages",
        )
        .await
        .unwrap();

        assert!(!service.stamp_is_current(&held).await.unwrap());
    }

    #[tokio::test]
    async fn typed_search_filters_share_the_authorized_count_and_page_predicate() {
        let (pool, service) = fixture().await;
        sqlx::query(
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) VALUES \
             ('old','server','public','owner','Alice','https://old.example needle','2026-09-01T12:00:00Z'), \
             ('match','server','public','owner','Alice','https://example.test needle','2026-09-03T12:00:00Z'), \
             ('other','server','public','member','Member','https://example.test needle','2026-09-03T13:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO attachments(id,uploader_id,message_id,filename,original_filename,content_type,file_size) \
             VALUES('attachment','owner','match','proof.txt','proof.txt','text/plain',5)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let actor = auth.issue_web_session("member").await.unwrap().1;
        let (rows, total, _) = service
            .search_messages(
                &auth,
                &actor,
                MessageSearch {
                    server_id: "server",
                    query: Some("needle"),
                    requested_channel_id: Some("public"),
                    sender: Some("alice"),
                    has_attachment: true,
                    has_link: true,
                    before: Some("2026-09-04T00:00:00Z"),
                    after: Some("2026-09-02T23:59:59Z"),
                    after_inclusive: false,
                    limit: 1,
                    offset: 0,
                    cursor_created_at: None,
                    cursor_message_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "match");
    }

    #[tokio::test]
    async fn typed_search_supports_filter_only_paging_and_tracks_edits_and_deletes() {
        let (pool, service) = fixture().await;
        sqlx::query(
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) VALUES \
             ('first','server','public','owner','Alice','original phrase','2026-09-01T12:00:00Z'), \
             ('second','server','public','owner','Alice','second phrase','2026-09-02T12:00:00Z'), \
             ('third','server','public','owner','Alice','third phrase','2026-09-03T12:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let actor = auth.issue_web_session("member").await.unwrap().1;

        let (page, total, _) = service
            .search_messages(
                &auth,
                &actor,
                MessageSearch {
                    server_id: "server",
                    query: None,
                    requested_channel_id: Some("public"),
                    sender: Some("alice"),
                    has_attachment: false,
                    has_link: false,
                    before: None,
                    after: None,
                    after_inclusive: false,
                    limit: 1,
                    offset: 1,
                    cursor_created_at: None,
                    cursor_message_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(
            page.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["second"]
        );

        sqlx::query("UPDATE messages SET content='revised phrase' WHERE id='first'")
            .execute(&pool)
            .await
            .unwrap();
        for (query, expected) in [("original phrase", 0), ("revised phrase", 1)] {
            let (_, total, _) = service
                .search_messages(
                    &auth,
                    &actor,
                    MessageSearch {
                        server_id: "server",
                        query: Some(query),
                        requested_channel_id: None,
                        sender: None,
                        has_attachment: false,
                        has_link: false,
                        before: None,
                        after: None,
                        after_inclusive: false,
                        limit: 50,
                        offset: 0,
                        cursor_created_at: None,
                        cursor_message_id: None,
                    },
                )
                .await
                .unwrap();
            assert_eq!(total, expected, "unexpected count for {query:?}");
        }

        sqlx::query("UPDATE messages SET deleted_at='2026-09-04T00:00:00Z' WHERE id='first'")
            .execute(&pool)
            .await
            .unwrap();
        let (rows, total, _) = service
            .search_messages(
                &auth,
                &actor,
                MessageSearch {
                    server_id: "server",
                    query: Some("revised phrase"),
                    requested_channel_id: None,
                    sender: None,
                    has_attachment: false,
                    has_link: false,
                    before: None,
                    after: None,
                    after_inclusive: false,
                    limit: 50,
                    offset: 0,
                    cursor_created_at: None,
                    cursor_message_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn search_keyset_remains_stable_across_concurrent_insert_and_delete() {
        let (pool, service) = fixture().await;
        sqlx::query(
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) VALUES \
             ('a','server','public','owner','Alice','needle','2026-09-01T00:00:00Z'), \
             ('b','server','public','owner','Alice','needle','2026-09-01T20:00:00Z'), \
             ('c','server','public','owner','Alice','needle','2026-09-01T22:00:00-02:00'), \
             ('d','server','public','owner','Alice','needle','2026-09-02 00:00:00')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let actor = auth.issue_web_session("member").await.unwrap().1;
        let (first, total, _) = service
            .search_messages(
                &auth,
                &actor,
                MessageSearch {
                    server_id: "server",
                    query: Some("needle"),
                    requested_channel_id: None,
                    sender: None,
                    has_attachment: false,
                    has_link: false,
                    before: None,
                    after: None,
                    after_inclusive: false,
                    limit: 2,
                    offset: 0,
                    cursor_created_at: None,
                    cursor_message_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(total, 4);
        assert_eq!(
            first.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["d", "c"]
        );

        sqlx::query(
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) \
             VALUES('e','server','public','owner','Alice','needle','2026-09-05T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE messages SET deleted_at='2026-09-06T00:00:00Z' WHERE id='c'")
            .execute(&pool)
            .await
            .unwrap();
        let (second, total, _) = service
            .search_messages(
                &auth,
                &actor,
                MessageSearch {
                    server_id: "server",
                    query: Some("needle"),
                    requested_channel_id: None,
                    sender: None,
                    has_attachment: false,
                    has_link: false,
                    before: None,
                    after: None,
                    after_inclusive: false,
                    limit: 2,
                    offset: 0,
                    cursor_created_at: Some(&first[1].created_at),
                    cursor_message_id: Some(&first[1].id),
                },
            )
            .await
            .unwrap();
        assert_eq!(total, 4);
        assert_eq!(
            second.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
    }

    #[tokio::test]
    async fn search_authorized_channel_set_exceeds_sqlite_variable_limit() {
        let (pool, service) = fixture().await;
        for index in 0..1_005 {
            let channel = format!("many-{index}");
            sqlx::query("INSERT INTO channels(id,server_id,name) VALUES(?,'server',?)")
                .bind(&channel)
                .bind(format!("#many-{index}"))
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) \
                 VALUES(?,'server',?,'owner','Alice','needle',?)",
            )
            .bind(format!("message-{index}"))
            .bind(&channel)
            .bind(format!("2026-09-01T00:{:02}:{:02}Z", (index / 60) % 60, index % 60))
            .execute(&pool)
            .await
            .unwrap();
        }
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let actor = auth.issue_web_session("member").await.unwrap().1;
        let (rows, total, _) = service
            .search_messages(
                &auth,
                &actor,
                MessageSearch {
                    server_id: "server",
                    query: Some("needle"),
                    requested_channel_id: None,
                    sender: None,
                    has_attachment: false,
                    has_link: false,
                    before: None,
                    after: None,
                    after_inclusive: false,
                    limit: 50,
                    offset: 0,
                    cursor_created_at: None,
                    cursor_message_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(total, 1_005);
        assert_eq!(rows.len(), 50);
        assert!(rows.windows(2).all(|pair| {
            (pair[0].created_at.as_str(), pair[0].id.as_str())
                > (pair[1].created_at.as_str(), pair[1].id.as_str())
        }));
    }

    #[tokio::test]
    async fn date_only_after_excludes_the_named_utc_day_and_includes_next_midnight() {
        let (pool, service) = fixture().await;
        sqlx::query(
            "INSERT INTO messages(id,server_id,channel_id,sender_id,sender_nick,content,created_at) VALUES \
             ('near-midnight','server','public','owner','Alice','near','2026-09-01T23:59:59.999Z'), \
             ('midnight','server','public','owner','Alice','midnight','2026-09-02T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let actor = auth.issue_web_session("member").await.unwrap().1;
        let (rows, total, _) = service
            .search_messages(
                &auth,
                &actor,
                MessageSearch {
                    server_id: "server",
                    query: None,
                    requested_channel_id: None,
                    sender: None,
                    has_attachment: false,
                    has_link: false,
                    before: None,
                    after: Some("2026-09-02T00:00:00Z"),
                    after_inclusive: true,
                    limit: 50,
                    offset: 0,
                    cursor_created_at: None,
                    cursor_message_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(rows[0].id, "midnight");
    }

    #[tokio::test]
    async fn direct_conversation_send_enforces_participants_blocks_and_preferences() {
        let (pool, service) = fixture().await;
        sqlx::query("INSERT INTO conversations(id,kind) VALUES('dm','direct')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO conversation_participants(conversation_id,user_id) VALUES('dm','owner'),('dm','member')")
            .execute(&pool).await.unwrap();
        let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
        let (_, actor) = auth.issue_web_session("member").await.unwrap();

        let authorize = async || {
            let mut transaction = pool.begin().await.unwrap();
            service
                .authorize_conversation_actor_in(
                    &mut transaction,
                    &auth,
                    &actor,
                    "dm",
                    ConversationAction::Send,
                )
                .await
        };
        authorize().await.unwrap();
        sqlx::query(
            "INSERT INTO user_blocks(blocker_user_id,blocked_user_id) VALUES('owner','member')",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(matches!(
            authorize().await,
            Err(AuthorizationError::Unavailable)
        ));
        sqlx::query("DELETE FROM user_blocks")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO direct_message_preferences(user_id,allow_from) VALUES('owner','none')",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(matches!(
            authorize().await,
            Err(AuthorizationError::Unavailable)
        ));
        sqlx::query(
            "UPDATE direct_message_preferences SET allow_from='everyone' WHERE user_id='owner'",
        )
        .execute(&pool)
        .await
        .unwrap();
        authorize().await.unwrap();
    }
}

impl ChannelAction {
    fn permission(self) -> Permissions {
        match self {
            Self::View => Permissions::VIEW_CHANNELS,
            Self::ReadHistory => Permissions::READ_MESSAGE_HISTORY,
            Self::Send => Permissions::SEND_MESSAGES,
            Self::Manage => Permissions::MANAGE_CHANNELS,
            Self::ManageMessages => Permissions::MANAGE_MESSAGES,
        }
    }
}

#[derive(Debug)]
pub enum AuthorizationError {
    Unavailable,
    Database(sqlx::Error),
    Authentication(crate::auth::authority::AuthError),
}

impl fmt::Display for AuthorizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("resource unavailable"),
            Self::Database(_) => formatter.write_str("authorization database operation failed"),
            Self::Authentication(error) => write!(formatter, "authentication failed: {error}"),
        }
    }
}

impl std::error::Error for AuthorizationError {}

impl From<sqlx::Error> for AuthorizationError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

#[derive(Clone)]
pub struct AuthorizationService {
    pool: SqlitePool,
}

pub struct MessageSearch<'a> {
    pub server_id: &'a str,
    pub query: Option<&'a str>,
    pub requested_channel_id: Option<&'a str>,
    pub sender: Option<&'a str>,
    pub has_attachment: bool,
    pub has_link: bool,
    pub before: Option<&'a str>,
    pub after: Option<&'a str>,
    pub after_inclusive: bool,
    pub limit: i64,
    pub offset: i64,
    pub cursor_created_at: Option<&'a str>,
    pub cursor_message_id: Option<&'a str>,
}

struct ServerAuthority {
    owner_id: String,
    role_permissions: Vec<(String, Permissions)>,
    default_role_id: String,
    base_permissions: Permissions,
    privileged: bool,
}

struct ActorScopeRequirement<'a> {
    server_id: &'a str,
    scope: &'a str,
    channel_id: Option<&'a str>,
    allow_exact_channel: bool,
}

impl AuthorizationService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) async fn authorize_bot_installation_scope(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        scope: &str,
    ) -> Result<(), AuthorizationError> {
        if actor.kind() != CredentialKind::BotToken {
            return Err(AuthorizationError::Unavailable);
        }
        let mut transaction = self.pool.begin().await?;
        self.require_actor_scope_in(
            &mut transaction,
            auth,
            actor,
            ActorScopeRequirement {
                server_id,
                scope,
                channel_id: None,
                allow_exact_channel: false,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn require_actor_scope_in(
        &self,
        connection: &mut SqliteConnection,
        auth: &AuthService,
        actor: &Actor,
        requirement: ActorScopeRequirement<'_>,
    ) -> Result<(), AuthorizationError> {
        auth.validate_actor_in(connection, actor)
            .await
            .map_err(AuthorizationError::Authentication)?;
        let transport = match actor.kind() {
            CredentialKind::WebSession => "web",
            CredentialKind::IrcToken => "irc",
            CredentialKind::BotToken => "bot",
        };
        if !actor.scopes().contains(transport) {
            return Err(AuthorizationError::Unavailable);
        }
        if actor.kind() != CredentialKind::BotToken {
            return Ok(());
        }
        let granted: Option<String> = sqlx::query_scalar(
            "SELECT granted_scopes FROM bot_installations WHERE bot_user_id=? AND server_id=? \
             AND state='active' AND revoked_at IS NULL",
        )
        .bind(actor.user_id().as_str())
        .bind(requirement.server_id)
        .fetch_optional(&mut *connection)
        .await?;
        let granted = granted.ok_or(AuthorizationError::Unavailable)?;
        let installation_scopes = crate::auth::authority::CredentialScopes::parse(&granted);
        let exact = requirement
            .channel_id
            .map(|id| format!("webhook:channel:{id}"));
        let credential_allows = actor.scopes().contains(requirement.scope)
            || actor.scopes().contains("*")
            || exact.as_deref().is_some_and(|scope| {
                requirement.allow_exact_channel && actor.scopes().contains(scope)
            });
        let installation_allows = installation_scopes.contains(requirement.scope)
            || installation_scopes.contains("*")
            || exact.as_deref().is_some_and(|scope| {
                requirement.allow_exact_channel && installation_scopes.contains(scope)
            });
        if credential_allows && installation_allows {
            Ok(())
        } else {
            Err(AuthorizationError::Unavailable)
        }
    }

    pub async fn authorize_actor(
        &self,
        auth: &AuthService,
        actor: &Actor,
        channel_id: &str,
        action: ChannelAction,
    ) -> Result<(), AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        self.authorize_actor_in(&mut transaction, auth, actor, channel_id, action)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn authorize_actor_stamped(
        &self,
        auth: &AuthService,
        actor: &Actor,
        channel_id: &str,
        action: ChannelAction,
    ) -> Result<AuthorizationStamp, AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        self.authorize_actor_in(&mut transaction, auth, actor, channel_id, action)
            .await?;
        let channel: ChannelRow = self.load_channel(&mut transaction, channel_id).await?;
        let stamp = self
            .authorization_stamp(&mut transaction, &channel.server_id, &[channel.id])
            .await?;
        transaction.commit().await?;
        Ok(stamp)
    }

    pub(crate) async fn authorize_actor_in(
        &self,
        connection: &mut SqliteConnection,
        auth: &AuthService,
        actor: &Actor,
        resource: &str,
        action: ChannelAction,
    ) -> Result<(), AuthorizationError> {
        let channel: ChannelRow = self.load_channel(connection, resource).await?;
        let scope = if action == ChannelAction::Manage {
            "channels"
        } else {
            "messages"
        };
        self.require_actor_scope_in(
            connection,
            auth,
            actor,
            ActorScopeRequirement {
                server_id: &channel.server_id,
                scope,
                channel_id: Some(resource),
                allow_exact_channel: action == ChannelAction::Send,
            },
        )
        .await?;
        self.authorize_channel_in(connection, actor.user_id().as_str(), resource, action)
            .await
    }

    pub(crate) async fn require_server_actor_in(
        &self,
        connection: &mut SqliteConnection,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        required: Permissions,
    ) -> Result<(), AuthorizationError> {
        let permissions = self
            .server_actor_permissions_in(connection, auth, actor, server_id)
            .await?;
        if permissions.contains(required) {
            Ok(())
        } else {
            Err(AuthorizationError::Unavailable)
        }
    }

    pub(crate) async fn require_channel_actor_permission_in(
        &self,
        connection: &mut SqliteConnection,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        channel_id: &str,
        required: Permissions,
    ) -> Result<(), AuthorizationError> {
        let scope = if required.intersects(Permissions::MANAGE_CHANNELS) {
            "channels"
        } else {
            "messages"
        };
        self.require_actor_scope_in(
            connection,
            auth,
            actor,
            ActorScopeRequirement {
                server_id,
                scope,
                channel_id: Some(channel_id),
                allow_exact_channel: required == Permissions::SEND_MESSAGES,
            },
        )
        .await?;
        let authority = self
            .server_authority(connection, actor.user_id().as_str(), server_id)
            .await?;
        let channel = self.load_channel(connection, channel_id).await?;
        if channel.server_id != server_id {
            return Err(AuthorizationError::Unavailable);
        }
        let permissions = self
            .channel_permissions(connection, actor.user_id().as_str(), &channel, &authority)
            .await?;
        if permissions.contains(required) {
            Ok(())
        } else {
            Err(AuthorizationError::Unavailable)
        }
    }

    pub(crate) async fn server_actor_permissions_in(
        &self,
        connection: &mut SqliteConnection,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
    ) -> Result<Permissions, AuthorizationError> {
        self.require_actor_scope_in(
            connection,
            auth,
            actor,
            ActorScopeRequirement {
                server_id,
                scope: "server",
                channel_id: None,
                allow_exact_channel: false,
            },
        )
        .await?;
        let authority = self
            .server_authority(connection, actor.user_id().as_str(), server_id)
            .await?;
        let permissions = if authority.privileged {
            Permissions::all()
        } else {
            compute_effective_permissions(
                authority.base_permissions,
                &authority.role_permissions,
                &[],
                &authority.default_role_id,
                actor.user_id().as_str(),
                authority.owner_id == actor.user_id().as_str(),
            )
        };
        Ok(permissions)
    }

    pub async fn authorize_conversation_actor_in(
        &self,
        connection: &mut SqliteConnection,
        auth: &AuthService,
        actor: &Actor,
        conversation_id: &str,
        action: ConversationAction,
    ) -> Result<(), AuthorizationError> {
        let conversation = sqlx::query("SELECT kind,channel_id FROM conversations WHERE id=?")
            .bind(conversation_id)
            .fetch_optional(&mut *connection)
            .await?
            .ok_or(AuthorizationError::Unavailable)?;
        if conversation.get::<String, _>(0) == "channel" {
            let channel_id: String = conversation
                .get::<Option<String>, _>(1)
                .ok_or(AuthorizationError::Unavailable)?;
            let channel_action = match action {
                ConversationAction::View => ChannelAction::View,
                ConversationAction::Read => ChannelAction::ReadHistory,
                ConversationAction::Send => ChannelAction::Send,
                ConversationAction::ManageMessages => ChannelAction::ManageMessages,
            };
            let channel = self.load_channel(connection, &channel_id).await?;
            self.require_actor_scope_in(
                connection,
                auth,
                actor,
                ActorScopeRequirement {
                    server_id: &channel.server_id,
                    scope: "messages",
                    channel_id: Some(&channel_id),
                    allow_exact_channel: action == ConversationAction::Send,
                },
            )
            .await?;
            return self
                .authorize_channel_in(
                    connection,
                    actor.user_id().as_str(),
                    &channel_id,
                    channel_action,
                )
                .await;
        }

        auth.validate_actor_in(connection, actor)
            .await
            .map_err(AuthorizationError::Authentication)?;
        let transport = match actor.kind() {
            CredentialKind::WebSession => "web",
            CredentialKind::IrcToken => "irc",
            CredentialKind::BotToken => return Err(AuthorizationError::Unavailable),
        };
        if !actor.scopes().contains(transport) {
            return Err(AuthorizationError::Unavailable);
        }

        let user_id = actor.user_id().as_str();
        let participant: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM conversation_participants cp JOIN users u ON u.id=cp.user_id WHERE cp.conversation_id=? AND cp.user_id=? AND cp.left_at IS NULL AND u.disabled_at IS NULL)",
        )
        .bind(conversation_id)
        .bind(user_id)
        .fetch_one(&mut *connection)
        .await?;
        if !participant || action == ConversationAction::ManageMessages {
            return Err(AuthorizationError::Unavailable);
        }
        if action != ConversationAction::Send {
            return Ok(());
        }
        let recipient: String = sqlx::query_scalar(
            "SELECT cp.user_id FROM conversation_participants cp JOIN users u ON u.id=cp.user_id \
             WHERE cp.conversation_id=? AND cp.user_id<>? AND cp.left_at IS NULL AND u.disabled_at IS NULL \
             ORDER BY cp.user_id LIMIT 1",
        )
        .bind(conversation_id)
        .bind(user_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(AuthorizationError::Unavailable)?;
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM user_blocks WHERE (blocker_user_id=? AND blocked_user_id=?) OR (blocker_user_id=? AND blocked_user_id=?))",
        )
        .bind(user_id)
        .bind(&recipient)
        .bind(&recipient)
        .bind(user_id)
        .fetch_one(&mut *connection)
        .await?;
        if blocked {
            return Err(AuthorizationError::Unavailable);
        }
        let preference: Option<String> =
            sqlx::query_scalar("SELECT allow_from FROM direct_message_preferences WHERE user_id=?")
                .bind(&recipient)
                .fetch_optional(&mut *connection)
                .await?;
        match preference.as_deref().unwrap_or("shared_server") {
            "everyone" => Ok(()),
            "shared_server" => {
                let shared: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM server_members sender JOIN server_members recipient ON recipient.server_id=sender.server_id WHERE sender.user_id=? AND recipient.user_id=? AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=sender.server_id AND b.user_id IN (?,?)))",
                )
                .bind(user_id)
                .bind(&recipient)
                .bind(user_id)
                .bind(&recipient)
                .fetch_one(connection)
                .await?;
                shared.then_some(()).ok_or(AuthorizationError::Unavailable)
            }
            _ => Err(AuthorizationError::Unavailable),
        }
    }

    pub async fn authorize_channel(
        &self,
        user_id: &str,
        channel_id: &str,
        action: ChannelAction,
    ) -> Result<(), AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        self.authorize_channel_in(&mut transaction, user_id, channel_id, action)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn effective_permissions(
        &self,
        user_id: &str,
        server_id: &str,
        channel_id: Option<&str>,
    ) -> Result<Permissions, AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        let authority = self
            .server_authority(&mut transaction, user_id, server_id)
            .await?;
        let permissions = if let Some(channel_id) = channel_id {
            let channel = self.load_channel(&mut transaction, channel_id).await?;
            if channel.server_id != server_id {
                return Err(AuthorizationError::Unavailable);
            }
            self.channel_permissions(&mut transaction, user_id, &channel, &authority)
                .await?
        } else if authority.privileged {
            Permissions::all()
        } else {
            compute_effective_permissions(
                authority.base_permissions,
                &authority.role_permissions,
                &[],
                &authority.default_role_id,
                user_id,
                authority.owner_id == user_id,
            )
        };
        transaction.commit().await?;
        Ok(permissions)
    }

    pub async fn visible_channels(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Vec<ChannelRow>, AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        let authority = self
            .server_authority(&mut transaction, user_id, server_id)
            .await?;
        let channels = sqlx::query_as::<_, ChannelRow>(
            "SELECT * FROM channels WHERE server_id=? ORDER BY position,name",
        )
        .bind(server_id)
        .fetch_all(&mut *transaction)
        .await?;
        let mut visible = Vec::new();
        for channel in channels {
            let permissions = self
                .channel_permissions(&mut transaction, user_id, &channel, &authority)
                .await?;
            if permissions.contains(Permissions::VIEW_CHANNELS)
                && self
                    .visibility_granted(&mut transaction, user_id, &channel, &authority)
                    .await?
            {
                visible.push(channel);
            }
        }
        transaction.commit().await?;
        Ok(visible)
    }

    pub async fn visible_channels_for_actor(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
    ) -> Result<(Vec<ChannelRow>, AuthorizationStamp), AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        self.require_actor_scope_in(
            &mut transaction,
            auth,
            actor,
            ActorScopeRequirement {
                server_id,
                scope: "messages",
                channel_id: None,
                allow_exact_channel: false,
            },
        )
        .await?;
        let user_id = actor.user_id().as_str();
        let authority = self
            .server_authority(&mut transaction, user_id, server_id)
            .await?;
        let channels = sqlx::query_as::<_, ChannelRow>(
            "SELECT * FROM channels WHERE server_id=? ORDER BY position,name",
        )
        .bind(server_id)
        .fetch_all(&mut *transaction)
        .await?;
        let mut visible = Vec::new();
        for channel in channels {
            let permissions = self
                .channel_permissions(&mut transaction, user_id, &channel, &authority)
                .await?;
            if permissions.contains(Permissions::VIEW_CHANNELS)
                && self
                    .visibility_granted(&mut transaction, user_id, &channel, &authority)
                    .await?
            {
                visible.push(channel);
            }
        }
        let ids = visible
            .iter()
            .map(|channel| channel.id.clone())
            .collect::<Vec<_>>();
        let stamp = self
            .authorization_stamp(&mut transaction, server_id, &ids)
            .await?;
        transaction.commit().await?;
        Ok((visible, stamp))
    }

    pub async fn server_members_for_actor(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
    ) -> Result<(Vec<ServerMemberRow>, AuthorizationStamp), AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        self.require_actor_scope_in(
            &mut transaction,
            auth,
            actor,
            ActorScopeRequirement {
                server_id,
                scope: "server",
                channel_id: None,
                allow_exact_channel: false,
            },
        )
        .await?;
        self.server_authority(&mut transaction, actor.user_id().as_str(), server_id)
            .await?;
        let rows = sqlx::query_as::<_, ServerMemberRow>(
            "SELECT * FROM server_members WHERE server_id=? ORDER BY joined_at,user_id",
        )
        .bind(server_id)
        .fetch_all(&mut *transaction)
        .await?;
        let stamp = self
            .authorization_stamp(&mut transaction, server_id, &[])
            .await?;
        transaction.commit().await?;
        Ok((rows, stamp))
    }

    pub async fn search_messages(
        &self,
        auth: &AuthService,
        actor: &Actor,
        request: MessageSearch<'_>,
    ) -> Result<(Vec<MessageRow>, i64, AuthorizationStamp), AuthorizationError> {
        let MessageSearch {
            server_id,
            query,
            requested_channel_id,
            sender,
            has_attachment,
            has_link,
            before,
            after,
            after_inclusive,
            limit,
            offset,
            cursor_created_at,
            cursor_message_id,
        } = request;
        let mut transaction = self.pool.begin().await?;
        auth.validate_actor_in(&mut transaction, actor)
            .await
            .map_err(AuthorizationError::Authentication)?;
        if !actor.scopes().contains("web") && !actor.scopes().contains("irc") {
            return Err(AuthorizationError::Unavailable);
        }
        let user_id = actor.user_id().as_str();
        let authority = self
            .server_authority(&mut transaction, user_id, server_id)
            .await?;
        let channels =
            sqlx::query_as::<_, ChannelRow>("SELECT * FROM channels WHERE server_id=? ORDER BY id")
                .bind(server_id)
                .fetch_all(&mut *transaction)
                .await?;
        let mut readable = Vec::new();
        for channel in channels {
            let permissions = self
                .channel_permissions(&mut transaction, user_id, &channel, &authority)
                .await?;
            if permissions.contains(Permissions::VIEW_CHANNELS | Permissions::READ_MESSAGE_HISTORY)
                && self
                    .visibility_granted(&mut transaction, user_id, &channel, &authority)
                    .await?
            {
                readable.push(channel.id);
            }
        }
        if let Some(requested) = requested_channel_id {
            if !readable.iter().any(|id| id == requested) {
                return Err(AuthorizationError::Unavailable);
            }
            readable.retain(|id| id == requested);
        }
        if readable.is_empty() {
            let stamp = self
                .authorization_stamp(&mut transaction, server_id, &readable)
                .await?;
            transaction.commit().await?;
            return Ok((Vec::new(), 0, stamp));
        }

        // Keep the authorized set on this transaction's SQLite connection.
        // Inserts use a fixed two-parameter statement, avoiding an unbounded
        // `IN (?, …)` list while retaining every readable channel for global
        // ordering and counts.
        sqlx::query(
            "CREATE TEMP TABLE IF NOT EXISTS search_readable_channels(\
             channel_id TEXT PRIMARY KEY) WITHOUT ROWID",
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM search_readable_channels")
            .execute(&mut *transaction)
            .await?;
        for channel_id in &readable {
            sqlx::query("INSERT INTO search_readable_channels(channel_id) VALUES(?)")
                .bind(channel_id)
                .execute(&mut *transaction)
                .await?;
        }

        let safe_query = query.map(|query| format!("\"{}\"", query.replace('"', "")));
        let append_predicate = |builder: &mut QueryBuilder<Sqlite>, include_cursor: bool| {
            builder.push(" FROM messages m ");
            if safe_query.is_some() {
                builder.push("JOIN messages_fts f ON m.rowid=f.rowid ");
            }
            builder.push("WHERE ");
            if let Some(safe_query) = &safe_query {
                builder
                    .push("f.content MATCH ")
                    .push_bind(safe_query.clone())
                    .push(" AND ");
            }
            builder
                .push("m.server_id=")
                .push_bind(server_id.to_owned())
                .push(
                    " AND m.deleted_at IS NULL AND m.channel_id IN (\
                     SELECT channel_id FROM search_readable_channels)",
                );
            if let Some(sender) = sender {
                builder
                    .push(" AND (m.sender_id=")
                    .push_bind(sender.to_owned())
                    .push(" OR m.sender_nick COLLATE NOCASE=")
                    .push_bind(sender.to_owned())
                    .push(")");
            }
            if has_attachment {
                builder.push(" AND EXISTS(SELECT 1 FROM attachments a WHERE a.message_id=m.id)");
            }
            if has_link {
                builder.push(" AND (m.content LIKE '%http://%' OR m.content LIKE '%https://%')");
            }
            if let Some(before) = before {
                builder
                    .push(" AND julianday(m.created_at)<julianday(")
                    .push_bind(before.to_owned())
                    .push(")");
            }
            if let Some(after) = after {
                builder
                    .push(if after_inclusive {
                        " AND julianday(m.created_at)>=julianday("
                    } else {
                        " AND julianday(m.created_at)>julianday("
                    })
                    .push_bind(after.to_owned())
                    .push(")");
            }
            if include_cursor
                && let (Some(created_at), Some(message_id)) = (cursor_created_at, cursor_message_id)
            {
                builder
                    .push(" AND (julianday(m.created_at)<julianday(")
                    .push_bind(created_at.to_owned())
                    .push(") OR (julianday(m.created_at)=julianday(")
                    .push_bind(created_at.to_owned())
                    .push(") AND m.id<")
                    .push_bind(message_id.to_owned())
                    .push("))");
            }
        };
        let mut count_builder = QueryBuilder::<Sqlite>::new("SELECT COUNT(*)");
        append_predicate(&mut count_builder, false);
        let total: i64 = count_builder
            .build_query_scalar()
            .fetch_one(&mut *transaction)
            .await?;
        let mut rows_builder = QueryBuilder::<Sqlite>::new(
            "SELECT m.id,m.server_id,m.channel_id,m.sender_id,m.sender_nick,m.content,m.created_at,m.target_user_id,m.edited_at,m.deleted_at,m.reply_to_id",
        );
        append_predicate(&mut rows_builder, true);
        rows_builder
            .push(" ORDER BY julianday(m.created_at) DESC,m.id DESC LIMIT ")
            .push_bind(limit.clamp(1, 50))
            .push(" OFFSET ")
            .push_bind(offset.clamp(0, 10_000));
        let rows = rows_builder
            .build_query_as::<MessageRow>()
            .fetch_all(&mut *transaction)
            .await?;
        let stamp = self
            .authorization_stamp(&mut transaction, server_id, &readable)
            .await?;
        transaction.commit().await?;
        Ok((rows, total, stamp))
    }

    pub(crate) async fn authorization_stamp(
        &self,
        connection: &mut SqliteConnection,
        server_id: &str,
        channel_ids: &[String],
    ) -> Result<AuthorizationStamp, AuthorizationError> {
        let server_version =
            sqlx::query_scalar("SELECT authorization_version FROM servers WHERE id=?")
                .bind(server_id)
                .fetch_optional(&mut *connection)
                .await?
                .ok_or(AuthorizationError::Unavailable)?;
        let mut channel_versions = Vec::with_capacity(channel_ids.len());
        for channel_id in channel_ids {
            let version = sqlx::query_scalar(
                "SELECT authorization_version FROM channels WHERE id=? AND server_id=?",
            )
            .bind(channel_id)
            .bind(server_id)
            .fetch_optional(&mut *connection)
            .await?
            .ok_or(AuthorizationError::Unavailable)?;
            channel_versions.push((channel_id.clone(), version));
        }
        Ok(AuthorizationStamp {
            server_id: server_id.to_owned(),
            server_version,
            channel_versions,
        })
    }

    pub async fn stamp_is_current(
        &self,
        stamp: &AuthorizationStamp,
    ) -> Result<bool, AuthorizationError> {
        let mut transaction = self.pool.begin().await?;
        let server_version: Option<i64> =
            sqlx::query_scalar("SELECT authorization_version FROM servers WHERE id=?")
                .bind(&stamp.server_id)
                .fetch_optional(&mut *transaction)
                .await?;
        if server_version != Some(stamp.server_version) {
            return Ok(false);
        }
        for (channel_id, expected) in &stamp.channel_versions {
            let actual: Option<i64> = sqlx::query_scalar(
                "SELECT authorization_version FROM channels WHERE id=? AND server_id=?",
            )
            .bind(channel_id)
            .bind(&stamp.server_id)
            .fetch_optional(&mut *transaction)
            .await?;
            if actual != Some(*expected) {
                return Ok(false);
            }
        }
        transaction.commit().await?;
        Ok(true)
    }

    pub(crate) async fn authorize_channel_in(
        &self,
        connection: &mut SqliteConnection,
        user_id: &str,
        channel_id: &str,
        action: ChannelAction,
    ) -> Result<(), AuthorizationError> {
        let channel = self.load_channel(connection, channel_id).await?;
        let authority = self
            .server_authority(connection, user_id, &channel.server_id)
            .await?;
        let permissions = self
            .channel_permissions(connection, user_id, &channel, &authority)
            .await?;
        if !permissions.contains(Permissions::VIEW_CHANNELS)
            || !permissions.contains(action.permission())
            || !self
                .visibility_granted(connection, user_id, &channel, &authority)
                .await?
        {
            return Err(AuthorizationError::Unavailable);
        }
        Ok(())
    }

    async fn load_channel(
        &self,
        connection: &mut SqliteConnection,
        channel_id: &str,
    ) -> Result<ChannelRow, AuthorizationError> {
        sqlx::query_as::<_, ChannelRow>("SELECT * FROM channels WHERE id=?")
            .bind(channel_id)
            .fetch_optional(connection)
            .await?
            .ok_or(AuthorizationError::Unavailable)
    }

    async fn server_authority(
        &self,
        connection: &mut SqliteConnection,
        user_id: &str,
        server_id: &str,
    ) -> Result<ServerAuthority, AuthorizationError> {
        let row = sqlx::query(
            "SELECT s.owner_id,sm.role FROM servers s JOIN server_members sm ON sm.server_id=s.id AND sm.user_id=? WHERE s.id=? AND NOT EXISTS(SELECT 1 FROM bans b WHERE b.server_id=s.id AND b.user_id=sm.user_id)",
        )
        .bind(user_id)
        .bind(server_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(AuthorizationError::Unavailable)?;
        let owner_id: String = row.get(0);
        let member_role: String = row.get(1);
        let default = sqlx::query("SELECT id,permissions FROM roles WHERE server_id=? AND is_default=1 ORDER BY id LIMIT 1")
            .bind(server_id).fetch_optional(&mut *connection).await?;
        let (default_role_id, base_permissions) = match default {
            Some(row) => (
                row.get(0),
                Permissions::from_bits_truncate(row.get::<i64, _>(1) as u64),
            ),
            None => (
                String::new(),
                ServerRole::parse(&member_role).to_default_permissions(),
            ),
        };
        let rows = sqlx::query("SELECT r.id,r.permissions FROM roles r JOIN user_roles ur ON ur.role_id=r.id AND ur.server_id=r.server_id WHERE ur.server_id=? AND ur.user_id=?")
            .bind(server_id).bind(user_id).fetch_all(&mut *connection).await?;
        let role_permissions: Vec<(String, Permissions)> = rows
            .into_iter()
            .map(|row| {
                (
                    row.get(0),
                    Permissions::from_bits_truncate(row.get::<i64, _>(1) as u64),
                )
            })
            .collect();
        let privileged = owner_id == user_id
            || matches!(member_role.as_str(), "owner" | "admin")
            || role_permissions
                .iter()
                .any(|(_, permissions)| permissions.contains(Permissions::ADMINISTRATOR));
        Ok(ServerAuthority {
            owner_id,
            role_permissions,
            default_role_id,
            base_permissions,
            privileged,
        })
    }

    async fn channel_permissions(
        &self,
        connection: &mut SqliteConnection,
        user_id: &str,
        channel: &ChannelRow,
        authority: &ServerAuthority,
    ) -> Result<Permissions, AuthorizationError> {
        if authority.privileged {
            return Ok(Permissions::all());
        }
        let rows = sqlx::query("SELECT target_type,target_id,allow_bits,deny_bits FROM channel_permission_overrides WHERE channel_id=?")
            .bind(&channel.id).fetch_all(connection).await?;
        let overrides = rows
            .into_iter()
            .map(|row| ChannelOverride {
                target_type: if row.get::<String, _>(0) == "role" {
                    OverrideTargetType::Role
                } else {
                    OverrideTargetType::User
                },
                target_id: row.get(1),
                allow: Permissions::from_bits_truncate(row.get::<i64, _>(2) as u64),
                deny: Permissions::from_bits_truncate(row.get::<i64, _>(3) as u64),
            })
            .collect::<Vec<_>>();
        Ok(compute_effective_permissions(
            authority.base_permissions,
            &authority.role_permissions,
            &overrides,
            &authority.default_role_id,
            user_id,
            false,
        ))
    }

    async fn visibility_granted(
        &self,
        connection: &mut SqliteConnection,
        user_id: &str,
        channel: &ChannelRow,
        authority: &ServerAuthority,
    ) -> Result<bool, AuthorizationError> {
        if channel.visibility_repair_required != 0 {
            return Ok(false);
        }
        if channel.channel_type == "public_thread" || channel.channel_type == "private_thread" {
            let Some(parent_id) = channel.parent_channel_id.as_deref() else {
                return Ok(false);
            };
            let parent = self.load_channel(connection, parent_id).await?;
            if parent.server_id != channel.server_id || parent.channel_type.ends_with("thread") {
                return Ok(false);
            }
            let parent_permissions = self
                .channel_permissions(connection, user_id, &parent, authority)
                .await?;
            if !parent_permissions.contains(Permissions::VIEW_CHANNELS) {
                return Ok(false);
            }
            if !self
                .visibility_granted_non_thread(connection, user_id, &parent, authority)
                .await?
            {
                return Ok(false);
            }
            if channel.channel_type == "private_thread" && !authority.privileged {
                return sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM thread_members WHERE thread_id=? AND user_id=?)",
                )
                .bind(&channel.id)
                .bind(user_id)
                .fetch_one(connection)
                .await
                .map_err(Into::into);
            }
            return Ok(true);
        }
        self.visibility_granted_non_thread(connection, user_id, channel, authority)
            .await
    }

    async fn visibility_granted_non_thread(
        &self,
        connection: &mut SqliteConnection,
        user_id: &str,
        channel: &ChannelRow,
        authority: &ServerAuthority,
    ) -> Result<bool, AuthorizationError> {
        if channel.visibility_repair_required != 0 {
            return Ok(false);
        }
        if channel.is_private == 0 || authority.privileged {
            return Ok(true);
        }
        let granted: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM channel_visibility_grants g WHERE g.channel_id=? AND ((g.target_type='user' AND g.target_id=?) OR (g.target_type='role' AND g.target_id IN (SELECT role_id FROM user_roles WHERE server_id=? AND user_id=?))))",
        )
        .bind(&channel.id).bind(user_id)
        .bind(&channel.server_id).bind(user_id).fetch_one(connection).await?;
        Ok(granted)
    }
}
