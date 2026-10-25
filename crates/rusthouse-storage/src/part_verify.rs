use crate::{
    CompressedBlockReader, PartError, WideMarkFormat, inspect_part, read_wide_marks,
    validate_wide_marks,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartVerification {
    pub streams: usize,
    pub marks: usize,
    pub blocks: u64,
    pub decompressed_bytes: u64,
}

pub fn verify_wide_part(path: impl AsRef<Path>) -> Result<PartVerification, PartError> {
    let path = path.as_ref();
    let metadata = inspect_part(path)?;
    let mut data_files = BTreeSet::new();
    let mut mark_files = BTreeMap::new();
    let entries = fs::read_dir(path).map_err(|source| io_error(path, source))?;

    for entry in entries {
        let entry = entry.map_err(|source| io_error(path, source))?;
        let filename = entry.path();
        let extension = filename
            .extension()
            .and_then(|extension| extension.to_str());
        let is_data = extension == Some("bin");
        let is_mark = matches!(extension, Some("mrk" | "mrk2" | "cmrk" | "cmrk2"));
        if matches!(extension, Some("mrk3" | "mrk4" | "cmrk3" | "cmrk4")) {
            return Err(PartError::UnsupportedMarksFormat(filename));
        }
        if !is_data && !is_mark {
            continue;
        }
        if !entry
            .file_type()
            .map_err(|source| io_error(&filename, source))?
            .is_file()
        {
            return Err(invalid(
                &filename,
                "expected a regular stream or marks file",
            ));
        }
        if is_data {
            data_files.insert(filename);
        } else {
            let data = filename.with_extension("bin");
            if mark_files.insert(data, filename.clone()).is_some() {
                return Err(invalid(&filename, "multiple mark files for one stream"));
            }
        }
    }

    if metadata.rows > 0 && data_files.is_empty() {
        return Err(invalid(path, "nonempty part has no wide data streams"));
    }
    if mark_files.keys().any(|data| !data_files.contains(data)) {
        return Err(invalid(path, "mark file has no matching data stream"));
    }
    if data_files.iter().any(|data| !mark_files.contains_key(data)) {
        return Err(invalid(path, "data stream has no matching marks"));
    }

    let mut report = PartVerification {
        streams: 0,
        marks: 0,
        blocks: 0,
        decompressed_bytes: 0,
    };
    for data in data_files {
        let marks_path = mark_files.remove(&data).unwrap();
        let marks = read_wide_marks(marks_path)?;
        if marks.format == WideMarkFormat::Adaptive {
            let rows = marks.marks.iter().try_fold(0u64, |total, mark| {
                total.checked_add(mark.rows.unwrap_or(0))
            });
            if rows != Some(metadata.rows) {
                return Err(invalid(&data, "mark row counts do not match count.txt"));
            }
        }
        validate_wide_marks(&marks, CompressedBlockReader::open(&data)?)?;

        let mut reader = CompressedBlockReader::open(&data)?;
        while let Some(block) = reader.next_decoded() {
            let block = block?;
            report.blocks = report
                .blocks
                .checked_add(1)
                .ok_or_else(|| invalid(&data, "block count overflows"))?;
            report.decompressed_bytes = report
                .decompressed_bytes
                .checked_add(block.data.len() as u64)
                .ok_or_else(|| invalid(&data, "decoded byte count overflows"))?;
        }
        report.marks = report
            .marks
            .checked_add(marks.marks.len())
            .ok_or_else(|| invalid(&data, "mark count overflows"))?;
        report.streams += 1;
    }
    Ok(report)
}

fn invalid(path: &Path, reason: &'static str) -> PartError {
    PartError::InvalidPart {
        path: path.to_path_buf(),
        reason,
    }
}

fn io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ch_cityhash102::cityhash128;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    fn fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-verify-part-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("count.txt"), "2\n").unwrap();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n1 columns:\n`id` UInt64\n",
        )
        .unwrap();
        let bytes: Vec<_> = [10u64, 20].into_iter().flat_map(u64::to_le_bytes).collect();
        let mut encoded = vec![0x02];
        encoded.extend_from_slice(&(9 + bytes.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&bytes);
        let mut block = u128::from(cityhash128(&encoded)).to_le_bytes().to_vec();
        block.extend_from_slice(&encoded);
        fs::write(dir.join("id.bin"), block).unwrap();
        let mut mark = Vec::new();
        mark.extend_from_slice(&0u64.to_le_bytes());
        mark.extend_from_slice(&0u64.to_le_bytes());
        mark.extend_from_slice(&2u64.to_le_bytes());
        fs::write(dir.join("id.mrk2"), mark).unwrap();
        dir
    }

    #[test]
    fn verifies_stream_blocks_and_adaptive_mark_offsets() {
        let dir = fixture();
        assert_eq!(
            verify_wide_part(&dir).unwrap(),
            PartVerification {
                streams: 1,
                marks: 1,
                blocks: 1,
                decompressed_bytes: 16,
            }
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_corrupt_block_and_invalid_mark_offsets() {
        let dir = fixture();
        let path = dir.join("id.bin");
        let mut block = fs::read(&path).unwrap();
        block[25] ^= 1;
        fs::write(&path, block).unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidCompressedBlock {
                reason: "checksum mismatch",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();

        let dir = fixture();
        let path = dir.join("id.mrk2");
        let mut marks = fs::read(&path).unwrap();
        marks[..8].copy_from_slice(&1u64.to_le_bytes());
        fs::write(&path, marks).unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidMarkOffset { .. })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_mismatched_rows_and_missing_or_duplicate_pairs() {
        let dir = fixture();
        fs::write(dir.join("count.txt"), "3\n").unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidPart {
                reason: "mark row counts do not match count.txt",
                ..
            })
        ));
        fs::write(dir.join("count.txt"), "2\n").unwrap();

        fs::write(dir.join("id.cmrk2"), [1u8, 2]).unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidPart {
                reason: "multiple mark files for one stream",
                ..
            })
        ));
        fs::remove_file(dir.join("id.cmrk2")).unwrap();

        fs::remove_file(dir.join("id.mrk2")).unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidPart {
                reason: "data stream has no matching marks",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_part_without_streams_is_valid() {
        let dir = fixture();
        fs::write(dir.join("count.txt"), "0\n").unwrap();
        fs::remove_file(dir.join("id.bin")).unwrap();
        fs::remove_file(dir.join("id.mrk2")).unwrap();
        assert_eq!(
            verify_wide_part(&dir).unwrap(),
            PartVerification {
                streams: 0,
                marks: 0,
                blocks: 0,
                decompressed_bytes: 0,
            }
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
