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

mod authorization;
mod behavior;
mod lifecycle;
mod membership;
mod messaging;
mod revocation;
