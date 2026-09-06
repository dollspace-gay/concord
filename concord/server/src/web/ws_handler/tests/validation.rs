use super::*;

#[test]
fn test_malformed_json_completely_invalid() {
    assert!(parse_msg("not json at all").is_err());
}
