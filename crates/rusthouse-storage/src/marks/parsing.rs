use super::*;

#[cfg(test)]
pub(super) fn parse_wide_marks(
    content: &[u8],
    format: WideMarkFormat,
    path: &Path,
) -> Result<Vec<WideMark>, PartError> {
    if content.is_empty() {
        return Err(PartError::EmptyMarksFile(path.to_path_buf()));
    }

    let record_size = format.record_size();
    if content.len() % record_size != 0 {
        return Err(PartError::InvalidMarksLength {
            path: path.to_path_buf(),
            size: content.len(),
            record_size,
        });
    }

    Ok(content
        .chunks_exact(record_size)
        .map(|record| parse_mark_record(record, format))
        .collect())
}

pub(super) fn parse_mark_record(record: &[u8], format: WideMarkFormat) -> WideMark {
    WideMark {
        compressed_offset: read_u64(&record[..8]),
        decompressed_offset: read_u64(&record[8..16]),
        rows: (format == WideMarkFormat::Adaptive).then(|| read_u64(&record[16..24])),
    }
}

pub(super) fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("eight-byte mark word"))
}
