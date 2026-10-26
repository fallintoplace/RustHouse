use std::fmt;
use std::io::{self, Read};

const MAX_BATCH_BYTES: usize = 64 * 1024 * 1024;
const READ_BUFFER_BYTES: usize = 16 * 1024;

/// A UUID in canonical byte order, independent of ClickHouse's disk encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UuidValue {
    bytes: [u8; 16],
}

impl UuidValue {
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.bytes
    }
}

impl fmt::Display for UuidValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.bytes.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Decode ClickHouse UUID values from pairs of little-endian UInt64 halves.
///
/// Each half is reversed separately to obtain the canonical UUID byte order.
/// Rows are bounded to 64 MiB and decoded without reading into the next batch.
pub fn read_uuid_values<R: Read>(reader: &mut R, rows: usize) -> io::Result<Vec<UuidValue>> {
    if rows > MAX_BATCH_BYTES / 16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "UUID batch exceeds 64 MiB",
        ));
    }

    let mut result = Vec::with_capacity(rows);
    let mut buffer = [0u8; READ_BUFFER_BYTES];
    while result.len() < rows {
        let count = (rows - result.len()).min(buffer.len() / 16);
        let size = count * 16;
        reader.read_exact(&mut buffer[..size])?;
        result.extend(buffer[..size].chunks_exact(16).map(|chunk| {
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(chunk);
            bytes[..8].reverse();
            bytes[8..].reverse();
            UuidValue { bytes }
        }));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const EXAMPLE: [u8; 16] = [
        0xe7, 0x11, 0xb3, 0x5c, 0x04, 0xc4, 0xf0, 0x61, 0xa0, 0xdb, 0xd3, 0x6a, 0x00,
        0xa6, 0x7b, 0x90,
    ];

    #[test]
    fn decodes_two_little_endian_halves_into_standard_uuid_order() {
        let mut disk = Vec::from(EXAMPLE);
        disk.extend_from_slice(&[0; 16]);
        disk.push(77);
        let mut reader = Cursor::new(disk);

        let values = read_uuid_values(&mut reader, 2).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(
            values[0].to_string(),
            "61f0c404-5cb3-11e7-907b-a6006ad3dba0"
        );
        assert_eq!(
            values[0].as_bytes(),
            &[
                0x61, 0xf0, 0xc4, 0x04, 0x5c, 0xb3, 0x11, 0xe7, 0x90, 0x7b, 0xa6, 0,
                0x6a, 0xd3, 0xdb, 0xa0
            ]
        );
        assert_eq!(
            values[1].to_string(),
            "00000000-0000-0000-0000-000000000000"
        );
        assert_eq!(reader.position(), 32);
        assert_eq!(read_uuid_values(&mut &[][..], 0).unwrap(), vec![]);
    }

    #[test]
    fn rejects_truncated_and_oversized_batches_before_allocation() {
        assert_eq!(
            read_uuid_values(&mut &EXAMPLE[..15], 1)
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(
            read_uuid_values(&mut &[][..], MAX_BATCH_BYTES / 16 + 1)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn reads_bounded_chunks_and_tolerates_short_reads() {
        struct CountingReader<R> {
            inner: R,
            reads: usize,
        }
        impl<R: Read> Read for CountingReader<R> {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                self.reads += 1;
                self.inner.read(bytes)
            }
        }

        let disk = EXAMPLE.repeat(2048);
        let mut reader = CountingReader {
            inner: disk.as_slice(),
            reads: 0,
        };
        let decoded = read_uuid_values(&mut reader, 2048).unwrap();
        assert_eq!(reader.reads, 2);
        assert_eq!(decoded[1024], decoded[0]);

        struct Bytewise<R>(R);
        impl<R: Read> Read for Bytewise<R> {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                let count = bytes.len().min(1);
                self.0.read(&mut bytes[..count])
            }
        }

        let decoded = read_uuid_values(&mut Bytewise(EXAMPLE.as_slice()), 1).unwrap();
        assert_eq!(
            decoded[0].to_string(),
            "61f0c404-5cb3-11e7-907b-a6006ad3dba0"
        );
    }
}
