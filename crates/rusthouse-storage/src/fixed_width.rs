use std::io::{self, Read};

pub(crate) const MAX_BATCH_BYTES: usize = 64 * 1024 * 1024;

const READ_BUFFER_BYTES: usize = 16 * 1024;

pub(crate) fn read_fixed_width<R: Read, T, const WIDTH: usize>(
    reader: &mut R,
    rows: usize,
    error_message: &'static str,
    decode: impl Fn([u8; WIDTH]) -> T,
) -> io::Result<Vec<T>> {
    if WIDTH == 0 || WIDTH > READ_BUFFER_BYTES || rows > MAX_BATCH_BYTES / WIDTH {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, error_message));
    }
    let mut result = Vec::with_capacity(rows);
    let mut buffer = [0u8; READ_BUFFER_BYTES];
    while result.len() < rows {
        let count = (rows - result.len()).min(buffer.len() / WIDTH);
        let size = count * WIDTH;
        reader.read_exact(&mut buffer[..size])?;
        result.extend(buffer[..size].chunks_exact(WIDTH).map(|chunk| {
            decode(chunk.try_into().expect("fixed-width chunk size"))
        }));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_invalid_element_widths_without_panicking() {
        let zero = read_fixed_width::<_, u8, 0>(
            &mut io::empty(),
            1,
            "invalid element width",
            |_| 0,
        );
        assert_eq!(zero.unwrap_err().kind(), io::ErrorKind::InvalidInput);
        let oversized = read_fixed_width::<_, u8, 16385>(
            &mut io::empty(),
            1,
            "invalid element width",
            |_| 0,
        );
        assert_eq!(oversized.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
}
