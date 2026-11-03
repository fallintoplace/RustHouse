use rusthouse_server::{Config, handle_request};

#[test]
fn grouped_literal_is_present_in_http_formats() {
    let config = Config {
        max_rows: 10,
        ..Config::default()
    };
    let sql = b"SELECT 1, number, COUNT(*) AS c FROM numbers(3) GROUP BY 1, number ORDER BY number";
    let tsv = handle_request("POST", "/", sql, &config);
    assert_eq!(
        tsv.status,
        "200 OK",
        "{}",
        String::from_utf8_lossy(&tsv.body)
    );
    assert_eq!(tsv.body, b"1\tnumber\tc\n1\t0\t1\n1\t1\t1\n1\t2\t1\n");
    let json = handle_request("POST", "/?format=json", sql, &config);
    assert_eq!(
        json.status,
        "200 OK",
        "{}",
        String::from_utf8_lossy(&json.body)
    );
    assert!(String::from_utf8_lossy(&json.body).contains("\"1\":\"1\""));
}
