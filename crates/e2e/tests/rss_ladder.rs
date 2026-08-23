mod common;

fn parse_rss_n_from(raw: Option<&str>) -> Result<usize, &'static str> {
    match raw {
        None => Ok(1),
        Some("1") => Ok(1),
        Some("2") => Ok(2),
        Some("4") => Ok(4),
        _ => Err("rss_ladder: RSS_N must be 1, 2, or 4"),
    }
}

#[test]
fn parse_rss_n_default_and_allowed() {
    assert_eq!(parse_rss_n_from(None).unwrap(), 1);
    assert_eq!(parse_rss_n_from(Some("1")).unwrap(), 1);
    assert_eq!(parse_rss_n_from(Some("2")).unwrap(), 2);
    assert_eq!(parse_rss_n_from(Some("4")).unwrap(), 4);
}

#[test]
fn parse_rss_n_rejects_bad() {
    let err = "rss_ladder: RSS_N must be 1, 2, or 4";
    assert_eq!(parse_rss_n_from(Some("")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("3")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("50")).unwrap_err(), err);
    assert_eq!(parse_rss_n_from(Some("one")).unwrap_err(), err);
}
