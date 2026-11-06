use super::*;

#[test]
fn busy_response_has_a_valid_bounded_content_length() {
    let (head, body) = BUSY_RESPONSE.split_at(
        BUSY_RESPONSE
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap()
            + 4,
    );
    assert!(head.starts_with(b"HTTP/1.1 503 Service Unavailable\r\n"));
    assert!(
        head.windows(b"Content-Length: 12\r\n".len())
            .any(|part| part == b"Content-Length: 12\r\n")
    );
    assert_eq!(body, b"server busy\n");
}

#[test]
fn busy_replies_are_sent_without_the_regular_response_writer() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();
    reject_busy_connection(server);
    let mut response = Vec::new();
    let mut client = client;
    client.read_to_end(&mut response).unwrap();
    assert_eq!(response, BUSY_RESPONSE);
}
