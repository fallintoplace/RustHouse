use super::*;

struct InterruptThenPartial {
    bytes: Vec<u8>,
    calls: usize,
}

impl Write for InterruptThenPartial {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls == 1 {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = input.len().min(3);
        self.bytes.extend_from_slice(&input[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn interrupted_and_partial_writes_preserve_every_response_byte() {
    let mut writer = InterruptThenPartial {
        bytes: Vec::new(),
        calls: 0,
    };
    let mut input = b"HTTP/1.1 200 OK\r\n\r\nhello".as_slice();
    assert_eq!(write_response_chunk(&mut writer, input).unwrap(), None);
    while !input.is_empty() {
        if let Some(count) = write_response_chunk(&mut writer, input).unwrap() {
            input = &input[count..];
        }
    }
    assert_eq!(writer.bytes, b"HTTP/1.1 200 OK\r\n\r\nhello");
    assert!(writer.calls > 2);
}

#[test]
fn zero_length_write_is_not_retried_forever() {
    assert_eq!(
        write_response_chunk(&mut io::sink(), b"")
            .unwrap_err()
            .kind(),
        io::ErrorKind::WriteZero,
    );
}
