use std::io::{self, Read};

pub(crate) const MAX_BATCH_BYTES: usize = 64 * 1024 * 1024;

const READ_BUFFER_BYTES: usize = 64 * 1024;
const MAX_ELEMENT_WIDTH: usize = 16 * 1024;

pub(crate) fn read_fixed_width<R: Read, T, const WIDTH: usize>(
    reader: &mut R,
    rows: usize,
    error_message: &'static str,
    decode: impl Fn([u8; WIDTH]) -> T,
) -> io::Result<Vec<T>> {
    if WIDTH == 0 || WIDTH > MAX_ELEMENT_WIDTH || rows > MAX_BATCH_BYTES / WIDTH {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, error_message));
    }
    // Empty batches are common at mark and part boundaries. Avoid entering
    // the fixed-width decode path or setting up its 64 KiB scratch buffer.
    if rows == 0 {
        return Ok(Vec::new());
    }
    let mut result = Vec::with_capacity(rows);
    let mut buffer = [0u8; READ_BUFFER_BYTES];
    while result.len() < rows {
        let count = (rows - result.len()).min(buffer.len() / WIDTH);
        let size = count * WIDTH;
        reader.read_exact(&mut buffer[..size])?;
        result.extend(
            buffer[..size]
                .chunks_exact(WIDTH)
                .map(|chunk| decode(chunk.try_into().expect("fixed-width chunk size"))),
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_large_fixed_width_batches_in_one_chunk() {
        struct Counting<R> {
            inner: R,
            calls: usize,
        }
        impl<R: Read> Read for Counting<R> {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                self.calls += 1;
                self.inner.read(bytes)
            }
        }

        let mut input = Counting {
            inner: io::Cursor::new(vec![7u8; 32 * 1024]),
            calls: 0,
        };
        let values = read_fixed_width::<_, u64, 8>(
            &mut input,
            4096,
            "invalid fixed-width input",
            u64::from_le_bytes,
        )
        .unwrap();
        assert_eq!(values.len(), 4096);
        assert_eq!(input.calls, 1);
    }

    #[test]
    fn refuses_invalid_element_widths_without_panicking() {
        let zero =
            read_fixed_width::<_, u8, 0>(&mut io::empty(), 1, "invalid element width", |_| 0);
        assert_eq!(zero.unwrap_err().kind(), io::ErrorKind::InvalidInput);
        let oversized =
            read_fixed_width::<_, u8, 16385>(&mut io::empty(), 1, "invalid element width", |_| 0);
        assert_eq!(oversized.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
}

#[cfg(test)]
mod empty_batch_fast_path_tests {
    use super::*;

    #[test]
    fn empty_batch_never_reads_the_underlying_stream() {
        struct NoReads;
        impl Read for NoReads {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("unexpected read for an empty batch");
            }
        }
        let decoded =
            read_fixed_width::<_, u64, 8>(&mut NoReads, 0, "invalid width", u64::from_le_bytes)
                .unwrap();
        assert!(decoded.is_empty());
    }
}
