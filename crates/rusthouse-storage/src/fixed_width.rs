use std::io::{self, Read};

pub(crate) const MAX_BATCH_BYTES: usize = 64 * 1024 * 1024;

const READ_BUFFER_BYTES: usize = 16 * 1024;

pub(crate) fn read_fixed_width<R: Read, T, const WIDTH: usize>(
    reader: &mut R,
    rows: usize,
    error_message: &'static str,
    decode: impl Fn([u8; WIDTH]) -> T,
) -> io::Result<Vec<T>> {
    if rows > MAX_BATCH_BYTES / WIDTH {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, error_message));
    }
    let mut result = Vec::with_capacity(rows);
    let mut buffer = [0u8; READ_BUFFER_BYTES];
    while result.len() < rows {
        let count = (rows - result.len()).min(buffer.len() / WIDTH);
        let size = count * WIDTH;
        reader.read_exact(&mut buffer[..size])?;
        result.extend(buffer[..size].chunks_exact(WIDTH).map(|chunk| {
            let mut bytes = [0u8; WIDTH];
            bytes.copy_from_slice(chunk);
            decode(bytes)
        }));
    }
    Ok(result)
}
