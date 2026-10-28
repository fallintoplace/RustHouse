use rusthouse_server::{Config, handle_request};

#[test]
fn clickhouse_tab_separated_names_http_output() {
    let config = Config { max_rows: 10, ..Default::default() };
    let response = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%282%29&format=TabSeparatedWithNames",
        b"",
        &config,
    );
    assert_eq!(response.status, "200 OK", "{:?}", response.body);
    assert_eq!(response.content_type, "text/tab-separated-values; charset=utf-8");
    assert_eq!(response.body, b"number\n0\n1\n");
}
