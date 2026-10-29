use crate::{CompressedBlockReader, PartError, WideMark, WideMarks};
use std::io::{Read, Seek};

pub fn validate_wide_marks<R: Read + Seek>(
    marks: &WideMarks,
    mut reader: CompressedBlockReader<R>,
) -> Result<(), PartError> {
    validate_wide_marks_reader(marks, &mut reader)
}

pub(crate) fn validate_wide_marks_reader<R: Read + Seek>(
    marks: &WideMarks,
    reader: &mut CompressedBlockReader<R>,
) -> Result<(), PartError> {
    if marks.marks.is_empty() {
        return Err(PartError::EmptyMarksFile(marks.path.clone()));
    }

    let mut block = reader.next().transpose()?;
    let mut preceding_end = 0u64;
    let mut previous: Option<&WideMark> = None;

    for (index, mark) in marks.marks.iter().enumerate() {
        validate_mark_sequence(marks, index, mark, previous)?;

        while let Some(current) = block {
            if current.offset >= mark.compressed_offset {
                break;
            }
            preceding_end = current.offset + 16 + u64::from(current.compressed_size);
            block = reader.next().transpose()?;
        }

        validate_wide_mark_target(marks, index, mark, block, preceding_end)?;
        previous = Some(mark);
    }

    while block.is_some() {
        block = reader.next().transpose()?;
    }
    Ok(())
}

fn validate_wide_mark_target(
    marks: &WideMarks,
    index: usize,
    mark: &WideMark,
    block: Option<crate::CompressedBlock>,
    preceding_end: u64,
) -> Result<(), PartError> {
    let terminal = mark.rows == Some(0);
    match block {
        Some(current) if current.offset == mark.compressed_offset => {
            let size = u64::from(current.decompressed_size);
            if mark.decompressed_offset > size || (!terminal && mark.decompressed_offset == size) {
                return Err(invalid_mark(
                    marks,
                    index,
                    "decompressed offset is outside the block",
                ));
            }
        }
        None if terminal
            && mark.compressed_offset == preceding_end
            && mark.decompressed_offset == 0 => {}
        _ => {
            return Err(invalid_mark(
                marks,
                index,
                "mark does not point to a compressed block",
            ));
        }
    }
    Ok(())
}

fn mark_offset(mark: &WideMark) -> (u64, u64) {
    (mark.compressed_offset, mark.decompressed_offset)
}

fn validate_mark_sequence(
    marks: &WideMarks,
    index: usize,
    mark: &WideMark,
    previous: Option<&WideMark>,
) -> Result<(), PartError> {
    if mark.rows == Some(0) && index + 1 != marks.marks.len() {
        return Err(invalid_mark(marks, index, "zero-row mark must be last"));
    }
    if let Some(previous) = previous {
        let current_offset = mark_offset(mark);
        let previous_offset = mark_offset(previous);
        if current_offset < previous_offset {
            return Err(invalid_mark(marks, index, "mark offsets are not ordered"));
        }
        if current_offset == previous_offset && mark.rows != Some(0) {
            return Err(invalid_mark(marks, index, "nonterminal mark must advance"));
        }
    }
    Ok(())
}

fn invalid_mark(marks: &WideMarks, index: usize, reason: &'static str) -> PartError {
    PartError::InvalidMarkOffset {
        path: marks.path.clone(),
        index,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WideMarkFormat;
    use std::io::Cursor;
    use std::path::PathBuf;

    fn block(decompressed_size: u32) -> Vec<u8> {
        let mut bytes = vec![0; 16];
        bytes.push(0x82);
        bytes.extend_from_slice(&10u32.to_le_bytes());
        bytes.extend_from_slice(&decompressed_size.to_le_bytes());
        bytes.push(0);
        bytes
    }

    fn mark(compressed_offset: u64, decompressed_offset: u64, rows: Option<u64>) -> WideMark {
        WideMark {
            compressed_offset,
            decompressed_offset,
            rows,
        }
    }

    fn check(
        marks: Vec<WideMark>,
        format: WideMarkFormat,
        bytes: Vec<u8>,
    ) -> Result<(), PartError> {
        let marks = WideMarks {
            path: PathBuf::from("column.mrk2"),
            format,
            marks,
        };
        let reader = CompressedBlockReader::new(Cursor::new(bytes), "column.bin").unwrap();
        validate_wide_marks(&marks, reader)
    }

    #[test]
    fn accepts_multiple_marks_per_block_and_final_mark_inside_block() {
        let mut bytes = block(16);
        let second_offset = bytes.len() as u64;
        bytes.extend(block(7));

        check(
            vec![
                mark(0, 0, Some(12)),
                mark(0, 12, Some(4)),
                mark(second_offset, 0, Some(7)),
                mark(second_offset, 7, Some(0)),
            ],
            WideMarkFormat::Adaptive,
            bytes,
        )
        .unwrap();
    }

    #[test]
    fn accepts_end_of_file_terminal_mark() {
        let bytes = block(16);
        let end = bytes.len() as u64;
        check(
            vec![mark(0, 0, Some(16)), mark(end, 0, Some(0))],
            WideMarkFormat::Adaptive,
            bytes,
        )
        .unwrap();
    }

    #[test]
    fn accepts_fixed_granularity_marks() {
        let mut bytes = block(16);
        let offset = bytes.len() as u64;
        bytes.extend(block(6));
        check(
            vec![mark(0, 0, None), mark(offset, 2, None)],
            WideMarkFormat::Fixed,
            bytes,
        )
        .unwrap();
    }

    #[test]
    fn rejects_marks_inside_compressed_blocks() {
        let bytes = block(16);
        assert!(matches!(
            check(vec![mark(2, 0, Some(1))], WideMarkFormat::Adaptive, bytes,),
            Err(PartError::InvalidMarkOffset { index: 0, .. })
        ));
    }

    #[test]
    fn rejects_invalid_decompressed_offsets() {
        for rows in [Some(1), Some(0)] {
            let expected_offset = if rows == Some(0) { 17 } else { 16 };
            assert!(matches!(
                check(
                    vec![mark(0, expected_offset, rows)],
                    WideMarkFormat::Adaptive,
                    block(16),
                ),
                Err(PartError::InvalidMarkOffset { index: 0, .. })
            ));
        }
    }

    #[test]
    fn rejects_nonterminal_zero_row_marks() {
        assert!(matches!(
            check(
                vec![mark(0, 0, Some(0)), mark(0, 0, Some(3))],
                WideMarkFormat::Adaptive,
                block(16),
            ),
            Err(PartError::InvalidMarkOffset { index: 0, .. })
        ));
    }

    #[test]
    fn rejects_repeated_nonterminal_marks_but_allows_zero_row_terminal() {
        let bytes = block(16);
        assert!(matches!(
            check(
                vec![mark(0, 0, Some(1)), mark(0, 0, Some(1))],
                WideMarkFormat::Adaptive,
                bytes.clone(),
            ),
            Err(PartError::InvalidMarkOffset { index: 1, .. })
        ));
        check(
            vec![mark(0, 0, Some(1)), mark(0, 0, Some(0))],
            WideMarkFormat::Adaptive,
            bytes,
        )
        .unwrap();
    }

    #[test]
    fn rejects_backwards_offsets_and_offsets_past_eof() {
        for marks in [
            vec![mark(0, 8, Some(1)), mark(0, 7, Some(1))],
            vec![mark(0, 0, Some(1)), mark(300, 0, Some(1))],
        ] {
            assert!(matches!(
                check(marks, WideMarkFormat::Adaptive, block(16)),
                Err(PartError::InvalidMarkOffset { index: 1, .. })
            ));
        }
    }

    #[test]
    fn rejects_trailing_invalid_block_even_when_marks_are_valid() {
        let mut bytes = block(16);
        bytes.extend_from_slice(&[1, 2, 3]);
        assert!(matches!(
            check(vec![mark(0, 0, Some(16))], WideMarkFormat::Adaptive, bytes,),
            Err(PartError::InvalidCompressedBlock { .. })
        ));
    }

    #[test]
    fn rejects_empty_mark_lists() {
        assert!(matches!(
            check(Vec::new(), WideMarkFormat::Adaptive, block(16)),
            Err(PartError::EmptyMarksFile(_))
        ));
    }
}
