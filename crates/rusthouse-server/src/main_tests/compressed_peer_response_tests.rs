use super::*;
use std::net::Shutdown;

#[test]
fn negotiated_sql_response_compresses_without_changing_tsv_bytes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let client = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut output = Vec::new();
        stream.read_to_end(&mut output).unwrap();
        output
    });
    let (mut stream, _) = listener.accept().unwrap();
    let plain = b"user\tcount\n".repeat(8192);
    respond(
        &mut stream,
        Response {
            status: "200 OK",
            content_type: "text/tab-separated-values; charset=utf-8",
            body: plain.clone(),
        },
        true,
    )
    .unwrap();
    drop(stream);
    let raw = client.join().unwrap();
    let end = raw
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .unwrap()
        + 4;
    assert!(
        raw[..end]
            .windows(b"Content-Encoding: x-rusthouse-lz4".len())
            .any(|bytes| bytes == b"Content-Encoding: x-rusthouse-lz4")
    );
    assert_eq!(
        lz4_flex::decompress_size_prepended(&raw[end..]).unwrap(),
        plain
    );
}
