use rusthouse_server::{Config, handle_request};

fn config() -> Config {
    Config {
        max_rows: 2,
        ..Default::default()
    }
}

#[test]
fn markdown_format_works_for_get_post_and_empty_results() {
    let get = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%283%29&format=markdown",
        b"",
        &config(),
    );
    assert_eq!(get.status, "200 OK");
    assert_eq!(get.content_type, "text/markdown; charset=utf-8");
    assert_eq!(get.body, b"| number |\n| --- |\n| 0 |\n| 1 |\n");

    let post = handle_request(
        "POST",
        "/?format=markdown",
        b"SELECT 7 AS amount, 'x|<y>&' AS label",
        &config(),
    );
    assert_eq!(post.status, "200 OK");
    assert_eq!(
        post.body,
        b"| amount | label |\n| --- | --- |\n| 7 | x\\|&lt;y&gt;&amp; |\n"
    );

    let empty = handle_request(
        "POST",
        "/?format=markdown",
        b"SELECT number FROM numbers(0)",
        &config(),
    );
    assert_eq!(empty.status, "200 OK");
    assert_eq!(empty.body, b"| number |\n| --- |\n");
}
