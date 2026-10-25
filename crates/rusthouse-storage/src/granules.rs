use crate::{CompressedBlockReader, PartError, WideMark};
use std::io::{Read, Seek};

const MAX_GRANULE_BYTES: usize = 64 * 1024 * 1024;

pub fn read_mark_bytes<R: Read + Seek>(
    reader: &mut CompressedBlockReader<R>,
    mark: &WideMark,
    byte_count: usize,
) -> Result<Vec<u8>, PartError> {
    if byte_count > MAX_GRANULE_BYTES {
        return Err(invalid_mark(mark, "granule exceeds the 64 MiB read limit"));
    }
    if byte_count == 0 {
        return Ok(Vec::new());
    }

    let mut offset = usize::try_from(mark.decompressed_offset)
        .map_err(|_| invalid_mark(mark, "decompressed offset exceeds address space"))?;
    reader.seek_to_block(mark.compressed_offset)?;

    let mut result = Vec::with_capacity(byte_count);
    while result.len() < byte_count {
        let block = reader
            .next_decoded()
            .ok_or_else(|| invalid_mark(mark, "column stream ends before granule"))??;
        if offset > block.data.len() {
            return Err(invalid_mark(mark, "decompressed offset exceeds block size"));
        }
        let count = (byte_count - result.len()).min(block.data.len() - offset);
        result.extend_from_slice(&block.data[offset..offset + count]);
        offset = 0;
    }
    Ok(result)
}

pub fn read_u64_granule<R: Read + Seek>(
    reader: &mut CompressedBlockReader<R>,
    mark: &WideMark,
) -> Result<Vec<u64>, PartError> {
    let rows = mark
        .rows
        .ok_or_else(|| invalid_mark(mark, "UInt64 granules require adaptive marks"))?;
    if rows == 0 {
        return Ok(Vec::new());
    }
    let byte_count = usize::try_from(rows)
        .ok()
        .and_then(|rows| rows.checked_mul(size_of::<u64>()))
        .ok_or_else(|| invalid_mark(mark, "granule size overflows"))?;
    let bytes = read_mark_bytes(reader, mark, byte_count)?;
    Ok(bytes
        .chunks_exact(size_of::<u64>())
        .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

fn invalid_mark(mark: &WideMark, reason: &'static str) -> PartError {
    PartError::InvalidWideMark {
        compressed_offset: mark.compressed_offset,
        decompressed_offset: mark.decompressed_offset,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ch_cityhash102::cityhash128;
    use std::io::Cursor;

    fn checked_block(method: u8, data: &[u8]) -> Vec<u8> {
        let payload = if method == 0x82 {
            lz4_flex::block::compress(data)
        } else {
            data.to_vec()
        };
        let mut encoded = vec![method];
        encoded.extend_from_slice(&(9u32 + payload.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&(data.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&payload);
        let mut result = u128::from(cityhash128(&encoded)).to_le_bytes().to_vec();
        result.extend_from_slice(&encoded);
        result
    }

    fn mark(compressed_offset: u64, decompressed_offset: u64, rows: Option<u64>) -> WideMark {
        WideMark {
            compressed_offset,
            decompressed_offset,
            rows,
        }
    }

    fn reader(data: Vec<u8>) -> CompressedBlockReader<Cursor<Vec<u8>>> {
        CompressedBlockReader::new(Cursor::new(data), "values.bin").unwrap()
    }

    #[test]
    fn reads_selected_values_from_inside_a_block() {
        let values = [11u64, 22, 33, 44, 55, 66];
        let data: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let mut reader = reader(checked_block(0x02, &data));
        let selected_mark = mark(0, 16, Some(3));

        assert_eq!(
            read_u64_granule(&mut reader, &selected_mark).unwrap(),
            vec![33, 44, 55]
        );
        assert_eq!(
            read_mark_bytes(&mut reader, &selected_mark, 8).unwrap(),
            33u64.to_le_bytes()
        );
    }

    #[test]
    fn reads_values_crossing_lz4_and_uncompressed_blocks() {
        let values = [10u64, 20, 30, 40, 50];
        let data: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let mut blocks = checked_block(0x82, &data[..20]);
        let second_block_offset = blocks.len() as u64;
        blocks.extend(checked_block(0x02, &data[20..]));
        let mut reader = reader(blocks);

        assert_eq!(
            read_u64_granule(&mut reader, &mark(0, 8, Some(3))).unwrap(),
            vec![20, 30, 40]
        );
        assert_eq!(
            read_u64_granule(&mut reader, &mark(second_block_offset, 4, Some(1))).unwrap(),
            vec![40]
        );
        assert_eq!(
            read_u64_granule(&mut reader, &mark(0, 0, Some(1))).unwrap(),
            vec![10]
        );
    }

    #[test]
    fn does_not_read_beyond_requested_granule() {
        let mut blocks = checked_block(0x02, &12u64.to_le_bytes());
        let mut corrupt = checked_block(0x02, &34u64.to_le_bytes());
        corrupt[0] ^= 1;
        blocks.extend(corrupt);

        let mut reader = reader(blocks);
        assert_eq!(
            read_u64_granule(&mut reader, &mark(0, 0, Some(1))).unwrap(),
            vec![12]
        );
        assert!(matches!(
            read_u64_granule(&mut reader, &mark(0, 0, Some(2))),
            Err(PartError::InvalidCompressedBlock {
                reason: "checksum mismatch",
                ..
            })
        ));
    }

    #[test]
    fn rejects_marks_outside_a_decoded_block() {
        let mut reader = reader(checked_block(0x02, &12u64.to_le_bytes()));
        assert!(matches!(
            read_u64_granule(&mut reader, &mark(0, 9, Some(1))),
            Err(PartError::InvalidWideMark {
                reason: "decompressed offset exceeds block size",
                ..
            })
        ));
    }

    #[test]
    fn rejects_truncated_granules() {
        let mut reader = reader(checked_block(0x02, &12u64.to_le_bytes()));
        assert!(matches!(
            read_u64_granule(&mut reader, &mark(0, 0, Some(2))),
            Err(PartError::InvalidWideMark {
                reason: "column stream ends before granule",
                ..
            })
        ));
    }

    #[test]
    fn refuses_missing_or_unbounded_row_counts() {
        let mut reader = reader(checked_block(0x02, &12u64.to_le_bytes()));
        assert!(matches!(
            read_u64_granule(&mut reader, &mark(0, 0, None)),
            Err(PartError::InvalidWideMark {
                reason: "UInt64 granules require adaptive marks",
                ..
            })
        ));
        assert!(matches!(
            read_u64_granule(&mut reader, &mark(0, 0, Some(8 * 1024 * 1024 + 1))),
            Err(PartError::InvalidWideMark {
                reason: "granule exceeds the 64 MiB read limit",
                ..
            })
        ));
        assert_eq!(
            read_u64_granule(&mut reader, &mark(u64::MAX, 0, Some(0))).unwrap(),
            Vec::<u64>::new()
        );
    }

    #[test]
    fn rejects_nonexistent_block_offsets() {
        let blocks = checked_block(0x02, &12u64.to_le_bytes());
        let len = blocks.len() as u64;
        let mut reader = reader(blocks);
        assert!(matches!(
            read_u64_granule(&mut reader, &mark(len, 0, Some(1))),
            Err(PartError::InvalidCompressedBlock {
                reason: "block offset is outside file",
                ..
            })
        ));
    }
}
