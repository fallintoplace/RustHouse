use super::*;

#[test]
fn client_connections_receive_read_and_write_timeouts() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();
    configure_client_timeouts(&server).unwrap();
    assert_eq!(server.read_timeout().unwrap(), Some(Duration::from_secs(5)));
    assert_eq!(
        server.write_timeout().unwrap(),
        Some(Duration::from_secs(5))
    );
    drop(client);
}
