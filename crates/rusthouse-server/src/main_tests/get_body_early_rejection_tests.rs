use super::*;

#[test]
fn rejects_get_body_without_waiting_for_advertised_bytes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .write_all(b"GET /ping HTTP/1.1\r\nHost: localhost\r\nContent-Length: 65536\r\n\r\n")
        .unwrap();
    let (mut server, _) = listener.accept().unwrap();
    server
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    assert_eq!(
        read_request(&mut server)
            .err()
            .map(|response| response.status),
        Some("400 Bad Request"),
    );
    assert!(parse_request_line("GET /ping HTTP/1.1").is_ok());
}
