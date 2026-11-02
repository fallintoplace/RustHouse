use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn extracts_significant_domain_labels_and_suffixes() {
    for (sql, expected) in [
        ("SELECT firstSignificantSubdomain('https://news.clickhouse.com/')", "clickhouse"),
        ("SELECT firstSignificantSubdomain('https://news.clickhouse.com.tr/')", "clickhouse"),
        ("SELECT firstSignificantSubdomain('https://www.bbc.co.uk')", "bbc"),
        ("SELECT firstSignificantSubdomain('example.org')", "example"),
        ("SELECT cutToFirstSignificantSubdomain('https://news.clickhouse.com.tr/')", "clickhouse.com.tr"),
        ("SELECT cutToFirstSignificantSubdomain('www.example.com')", "example.com"),
        ("SELECT cutToFirstSignificantSubdomain('www.tr')", "tr"),
        ("SELECT cutToFirstSignificantSubdomain('tr')", ""),
        ("SELECT cutToFirstSignificantSubdomainWithWWW('www.tr')", "www.tr"),
        ("SELECT cutToFirstSignificantSubdomainWithWWW('https://www.example.com?q=1')", "www.example.com"),
        ("SELECT cutToFirstSignificantSubdomainWithWWW('https://news.clickhouse.com.tr/')", "clickhouse.com.tr"),
        ("SELECT cutToFirstSignificantSubdomain('https://user@example.com:8443/a')", "example.com"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
}

#[test]
fn rejects_malformed_hosts_and_invalid_arity() {
    for sql in [
        "SELECT firstSignificantSubdomain('http://127.0.0.1')",
        "SELECT firstSignificantSubdomain('https://bad..com')",
        "SELECT firstSignificantSubdomain('https://[::1]/')",
    ] {
        assert_eq!(value(sql), ConstantValue::Text(String::new()), "{sql}");
    }
    assert_eq!(value("SELECT cutToFirstSignificantSubdomain(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT firstSignificantSubdomain()").is_err());
    assert!(parse_select("SELECT cutToFirstSignificantSubdomain('example.com', 'extra')").is_err());
}
