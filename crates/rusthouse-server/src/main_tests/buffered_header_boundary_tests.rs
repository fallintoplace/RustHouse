use super::*;

#[test]
fn preserves_body_after_header_terminator_on_any_buffer_boundary() {
    let request = b"POST / HTTP/1.1\r\nHost: localhost:8123\r\nContent-Length: 4\r\n\r\nBODY";
    let terminator = request
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .unwrap()
        + 4;
    for split in 1..request.len() {
        let mut header = Vec::new();
        let (first, complete) = append_header_chunk(&mut header, &request[..split]).unwrap();
        if complete {
            assert_eq!(first, terminator);
            assert_eq!(header.len(), terminator);
        } else {
            assert_eq!(first, split);
            let (second, complete) =
                append_header_chunk(&mut header, &request[split..]).unwrap();
            assert!(complete);
            assert_eq!(split + second, terminator);
            assert_eq!(header.len(), terminator);
        }
    }
}

#[test]
fn rejects_header_budget_overflow_before_advancing_into_body() {
    let mut head = Vec::new();
    let oversized = vec![b'x'; MAX_HEADER_BYTES + 1];
    assert!(append_header_chunk(&mut head, &oversized).is_err());
    assert_eq!(head.len(), MAX_HEADER_BYTES + 1);
}
