use super::*;

pub(super) fn validate_stream_pairs(
    path: &Path,
    rows: u64,
    data_files: &BTreeSet<PathBuf>,
    mark_files: &BTreeMap<PathBuf, PathBuf>,
) -> Result<(), PartError> {
    if rows > 0 && data_files.is_empty() {
        return Err(invalid(path, "nonempty part has no wide data streams"));
    }
    if mark_files.keys().any(|data| !data_files.contains(data)) {
        return Err(invalid(path, "mark file has no matching data stream"));
    }
    if data_files.iter().any(|data| !mark_files.contains_key(data)) {
        return Err(invalid(path, "data stream has no matching marks"));
    }
    Ok(())
}

fn checked_decoded_bytes(total: u64, bytes: u64, path: &Path) -> Result<u64, PartError> {
    total
        .checked_add(bytes)
        .ok_or_else(|| invalid(path, "decoded byte count overflows"))
}

pub(super) fn account_decoded_blocks(
    data: &Path,
    report: &mut PartVerification,
) -> Result<u64, PartError> {
    let mut reader = CompressedBlockReader::open(data)?;
    let mut decoded_size = 0u64;
    // Verification consumes every block. Return the decoded allocation to the
    // reader rather than allocating a new payload vector for each frame.
    let mut scratch = Vec::new();
    while let Some(block) = reader.next_decoded_reusing(scratch) {
        let block = block?;
        report.blocks = report
            .blocks
            .checked_add(1)
            .ok_or_else(|| invalid(data, "block count overflows"))?;
        let bytes = block.data.len() as u64;
        decoded_size = checked_decoded_bytes(decoded_size, bytes, data)?;
        report.decompressed_bytes = checked_decoded_bytes(report.decompressed_bytes, bytes, data)?;
        scratch = block.data;
    }
    Ok(decoded_size)
}

pub(super) fn validate_adaptive_stream(
    data: &Path,
    marks: &WideMarks,
    expected_rows: u64,
    layout: &mut ScanLayout,
) -> Result<(), PartError> {
    if marks.format != WideMarkFormat::Adaptive {
        return Ok(());
    }
    let name = data
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("stream");
    layout
        .validate(name, &marks.marks, expected_rows)
        .map_err(|error| {
            if matches!(&error, PartError::InvalidScan(message)
                if message.starts_with("mark row counts do not match count.txt"))
            {
                return invalid(data, "mark row counts do not match count.txt");
            }
            // Preserve the old error precedence on malformed streams. A
            // boundary mismatch can be found before the layout has counted
            // all marks. Retry the row sum only on this error path; valid
            // streams never traverse their marks twice.
            let total_rows = marks.marks.iter().try_fold(0u64, |total, mark| {
                total.checked_add(mark.rows.unwrap_or(0))
            });
            if total_rows != Some(expected_rows) {
                invalid(data, "mark row counts do not match count.txt")
            } else {
                invalid(data, "adaptive streams have mismatched granule boundaries")
            }
        })
}

pub(super) fn collect_wide_file_pairs(
    path: &Path,
) -> Result<(BTreeSet<PathBuf>, BTreeMap<PathBuf, PathBuf>), PartError> {
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

    Ok((data_files, mark_files))
}

#[cfg(test)]
mod reused_verification_decode_tests {
    use super::*;
    use ch_cityhash102::cityhash128;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn reused_block_payloads_count_every_uncompressed_byte() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-reused-verify-{}-{}.bin",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let payloads = [vec![1u8; 8192], vec![2u8; 16_384], vec![3u8; 17]];
        let mut bytes = Vec::new();
        for payload in &payloads {
            let mut frame = vec![0u8; 16];
            frame.push(0x02);
            frame.extend_from_slice(&(9u32 + payload.len() as u32).to_le_bytes());
            frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            frame.extend_from_slice(payload);
            let checksum = u128::from(cityhash128(&frame[16..])).to_le_bytes();
            frame[..16].copy_from_slice(&checksum);
            bytes.extend_from_slice(&frame);
        }
        std::fs::write(&path, bytes).unwrap();
        let mut report = PartVerification {
            streams: 0,
            marks: 0,
            blocks: 0,
            decompressed_bytes: 0,
        };
        let result = account_decoded_blocks(&path, &mut report);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.unwrap(), 8192 + 16_384 + 17);
        assert_eq!(report.blocks, 3);
        assert_eq!(report.decompressed_bytes, 8192 + 16_384 + 17);
    }
}

#[cfg(test)]
mod adaptive_stream_single_pass_tests {
    use super::*;
    use crate::WideMark;

    fn marks(rows: &[u64]) -> WideMarks {
        WideMarks {
            path: "values.mrk2".into(),
            format: WideMarkFormat::Adaptive,
            marks: rows
                .iter()
                .enumerate()
                .map(|(index, &rows)| WideMark {
                    compressed_offset: index as u64 * 16,
                    decompressed_offset: 0,
                    rows: Some(rows),
                })
                .collect(),
        }
    }

    #[test]
    fn detects_total_row_mismatches_and_overflow() {
        let mut layout = ScanLayout::default();
        validate_adaptive_stream(Path::new("a.bin"), &marks(&[2, 3]), 5, &mut layout).unwrap();
        let mismatch =
            validate_adaptive_stream(Path::new("b.bin"), &marks(&[2, 4]), 5, &mut layout)
                .unwrap_err();
        assert!(matches!(
            mismatch,
            PartError::InvalidPart {
                reason: "mark row counts do not match count.txt",
                ..
            }
        ));
        let overflow = validate_adaptive_stream(
            Path::new("c.bin"),
            &marks(&[u64::MAX, 1]),
            u64::MAX,
            &mut ScanLayout::default(),
        )
        .unwrap_err();
        assert!(matches!(
            overflow,
            PartError::InvalidPart {
                reason: "mark row counts do not match count.txt",
                ..
            }
        ));
    }

    #[test]
    fn validates_shared_granules_after_first_column() {
        let mut layout = ScanLayout::default();
        validate_adaptive_stream(Path::new("a.bin"), &marks(&[2, 3]), 5, &mut layout).unwrap();
        validate_adaptive_stream(Path::new("b.bin"), &marks(&[2, 3]), 5, &mut layout).unwrap();
        assert!(
            validate_adaptive_stream(Path::new("c.bin"), &marks(&[3, 2]), 5, &mut layout).is_err()
        );
    }
}
