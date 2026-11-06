use super::*;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

#[test]
fn rejects_noncanonical_capability_row_limits() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let responder = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
        }
        let body = b"{\"name\":\"RustHouse\",\"source\":\"database\",\"read_only\":true,\"max_rows\":42extra}\n";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
    });
    let peer = Peer::parse(&format!("127.0.0.1:{}", address.port())).unwrap();
    assert!(shard_row_cap(peer).is_err());
    responder.join().unwrap();
}
