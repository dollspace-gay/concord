use super::*;

#[test]
fn test_nick_change_event() {
    let engine = test_engine();
    let lines = event_to_irc_lines(
        &engine,
        "viewer",
        &ChatEvent::NickChange {
            old_nick: "alice".into(),
            new_nick: "alice_".into(),
        },
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("NICK"));
    assert!(lines[0].contains("alice_"));
}
