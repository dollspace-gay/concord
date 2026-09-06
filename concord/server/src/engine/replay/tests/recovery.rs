use super::*;

#[tokio::test]
async fn cursor_is_opaque_tamper_evident_and_survives_service_restart() {
    let (pool, auth, actor, conversation, messaging, replay) = fixture().await;
    send(&messaging, &actor, "send-1", "first").await;
    let snapshot = replay
        .snapshot(&actor, std::slice::from_ref(&conversation))
        .await
        .unwrap();
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&snapshot.cursor)
        .unwrap();
    assert!(!String::from_utf8_lossy(&decoded).contains("event_sequence"));
    assert!(!String::from_utf8_lossy(&decoded).contains("user"));

    let restarted = ReplayService::new(pool, auth, "persistent-secret");
    let batch = restarted
        .replay(
            &actor,
            std::slice::from_ref(&conversation),
            &snapshot.cursor,
            100,
        )
        .await
        .unwrap();
    assert!(batch.events.is_empty());

    let mut tampered = snapshot.cursor.into_bytes();
    let last = tampered.last_mut().unwrap();
    *last = if *last == b'A' { b'B' } else { b'A' };
    assert!(matches!(
        restarted
            .replay(
                &actor,
                &[conversation],
                std::str::from_utf8(&tampered).unwrap(),
                100
            )
            .await,
        Err(ReplayError::ResyncRequired(ResyncReason::InvalidCursor))
    ));
}
