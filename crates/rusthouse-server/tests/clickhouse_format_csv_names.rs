use rusthouse_server::{Config, handle_request};

#[test]
fn clickhouse_csv_names_http_output() {
    let config = Config { max_rows: 10, ..Default::default() };
    let response = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%282%29&format=CSVWithNames",
        b"",
        &config,
    );
    assert_eq!(response.status, "200 OK", "{:?}", response.body);
    assert_eq!(response.content_type, "text/csv; charset=utf-8");
    assert_eq!(response.body, b"number\r\n0\r\n1\r\n");
}
