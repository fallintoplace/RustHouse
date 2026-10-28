//! Streaming CRC32C for accidental corruption checks, not authentication.
use std::io::{self, Read};

const POLYNOMIAL: u32 = 0x82f6_3b78;

/// Compute CRC32C (Castagnoli) for one byte slice.
pub fn crc32c(bytes: &[u8]) -> u32 {
    update(!0, bytes) ^ !0
}

fn update(mut crc: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 == 1 { POLYNOMIAL } else { 0 };
        }
    }
    crc
}

/// Stream a file without reading beyond max_bytes. Returns (length, checksum).
pub fn crc32c_reader(reader: &mut impl Read, max_bytes: u64) -> io::Result<(u64, u32)> {
    let mut crc = !0u32;
    let mut total = 0u64;
    let mut buffer = [0u8; 32 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("transfer length overflow"))?;
        if total > max_bytes {
            return Err(io::Error::other("transfer exceeds configured byte cap"));
        }
        crc = update(crc, &buffer[..count]);
    }
    Ok((total, crc ^ !0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn matches_castagnoli_reference_vector() {
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
        assert_eq!(crc32c(b""), 0);
    }

    #[test]
    fn streaming_matches_in_memory_and_enforces_cap() {
        let data = vec![42u8; 75_000];
        assert_eq!(
            crc32c_reader(&mut Cursor::new(&data), data.len() as u64).unwrap(),
            (75_000, crc32c(&data))
        );
        assert!(crc32c_reader(&mut Cursor::new(data), 74_999).is_err());
    }
}
