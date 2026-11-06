use super::*;

#[test]
fn unsupported_expect_does_not_wait_for_the_request_body() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .write_all(
            b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1024\r\nExpect: 100-continue\r\n\r\n",
        )
        .unwrap();
    let (mut server, _) = listener.accept().unwrap();
    server
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    assert_eq!(
        read_request(&mut server)
            .err()
            .map(|response| response.status),
        Some("417 Expectation Failed"),
    );
}
