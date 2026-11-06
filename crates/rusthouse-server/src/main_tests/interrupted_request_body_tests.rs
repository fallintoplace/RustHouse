use super::*;

struct InterruptedOnce(bool);

impl Read for InterruptedOnce {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !self.0 {
            self.0 = true;
            return Err(io::ErrorKind::Interrupted.into());
        }
        buffer[..2].copy_from_slice(b"OK");
        Ok(2)
    }
}

#[test]
fn interrupted_body_read_retries_without_losing_bytes() {
    let mut reader = InterruptedOnce(false);
    let mut result = [0u8; 2];
    assert_eq!(
        read_body_chunk_with_deadline(
            &mut reader,
            &mut result,
            Instant::now(),
            Duration::from_secs(5),
        )
        .expect("interrupted body read should succeed"),
        2,
    );
    assert_eq!(&result, b"OK");
}

#[test]
fn expired_body_deadline_fails_before_reading() {
    let mut reader = InterruptedOnce(false);
    let mut result = [0u8; 2];
    assert!(read_body_chunk_with_deadline(
        &mut reader, &mut result, Instant::now(), Duration::ZERO,
    ).is_err());
    assert!(!reader.0);
}
