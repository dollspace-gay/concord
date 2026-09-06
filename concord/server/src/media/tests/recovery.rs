use super::*;

#[tokio::test]
async fn cancelled_start_after_reservation_leaves_only_collectable_state() {
    let (d, p, conversation) = fixture().await;
    let reserved = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let task = {
        let pool = p.clone();
        let root = d.path().to_owned();
        let reserved = reserved.clone();
        let resume = resume.clone();
        tokio::spawn(async move {
            MediaUpload::start_inner(
                pool,
                &root,
                StartMedia {
                    owner_id: "u",
                    intent: MediaIntent::Message {
                        conversation_id: conversation,
                    },
                    original_filename: "cancelled.bin",
                    content_type: "application/octet-stream",
                    max_bytes: 8,
                    per_user_bytes: 16,
                    total_bytes: 64,
                },
                Some((reserved, resume)),
            )
            .await
        })
    };
    reserved.notified().await;
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    let row: (String, i64) = sqlx::query_as(
        "SELECT media_state,reserved_bytes FROM attachments WHERE original_filename='cancelled.bin'",
    )
    .fetch_one(&p)
    .await
    .unwrap();
    assert_eq!(row, ("staging".into(), 8));
    assert_eq!(
        std::fs::read_dir(d.path().join("staging")).unwrap().count(),
        0
    );
    sqlx::query("UPDATE attachments SET upload_updated_at=datetime('now','-10 seconds') WHERE original_filename='cancelled.bin'")
        .execute(&p)
        .await
        .unwrap();
    assert_eq!(collect_expired(&p, d.path(), 1).await.unwrap(), 1);
    let row: (String, i64) = sqlx::query_as(
        "SELECT media_state,reserved_bytes FROM attachments WHERE original_filename='cancelled.bin'",
    )
    .fetch_one(&p)
    .await
    .unwrap();
    assert_eq!(row, ("failed".into(), 0));
}
