use super::*;

#[test]
fn test_channel_list_params() {
    let json = r#"{"server_id": "srv-1"}"#;
    let params: ChannelListParams = serde_json::from_str(json).unwrap();
    assert_eq!(params.server_id, Some("srv-1".into()));
}

#[test]
fn test_channel_list_params_empty() {
    let json = r#"{}"#;
    let params: ChannelListParams = serde_json::from_str(json).unwrap();
    assert!(params.server_id.is_none());
}

#[test]
fn test_search_params_full() {
    let json =
        r##"{"server_id": "s1", "q": "hello", "channel": "#general", "limit": 10, "offset": 5}"##;
    let params: SearchParams = serde_json::from_str(json).unwrap();
    assert_eq!(params.server_id, "s1");
    assert_eq!(params.q, "hello");
    assert_eq!(params.channel, Some("#general".into()));
    assert_eq!(params.limit, Some(10));
    assert_eq!(params.offset, Some(5));
}

#[test]
fn test_search_params_minimal() {
    let json = r#"{"server_id": "s1", "q": "test"}"#;
    let params: SearchParams = serde_json::from_str(json).unwrap();
    assert_eq!(params.server_id, "s1");
    assert_eq!(params.q, "test");
    assert!(params.channel.is_none());
    assert!(params.limit.is_none());
    assert!(params.offset.is_none());
}
