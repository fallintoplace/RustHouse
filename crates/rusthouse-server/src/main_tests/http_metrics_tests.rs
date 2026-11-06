use super::*;

#[test]
fn metrics_use_stable_prometheus_counter_format() {
    let response = metrics_response(125, 7);
    assert_eq!(response.status, "200 OK");
    assert_eq!(
        response.content_type,
        "text/plain; version=0.0.4; charset=utf-8"
    );
    let text = std::str::from_utf8(&response.body).unwrap();
    assert!(text.contains("# TYPE rusthouse_http_requests_total counter\n"));
    assert!(text.contains("rusthouse_http_requests_total 125\n"));
    assert!(text.contains("# TYPE rusthouse_http_errors_total counter\n"));
    assert!(text.contains("rusthouse_http_errors_total 7\n"));
}
