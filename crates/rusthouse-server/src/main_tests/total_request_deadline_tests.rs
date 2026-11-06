use super::*;
use std::net::TcpListener;
use std::time::Instant;

#[test]
fn slow_header_bytes_cannot_extend_total_request_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let writer = std::thread::spawn(move || {
        let mut client = TcpStream::connect(address).unwrap();
        for byte in b"GET /ping HTTP/1.1\\r\\n" {
            if client.write_all(&[*byte]).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(35));
        }
    });
    let (mut server, _) = listener.accept().unwrap();
    let started = Instant::now();
    let response =
        read_request_with_deadline(&mut server, Duration::from_millis(120)).unwrap_err();
    assert_eq!(response.status, "408 Request Timeout");
    assert!(started.elapsed() < Duration::from_secs(1));
    drop(server);
    writer.join().unwrap();
}

#[test]
fn slow_body_bytes_cannot_extend_total_request_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let writer = std::thread::spawn(move || {
        let mut client = TcpStream::connect(address).unwrap();
        client
            .write_all(b"POST / HTTP/1.1\\r\\nContent-Length: 32\\r\\n\\r\\n")
            .unwrap();
        for _ in 0..32 {
            if client.write_all(b"x").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(35));
        }
    });
    let (mut server, _) = listener.accept().unwrap();
    let response =
        read_request_with_deadline(&mut server, Duration::from_millis(120)).unwrap_err();
    assert_eq!(response.status, "408 Request Timeout");
    drop(server);
    writer.join().unwrap();
}
