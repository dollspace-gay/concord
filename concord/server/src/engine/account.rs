use std::collections::HashSet;

use sqlx::{Row, SqlitePool};

use crate::auth::authority::{Actor, AuthError, AuthService};

/// One persisted account-scoped server folder. Collapse state remains a client
/// presentation preference and is intentionally not stored here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerFolder {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub server_ids: Vec<String>,
}

pub struct AccountProfile {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

pub struct PublicAccountProfile {
    pub username: String,
    pub avatar_url: Option<String>,
    pub provider: Option<String>,
    pub provider_id: Option<String>,
}

pub struct IrcToken {
    pub id: String,
    pub label: Option<String>,
    pub last_used: Option<String>,
    pub created_at: String,
}

#[derive(Clone)]
pub struct AccountService {
    pool: SqlitePool,
    auth: AuthService,
    writes: super::write_admission::WriteAdmission,
}

impl AccountService {
    pub fn new(
        pool: SqlitePool,
        auth: AuthService,
        writes: super::write_admission::WriteAdmission,
    ) -> Self {
        Self { pool, auth, writes }
    }

    pub async fn current_profile(&self, actor: &Actor) -> Result<Option<AccountProfile>, String> {
        let mut transaction = self.pool.begin().await.map_err(dependency_error)?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(authentication_error)?;
        let row = sqlx::query(
            "SELECT id,username,email,avatar_url FROM users WHERE id=? AND disabled_at IS NULL",
        )
        .bind(actor.user_id().as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        transaction.commit().await.map_err(dependency_error)?;
        Ok(row.map(|row| AccountProfile {
            id: row.get(0),
            username: row.get(1),
            email: row.get(2),
            avatar_url: row.get(3),
        }))
    }

    pub async fn public_profile(
        &self,
        nickname: &str,
    ) -> Result<Option<PublicAccountProfile>, String> {
        let row = crate::db::queries::users::get_user_by_nickname(&self.pool, nickname)
            .await
            .map_err(dependency_error)?;
        Ok(row.map(
            |(_id, username, _email, avatar_url, provider, provider_id)| PublicAccountProfile {
                username,
                avatar_url,
                provider,
                provider_id,
            },
        ))
    }

    pub async fn list_irc_tokens(&self, actor: &Actor) -> Result<Vec<IrcToken>, String> {
        let mut transaction = self.pool.begin().await.map_err(dependency_error)?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(authentication_error)?;
        let rows = sqlx::query(
            "SELECT id,label,last_used,created_at FROM irc_tokens WHERE user_id=? ORDER BY created_at DESC,id",
        )
        .bind(actor.user_id().as_str())
        .fetch_all(&mut *transaction)
        .await
        .map_err(dependency_error)?;
        transaction.commit().await.map_err(dependency_error)?;
        Ok(rows
            .into_iter()
            .map(|row| IrcToken {
                id: row.get(0),
                label: row.get(1),
                last_used: row.get(2),
                created_at: row.get(3),
            })
            .collect())
    }

    pub async fn list_server_folders(&self, actor: &Actor) -> Result<Vec<ServerFolder>, String> {
        self.auth
            .validate_actor(actor)
            .await
            .map_err(authentication_error)?;
        let rows = sqlx::query(
            "SELECT f.id,f.name,f.color,i.server_id \
             FROM server_folders f \
             LEFT JOIN server_folder_items i ON i.folder_id=f.id \
             WHERE f.user_id=? ORDER BY f.position,i.position",
        )
        .bind(actor.user_id().as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(dependency_error)?;
        let mut folders: Vec<ServerFolder> = Vec::new();
        for row in rows {
            let id: String = row.get(0);
            if folders.last().is_none_or(|folder| folder.id != id) {
                folders.push(ServerFolder {
                    id,
                    name: row.get(1),
                    color: row.get(2),
                    server_ids: Vec::new(),
                });
            }
            if let Some(server_id) = row.get::<Option<String>, _>(3) {
                folders
                    .last_mut()
                    .expect("folder was inserted")
                    .server_ids
                    .push(server_id);
            }
        }
        Ok(folders)
    }

    pub async fn replace_server_folders(
        &self,
        actor: &Actor,
        folders: &[ServerFolder],
    ) -> Result<(), String> {
        validate_folders(folders)?;
        let (_permit, mut transaction) = self
            .writes
            .begin()
            .await
            .map_err(|_| dependency_error_message())?;
        self.auth
            .validate_actor_in(&mut transaction, actor)
            .await
            .map_err(authentication_error)?;
        let user_id = actor.user_id().as_str();
        for folder in folders {
            for server_id in &folder.server_ids {
                let member: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id=? AND user_id=?)",
                )
                .bind(server_id)
                .bind(user_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(dependency_error)?;
                if !member {
                    return Err("FORBIDDEN: resource unavailable".into());
                }
            }
        }
        sqlx::query("DELETE FROM server_folders WHERE user_id=?")
            .bind(user_id)
            .execute(&mut *transaction)
            .await
            .map_err(dependency_error)?;
        for (folder_position, folder) in folders.iter().enumerate() {
            sqlx::query(
                "INSERT INTO server_folders(id,user_id,name,color,position) VALUES(?,?,?,?,?)",
            )
            .bind(&folder.id)
            .bind(user_id)
            .bind(folder.name.trim())
            .bind(folder.color.as_deref())
            .bind(folder_position as i64)
            .execute(&mut *transaction)
            .await
            .map_err(dependency_error)?;
            for (position, server_id) in folder.server_ids.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO server_folder_items(folder_id,server_id,position) VALUES(?,?,?)",
                )
                .bind(&folder.id)
                .bind(server_id)
                .bind(position as i64)
                .execute(&mut *transaction)
                .await
                .map_err(dependency_error)?;
            }
        }
        transaction.commit().await.map_err(dependency_error)?;
        Ok(())
    }
}

fn validate_folders(folders: &[ServerFolder]) -> Result<(), String> {
    if folders.len() > 100
        || folders
            .iter()
            .map(|folder| folder.server_ids.len())
            .sum::<usize>()
            > 1_000
    {
        return Err("INVALID_INPUT: folder layout is too large".into());
    }
    let mut folder_ids = HashSet::new();
    for folder in folders {
        let valid_color = folder.color.as_deref().is_none_or(|color| {
            color.len() == 7
                && color.starts_with('#')
                && color[1..]
                    .chars()
                    .all(|character| character.is_ascii_hexdigit())
        });
        if !folder_ids.insert(folder.id.as_str())
            || folder.id.is_empty()
            || folder.id.len() > 100
            || folder.name.trim().is_empty()
            || folder.name.trim().chars().count() > 100
            || folder.name.chars().any(char::is_control)
            || !valid_color
        {
            return Err("INVALID_INPUT: invalid folder layout".into());
        }
        let unique_servers = folder.server_ids.iter().collect::<HashSet<_>>();
        if unique_servers.len() != folder.server_ids.len() {
            return Err("INVALID_INPUT: duplicate server in folder".into());
        }
    }
    Ok(())
}

fn authentication_error(error: AuthError) -> String {
    match error {
        AuthError::Database(_) | AuthError::VerificationBusy | AuthError::HashWorker(_) => {
            dependency_error_message()
        }
        _ => "UNAUTHENTICATED: authentication required".into(),
    }
}

fn dependency_error(_: sqlx::Error) -> String {
    dependency_error_message()
}

fn dependency_error_message() -> String {
    "DEPENDENCY_UNAVAILABLE: account dependency unavailable".into()
}
