use super::*;

pub(super) fn numeric_marks_path(part: &Path, name: &str) -> Result<PathBuf, PartError> {
    let plain_marks = part.join(format!("{name}.mrk2"));
    let compressed_marks = part.join(format!("{name}.cmrk2"));
    match (plain_marks.exists(), compressed_marks.exists()) {
        (true, true) => Err(PartError::InvalidScan(format!(
            "multiple mark files for {name}"
        ))),
        (_, true) => Ok(compressed_marks),
        _ => Ok(plain_marks),
    }
}

pub(super) fn open_numeric_stream(
    part: &Path,
    name: &str,
    data_type: &str,
) -> Result<ColumnStream, PartError> {
    let marks_path = numeric_marks_path(part, name)?;
    let marks_file = read_wide_marks(&marks_path)?;
    let data_path = part.join(format!("{name}.bin"));
    let mut reader = CompressedBlockReader::open(&data_path)?;
    crate::mark_validation::validate_wide_marks_reader(&marks_file, &mut reader)?;
    // Mark validation leaves the reader at EOF. Rewind the verified
    // file instead of reopening it for the actual granule scan.
    reader.seek_to_block(0)?;
    Ok(ColumnStream {
        data_type: data_type.to_owned(),
        marks: marks_file.marks,
        reader,
        cached: None,
    })
}
