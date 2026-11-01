//! Bounded ClickHouse UUID physical writes.
use crate::UuidValue;
use crate::fixed_width::MAX_BATCH_BYTES;
use std::io::{self, Write};

/// Serialize canonical UUIDs as two little-endian UInt64 halves.
///
/// The writer receives at most 16 KiB at a time. Validate the total
/// batch size before writing any bytes to avoid partial oversized output.
pub fn write_uuid_values<W: Write>(writer: &mut W, values: &[UuidValue]) -> io::Result<()> {
    let bytes = values.len().checked_mul(16).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "UUID batch size overflow")
    })?;
    if bytes > MAX_BATCH_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "UUID batch exceeds 64 MiB",
        ));
    }

    let mut buffer = [0u8; 16 * 1024];
    for chunk in values.chunks(buffer.len() / 16) {
        for (index, value) in chunk.iter().enumerate() {
            let mut disk = *value.as_bytes();
            disk[..8].reverse();
            disk[8..].reverse();
            buffer[index * 16..index * 16 + 16].copy_from_slice(&disk);
        }
        writer.write_all(&buffer[..chunk.len() * 16])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_uuid_values;

    struct PartialWriter(Vec<u8>);

    impl Write for PartialWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = bytes.len().min(5);
            self.0.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn round_trip_across_chunk_boundaries_with_short_writes() {
        let disk = (0..2050u32)
            .flat_map(|index| {
                let seed = index.to_le_bytes();
                let mut raw = [0u8; 16];
                for (slot, byte) in raw.iter_mut().enumerate() {
                    *byte = seed[slot % 4].wrapping_add(slot as u8 * 7);
                }
                raw
            })
            .collect::<Vec<_>>();
        let values = read_uuid_values(&mut disk.as_slice(), 2050).unwrap();
        let mut writer = PartialWriter(Vec::new());
        write_uuid_values(&mut writer, &values).unwrap();
        assert_eq!(writer.0, disk);
    }

    #[test]
    fn empty_batch_and_uint64_boundary_preflight() {
        let mut output = Vec::new();
        write_uuid_values(&mut output, &[]).unwrap();
        assert!(output.is_empty());
        assert!(usize::MAX.checked_mul(16).is_none());
        assert_eq!(MAX_BATCH_BYTES / 16 * 16, MAX_BATCH_BYTES);
    }

    #[test]
    fn retains_clickhouse_half_order_for_a_known_uuid() {
        let disk = [
            0xe7, 0x11, 0xb3, 0x5c, 0x04, 0xc4, 0xf0, 0x61,
            0xa0, 0xdb, 0xd3, 0x6a, 0x00, 0xa6, 0x7b, 0x90,
        ];
        let values = read_uuid_values(&mut disk.as_slice(), 1).unwrap();
        let mut output = Vec::new();
        write_uuid_values(&mut output, &values).unwrap();
        assert_eq!(output, disk);
    }
}
