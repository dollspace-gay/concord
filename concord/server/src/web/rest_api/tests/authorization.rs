use super::*;

#[test]
fn private_media_range_parser_rejects_ambiguous_and_out_of_bounds_ranges() {
    assert_eq!(parse_single_range(None, 10), Some((0, 9)));
    assert_eq!(parse_single_range(Some("bytes=2-5"), 10), Some((2, 5)));
    assert_eq!(parse_single_range(Some("bytes=-3"), 10), Some((7, 9)));
    assert_eq!(parse_single_range(Some("bytes=7-"), 10), Some((7, 9)));
    assert_eq!(parse_single_range(Some("bytes=10-11"), 10), None);
    assert_eq!(parse_single_range(Some("bytes=1-2,4-5"), 10), None);
}
