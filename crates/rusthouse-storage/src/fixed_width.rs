use std::io::{self, Read};

pub(crate) const MAX_BATCH_BYTES: usize = 64 * 1024 * 1024;

const READ_BUFFER_BYTES: usize = 64 * 1024;
const MAX_ELEMENT_WIDTH: usize = 16 * 1024;

fn append_decoded<T, const WIDTH: usize>(
    result: &mut Vec<T>,
    bytes: &[u8],
    decode: &impl Fn([u8; WIDTH]) -> T,
) {
    result.extend(
        bytes
            .chunks_exact(WIDTH)
            .map(|chunk| decode(chunk.try_into().expect("fixed-width chunk size"))),
    );
}

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
    // Small marks should not initialize the entire 64 KiB scan buffer.
    const SMALL_BYTES: usize = 512;
    let batch_bytes = rows * WIDTH;
    if batch_bytes <= SMALL_BYTES {
        let mut buffer = [0u8; SMALL_BYTES];
        reader.read_exact(&mut buffer[..batch_bytes])?;
        append_decoded::<T, WIDTH>(&mut result, &buffer[..batch_bytes], &decode);
        return Ok(result);
    }
    let mut buffer = [0u8; READ_BUFFER_BYTES];
    while result.len() < rows {
        let count = (rows - result.len()).min(buffer.len() / WIDTH);
        let size = count * WIDTH;
        reader.read_exact(&mut buffer[..size])?;
        append_decoded::<T, WIDTH>(&mut result, &buffer[..size], &decode);
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

#[cfg(test)]
mod small_mark_tests {
    use super::*;

    #[test]
    fn decodes_small_marks_and_preserves_the_next_record() {
        let mut data = Vec::new();
        for number in [4u64, 5, 6, 7] {
            data.extend_from_slice(&number.to_le_bytes());
        }
        let mut input = data.as_slice();
        let first = read_fixed_width::<_, u64, 8>(
            &mut input, 3, "invalid size", u64::from_le_bytes,
        ).unwrap();
        assert_eq!(first, vec![4, 5, 6]);
        let second = read_fixed_width::<_, u64, 8>(
            &mut input, 1, "invalid size", u64::from_le_bytes,
        ).unwrap();
        assert_eq!(second, vec![7]);
        assert!(input.is_empty());
    }
}
