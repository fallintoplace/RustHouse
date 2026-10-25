use crate::{CompressedBlock, CompressedBlockReader, PartError, WideMark, read_wide_marks};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkBlockSummary {
    pub mark_count: usize,
    pub block_count: usize,
}

pub fn check_wide_mark_offsets(
    marks_path: impl AsRef<Path>,
    data_path: impl AsRef<Path>,
) -> Result<MarkBlockSummary, PartError> {
    let marks_path = marks_path.as_ref();
    let data_path = data_path.as_ref();
    let marks = read_wide_marks(marks_path)?;
    let reader = CompressedBlockReader::open(data_path)?;
    let file_size = fs::metadata(data_path)
        .map_err(|source| PartError::Io {
            path: data_path.to_path_buf(),
            source,
        })?
        .len();
    let block_count = check_positions(&marks.marks, reader, file_size, marks_path)?;
    Ok(MarkBlockSummary {
        mark_count: marks.marks.len(),
        block_count,
    })
}

fn invalid_mark(
    path: &Path,
    index: usize,
    mark: &WideMark,
    reason: &'static str,
) -> PartError {
    PartError::InvalidMarkOffset {
        path: path.to_path_buf(),
        index,
        compressed_offset: mark.compressed_offset,
        decompressed_offset: mark.decompressed_offset,
        reason,
    }
}

fn check_positions(
    marks: &[WideMark],
    mut blocks: impl Iterator<Item = Result<CompressedBlock, PartError>>,
    file_size: u64,
    marks_path: &Path,
) -> Result<usize, PartError> {
    let mut next_block = blocks.next().transpose()?;
    let mut block_count = usize::from(next_block.is_some());
    let mut previous = None;

    for (index, mark) in marks.iter().enumerate() {
        let position = (mark.compressed_offset, mark.decompressed_offset);
        if previous.is_some_and(|prior| position < prior) {
            return Err(invalid_mark(
                marks_path,
                index,
                mark,
                "mark positions are not ordered",
            ));
        }
        previous = Some(position);

        if mark.compressed_offset > file_size {
            return Err(invalid_mark(
                marks_path,
                index,
                mark,
                "compressed offset exceeds file size",
            ));
        }

        while let Some(block) = next_block {
            if block.offset >= mark.compressed_offset {
                break;
            }
            next_block = blocks.next().transpose()?;
            block_count += usize::from(next_block.is_some());
        }

        if mark.compressed_offset == file_size {
            if mark.decompressed_offset != 0 {
                return Err(invalid_mark(
                    marks_path,
                    index,
                    mark,
                    "end-of-file mark has a nonzero decompressed offset",
                ));
            }
            continue;
        }

        let Some(block) = next_block else {
            return Err(invalid_mark(
                marks_path,
                index,
                mark,
                "mark does not point to a compressed block",
            ));
        };
        if block.offset != mark.compressed_offset {
            return Err(invalid_mark(
                marks_path,
                index,
                mark,
                "compressed offset is not a block boundary",
            ));
        }
        if mark.decompressed_offset > u64::from(block.decompressed_size) {
            return Err(invalid_mark(
                marks_path,
                index,
                mark,
                "decompressed offset exceeds block size",
            ));
        }
    }

    for block in blocks {
        block?;
        block_count += 1;
    }
    Ok(block_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(compressed_offset: u64, decompressed_offset: u64) -> WideMark {
        WideMark {
            compressed_offset,
            decompressed_offset,
            rows: Some(10),
        }
    }

    fn block(offset: u64, compressed_size: u32, decompressed_size: u32) -> CompressedBlock {
        CompressedBlock {
            offset,
            method: 0x82,
            compressed_size,
            decompressed_size,
        }
    }

    fn check(marks: &[WideMark], blocks: &[CompressedBlock], file_size: u64) -> Result<usize, PartError> {
        check_positions(
            marks,
            blocks.iter().copied().map(Ok),
            file_size,
            Path::new("id.mrk2"),
        )
    }

    #[test]
    fn accepts_repeated_marks_block_boundaries_and_final_mark() {
        let blocks = [block(0, 13, 100), block(29, 10, 80)];
        let marks = [
            mark(0, 0),
            mark(0, 100),
            mark(0, 100),
            mark(29, 0),
            mark(29, 80),
            WideMark {
                rows: Some(0),
                ..mark(55, 0)
            },
        ];
        assert_eq!(check(&marks, &blocks, 55).unwrap(), 2);
    }

    #[test]
    fn accepts_empty_column_stream_with_zero_marks() {
        assert_eq!(check(&[mark(0, 0), mark(0, 0)], &[], 0).unwrap(), 0);
    }

    #[test]
    fn rejects_interior_compressed_offsets() {
        let error = check(&[mark(15, 0)], &[block(0, 13, 100), block(29, 10, 80)], 55)
            .unwrap_err();
        assert!(matches!(
            error,
            PartError::InvalidMarkOffset {
                index: 0,
                reason: "compressed offset is not a block boundary",
                ..
            }
        ));
    }

    #[test]
    fn rejects_offsets_outside_decompressed_blocks() {
        let error = check(&[mark(0, 101)], &[block(0, 13, 100)], 29).unwrap_err();
        assert!(matches!(
            error,
            PartError::InvalidMarkOffset {
                reason: "decompressed offset exceeds block size",
                ..
            }
        ));
    }

    #[test]
    fn rejects_invalid_eof_and_out_of_file_marks() {
        for (offset, decompressed, reason) in [
            (29, 1, "end-of-file mark has a nonzero decompressed offset"),
            (30, 0, "compressed offset exceeds file size"),
        ] {
            let error = check(&[mark(offset, decompressed)], &[block(0, 13, 100)], 29)
                .unwrap_err();
            assert!(matches!(
                error,
                PartError::InvalidMarkOffset { reason: actual, .. } if actual == reason
            ));
        }
    }

    #[test]
    fn rejects_backwards_marks_even_across_blocks() {
        let error = check(
            &[mark(29, 0), mark(0, 100)],
            &[block(0, 13, 100), block(29, 10, 80)],
            55,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            PartError::InvalidMarkOffset {
                index: 1,
                reason: "mark positions are not ordered",
                ..
            }
        ));
    }

    #[test]
    fn scans_trailing_blocks_even_after_last_mark() {
        let error = check_positions(
            &[mark(0, 0)],
            [
                Ok(block(0, 13, 100)),
                Err(PartError::InvalidCompressedBlock {
                    path: "id.bin".into(),
                    offset: 29,
                    reason: "truncated checksum or block header",
                }),
            ]
            .into_iter(),
            30,
            Path::new("id.mrk2"),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            PartError::InvalidCompressedBlock { offset: 29, .. }
        ));
    }

    #[test]
    fn reads_real_mark_and_block_headers() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-mark-blocks-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let marks_path = dir.join("id.mrk2");
        let data_path = dir.join("id.bin");

        let mut marks = Vec::new();
        for (compressed, decompressed, rows) in [(0u64, 0u64, 10u64), (0, 100, 0)] {
            marks.extend_from_slice(&compressed.to_le_bytes());
            marks.extend_from_slice(&decompressed.to_le_bytes());
            marks.extend_from_slice(&rows.to_le_bytes());
        }
        fs::write(&marks_path, marks).unwrap();

        let mut data = vec![0; 16];
        data.push(0x82);
        data.extend_from_slice(&13u32.to_le_bytes());
        data.extend_from_slice(&100u32.to_le_bytes());
        data.extend_from_slice(&[1, 2, 3, 4]);
        fs::write(&data_path, data).unwrap();

        assert_eq!(
            check_wide_mark_offsets(&marks_path, &data_path).unwrap(),
            MarkBlockSummary {
                mark_count: 2,
                block_count: 1,
            }
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
