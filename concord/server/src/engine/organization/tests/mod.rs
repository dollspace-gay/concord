use super::*;

fn server_id(value: &str) -> ServerId {
    ServerId::from_stored(value).unwrap()
}

fn channel_id(value: &str) -> ChannelId {
    ChannelId::from_stored(value).unwrap()
}

async fn fixture() -> (SqlitePool, OrganizationService, Actor, String) {
    let pool = crate::db::pool::create_pool("sqlite::memory:")
        .await
        .unwrap();
    crate::db::pool::run_migrations(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username) VALUES('owner','owner'),('member','member')")
        .execute(&pool)
        .await
        .unwrap();
    crate::db::queries::servers::create_server(&pool, "server", "Server", "owner", None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO roles(id,server_id,name,position,permissions,is_default) VALUES('everyone','server','@everyone',0,0,1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO server_members(server_id,user_id,role) VALUES('server','member','member')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let default_role: String =
        sqlx::query_scalar("SELECT id FROM roles WHERE server_id='server' AND is_default=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let auth = AuthService::new(pool.clone(), "test-secret".into(), 1);
    let (_, actor) = auth.issue_web_session("owner").await.unwrap();
    let service = OrganizationService::new(
        pool.clone(),
        auth,
        super::super::write_admission::WriteAdmission::new(pool.clone()),
    );
    (pool, service, actor, default_role)
}

mod authorization;
mod behavior;
mod recovery;
mod revocation;
mod validation;
