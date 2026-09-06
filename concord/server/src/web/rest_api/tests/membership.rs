use super::*;

#[test]
fn test_history_params_only_server_id() {
    let json = r#"{"server_id": "default"}"#;
    let params: HistoryParams = serde_json::from_str(json).unwrap();
    assert_eq!(params.server_id, Some("default".into()));
    assert!(params.before.is_none());
    assert!(params.limit.is_none());
}
