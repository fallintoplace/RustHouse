//! Validated opening of wide-part column marks and compressed data.
//! Numeric and typed readers must follow the same symlink and identity rules.
use crate::{
    CompressedBlockReader, PartError, WideMark, read_wide_marks,
};
use std::fs::File;
use std::path::{Path, PathBuf};

// Path::exists follows symlinks and converts permission errors into false.
// A broken link or inaccessible competing marks file must not be ignored.
fn present_regular_marks_file(path: &Path) -> Result<bool, PartError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(PartError::InvalidScan(format!(
            "marks path is not a regular file: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(PartError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub(crate) fn marks_path(part: &Path, name: &str) -> Result<PathBuf, PartError> {
    let plain_marks = part.join(format!("{name}.mrk2"));
    let compressed_marks = part.join(format!("{name}.cmrk2"));
    let plain = present_regular_marks_file(&plain_marks)?;
    let compressed = present_regular_marks_file(&compressed_marks)?;
    match (plain, compressed) {
        (true, true) => Err(PartError::InvalidScan(format!(
            "multiple mark files for {name}"
        ))),
        (_, true) => Ok(compressed_marks),
        _ => Ok(plain_marks),
    }
}

/// Open and rewind a validated column stream without reopening data.bin.
pub(crate) fn open_validated_column(
    part: &Path,
    name: &str,
) -> Result<(Vec<WideMark>, CompressedBlockReader<File>), PartError> {
    let marks_path = marks_path(part, name)?;
    let marks_file = read_wide_marks(&marks_path)?;
    let data_path = part.join(format!("{name}.bin"));
    let mut reader = CompressedBlockReader::open(&data_path)?;
    crate::mark_validation::validate_wide_marks_reader(&marks_file, &mut reader)?;
    reader.seek_to_block(0)?;
    Ok((marks_file.marks, reader))
}
