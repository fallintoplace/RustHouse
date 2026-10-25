use crate::{
    CompressedBlockReader, NumericColumn, NumericReadError, PartError, WideMark,
    read_numeric_column,
};
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
        if offset >= block.data.len() {
            return Err(invalid_mark(mark, "decompressed offset is outside block"));
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

pub fn read_numeric_granule<R: Read + Seek>(
    reader: &mut CompressedBlockReader<R>,
    mark: &WideMark,
    data_type: &str,
) -> Result<NumericColumn, PartError> {
    let width = match data_type {
        "UInt8" | "Int8" => 1,
        "UInt16" | "Int16" => 2,
        "UInt32" | "Int32" | "Float32" => 4,
        "UInt64" | "Int64" | "Float64" => 8,
        _ => {
            return Err(PartError::NumericRead(NumericReadError::UnsupportedType(
                data_type.to_owned(),
            )));
        }
    };
    let rows = mark
        .rows
        .ok_or_else(|| invalid_mark(mark, "numeric granules require adaptive marks"))?;
    let rows = usize::try_from(rows).map_err(|_| invalid_mark(mark, "granule size overflows"))?;
    let byte_count = rows
        .checked_mul(width)
        .ok_or_else(|| invalid_mark(mark, "granule size overflows"))?;
    let bytes = read_mark_bytes(reader, mark, byte_count)?;
    read_numeric_column(&mut bytes.as_slice(), data_type, rows).map_err(PartError::NumericRead)
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
                reason: "decompressed offset is outside block",
                ..
            })
        ));
    }

    #[test]
    fn rejects_mark_at_end_of_nonempty_block() {
        let mut bytes = checked_block(0x02, &12u64.to_le_bytes());
        bytes.extend(checked_block(0x02, &34u64.to_le_bytes()));
        let mut reader = reader(bytes);
        assert!(matches!(
            read_u64_granule(&mut reader, &mark(0, 8, Some(1))),
            Err(PartError::InvalidWideMark {
                reason: "decompressed offset is outside block",
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
    fn reads_all_plain_numeric_types_from_adaptive_marks() {
        let cases = vec![
            ("UInt8", vec![0, 255], NumericColumn::UInt8(vec![0, 255])),
            ("Int8", vec![0, 255], NumericColumn::Int8(vec![0, -1])),
            (
                "UInt16",
                vec![0x34, 0x12, 0xff, 0xff],
                NumericColumn::UInt16(vec![0x1234, u16::MAX]),
            ),
            (
                "Int16",
                vec![0, 0x80, 0xff, 0x7f],
                NumericColumn::Int16(vec![i16::MIN, i16::MAX]),
            ),
            (
                "UInt32",
                [0x1234_5678u32, u32::MAX]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect(),
                NumericColumn::UInt32(vec![0x1234_5678, u32::MAX]),
            ),
            (
                "Int32",
                [i32::MIN, 1_000]
                    .into_iter()
                    .flat_map(i32::to_le_bytes)
                    .collect(),
                NumericColumn::Int32(vec![i32::MIN, 1_000]),
            ),
            (
                "UInt64",
                [0x0102_0304_0506_0708u64, u64::MAX]
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect(),
                NumericColumn::UInt64(vec![0x0102_0304_0506_0708, u64::MAX]),
            ),
            (
                "Int64",
                [i64::MIN, i64::MAX]
                    .into_iter()
                    .flat_map(i64::to_le_bytes)
                    .collect(),
                NumericColumn::Int64(vec![i64::MIN, i64::MAX]),
            ),
            (
                "Float32",
                [-0.0f32, 1.5f32]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
                    .collect(),
                NumericColumn::Float32(vec![-0.0, 1.5]),
            ),
            (
                "Float64",
                [-0.0f64, 1.5f64]
                    .into_iter()
                    .flat_map(f64::to_le_bytes)
                    .collect(),
                NumericColumn::Float64(vec![-0.0, 1.5]),
            ),
        ];
        for (data_type, data, expected) in cases {
            let mut blocks = reader(checked_block(0x02, &data));
            assert_eq!(
                read_numeric_granule(&mut blocks, &mark(0, 0, Some(2)), data_type).unwrap(),
                expected,
                "{data_type}"
            );
        }
    }

    #[test]
    fn reads_signed_values_across_lz4_and_plain_blocks() {
        let bytes: Vec<u8> = [-10i32, 20, 30, 40]
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect();
        let mut encoded = checked_block(0x82, &bytes[..7]);
        encoded.extend(checked_block(0x02, &bytes[7..]));
        let mut blocks = reader(encoded);

        assert_eq!(
            read_numeric_granule(&mut blocks, &mark(0, 4, Some(3)), "Int32").unwrap(),
            NumericColumn::Int32(vec![20, 30, 40])
        );
    }

    #[test]
    fn preserves_float_bits_in_marked_numeric_values() {
        let expected = [0x8000_0000u32, 0x7fc0_1234u32];
        let bytes: Vec<u8> = expected.into_iter().flat_map(u32::to_le_bytes).collect();
        let mut blocks = reader(checked_block(0x02, &bytes));
        let NumericColumn::Float32(values) =
            read_numeric_granule(&mut blocks, &mark(0, 0, Some(2)), "Float32").unwrap()
        else {
            panic!("unexpected numeric type");
        };
        assert_eq!(values[0].to_bits(), expected[0]);
        assert_eq!(values[1].to_bits(), expected[1]);
    }

    #[test]
    fn validates_type_and_size_before_reading_blocks() {
        let mut blocks = reader(Vec::new());
        assert!(matches!(
            read_numeric_granule(&mut blocks, &mark(0, 0, Some(1)), "Nullable(UInt8)"),
            Err(PartError::NumericRead(NumericReadError::UnsupportedType(_)))
        ));
        assert!(matches!(
            read_numeric_granule(&mut blocks, &mark(0, 0, None), "Int16"),
            Err(PartError::InvalidWideMark {
                reason: "numeric granules require adaptive marks",
                ..
            })
        ));
        assert!(matches!(
            read_numeric_granule(
                &mut blocks,
                &mark(0, 0, Some(MAX_GRANULE_BYTES as u64 / 2 + 1)),
                "Int16"
            ),
            Err(PartError::InvalidWideMark {
                reason: "granule exceeds the 64 MiB read limit",
                ..
            })
        ));
        assert!(matches!(
            read_numeric_granule(&mut blocks, &mark(0, 0, Some(u64::MAX)), "Int64"),
            Err(PartError::InvalidWideMark {
                reason: "granule size overflows",
                ..
            })
        ));
        assert_eq!(
            read_numeric_granule(&mut blocks, &mark(u64::MAX, 0, Some(0)), "Float64").unwrap(),
            NumericColumn::Float64(vec![])
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
