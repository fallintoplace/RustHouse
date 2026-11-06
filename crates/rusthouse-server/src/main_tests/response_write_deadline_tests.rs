use super::*;

#[test]
fn deadline_failure_happens_before_any_network_write() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut server, _) = listener.accept().unwrap();
    let error = write_with_deadline(
        &mut server,
        b"not sent",
        Instant::now() - Duration::from_millis(1),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    drop(client);
}
