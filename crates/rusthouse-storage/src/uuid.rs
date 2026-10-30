#[cfg(test)]
use crate::fixed_width::MAX_BATCH_BYTES;
use crate::fixed_width::read_fixed_width;
use std::fmt;
use std::io::{self, Read};

/// A UUID in canonical byte order, independent of ClickHouse's disk encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UuidValue {
    bytes: [u8; 16],
}

impl UuidValue {
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.bytes
    }

    fn decode_uuid_disk_bytes(mut bytes: [u8; 16]) -> Self {
        for half in bytes.chunks_exact_mut(8) {
            half.reverse();
        }
        Self { bytes }
    }
}

impl fmt::Display for UuidValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut text = [0u8; 36];
        let mut offset = 0;
        for (index, &byte) in self.bytes.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                text[offset] = b'-';
                offset += 1;
            }
            text[offset] = HEX[usize::from(byte >> 4)];
            text[offset + 1] = HEX[usize::from(byte & 0x0f)];
            offset += 2;
        }
        f.write_str(std::str::from_utf8(&text).expect("UUID formatting produces ASCII"))
    }
}

/// Decode ClickHouse UUID values from pairs of little-endian UInt64 halves.
///
/// Each half is reversed separately to obtain the canonical UUID byte order.
/// Rows are bounded to 64 MiB and decoded without reading into the next batch.
pub fn read_uuid_values<R: Read>(reader: &mut R, rows: usize) -> io::Result<Vec<UuidValue>> {
    read_fixed_width(
        reader,
        rows,
        "UUID batch exceeds 64 MiB",
        UuidValue::decode_uuid_disk_bytes,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const EXAMPLE: [u8; 16] = [
        0xe7, 0x11, 0xb3, 0x5c, 0x04, 0xc4, 0xf0, 0x61, 0xa0, 0xdb, 0xd3, 0x6a, 0x00, 0xa6, 0x7b,
        0x90,
    ];

    #[test]
    fn formats_with_one_string_write() {
        struct Sink {
            calls: usize,
            output: String,
        }
        impl fmt::Write for Sink {
            fn write_str(&mut self, text: &str) -> fmt::Result {
                self.calls += 1;
                self.output.push_str(text);
                Ok(())
            }
        }
        let value = UuidValue::decode_uuid_disk_bytes(EXAMPLE);
        let mut sink = Sink {
            calls: 0,
            output: String::new(),
        };
        fmt::write(&mut sink, format_args!("{value}")).unwrap();
        assert_eq!(sink.calls, 1);
        assert_eq!(sink.output, "61f0c404-5cb3-11e7-907b-a6006ad3dba0");
    }

    #[test]
    fn on_disk_half_reversal_is_reversible() {
        let disk: [u8; 16] = std::array::from_fn(|index| index as u8);
        let canonical = UuidValue::decode_uuid_disk_bytes(disk);
        assert_eq!(
            *UuidValue::decode_uuid_disk_bytes(*canonical.as_bytes()).as_bytes(),
            disk
        );
    }

    #[test]
    fn canonical_group_lengths_are_explicit() {
        let value = UuidValue { bytes: [0xff; 16] };
        assert_eq!(value.to_string(), "ffffffff-ffff-ffff-ffff-ffffffffffff");
    }

    #[test]
    fn converts_uuid_disk_halves_independently() {
        let disk = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let value = UuidValue::decode_uuid_disk_bytes(disk);
        assert_eq!(
            value.as_bytes(),
            &[7, 6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8]
        );
    }

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
                0x61, 0xf0, 0xc4, 0x04, 0x5c, 0xb3, 0x11, 0xe7, 0x90, 0x7b, 0xa6, 0, 0x6a, 0xd3,
                0xdb, 0xa0
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
            read_uuid_values(&mut &EXAMPLE[..15], 1).unwrap_err().kind(),
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
        assert_eq!(reader.reads, 1);
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
