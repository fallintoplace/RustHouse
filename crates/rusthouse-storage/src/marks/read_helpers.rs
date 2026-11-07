use super::*;

pub(super) fn marks_io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io {
        path: path.to_path_buf(),
        source,
    }
}

// Verify both path identity and immutable contents across a raw mark read.
pub(super) fn marks_file_unchanged(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    if !after.file_type().is_file()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec()
    }
    #[cfg(not(unix))]
    true
}

pub(super) fn read_plain_wide_marks(path: &Path, format: WideMarkFormat) -> Result<Vec<WideMark>, PartError> {
    let entry = std::fs::symlink_metadata(path).map_err(|source| marks_io_error(path, source))?;
    if !entry.file_type().is_file() {
        return Err(PartError::Io {
            path: path.to_path_buf(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "expected a regular marks file"),
        });
    }
    let mut file = File::open(path).map_err(|source| marks_io_error(path, source))?;
    let opened = file
        .metadata()
        .map_err(|source| marks_io_error(path, source))?;
    // The same-length path could have been rewritten in place between stat
    // and open. Check the baseline timestamps, not only its inode and length.
    if !marks_file_unchanged(&entry, &opened) {
        return Err(marks_io_error(
            path,
            io::Error::new(io::ErrorKind::InvalidData, "marks file changed before open"),
        ));
    }
    let size = opened.len();
    if size > MAX_MARKS_BYTES as u64 {
        return Err(marks_too_large(path));
    }
    // Malformed lengths cannot contain complete mark records. Reject them
    // before allocating or reading the potentially 64 MiB backing file.
    if size == 0 {
        return Err(PartError::EmptyMarksFile(path.to_path_buf()));
    }
    let record_size = format.record_size();
    if size % record_size as u64 != 0 {
        return Err(PartError::InvalidMarksLength {
            path: path.to_path_buf(),
            size: size as usize,
            record_size,
        });
    }
    // A mark file may contain millions of records. Decode in bounded
    // batches instead of retaining both a complete raw copy and all marks.
    // 24 KiB is divisible by both 16-byte and 24-byte record formats.
    const MARK_READ_BYTES: usize = 24 * 1024;
    let size = size as usize; // checked against the 64 MiB cap above
    let mut marks = Vec::with_capacity(size / record_size);
    let mut buffer = [0u8; MARK_READ_BYTES];
    let mut remaining = size;
    while remaining > 0 {
        let amount = remaining.min(buffer.len());
        if let Err(source) = file.read_exact(&mut buffer[..amount]) {
            if source.kind() == io::ErrorKind::UnexpectedEof {
                return Err(marks_io_error(
                    path,
                    io::Error::new(io::ErrorKind::InvalidData, "marks file changed during read"),
                ));
            }
            return Err(marks_io_error(path, source));
        }
        marks.extend(
            buffer[..amount]
                .chunks_exact(record_size)
                .map(|record| parse_mark_record(record, format)),
        );
        remaining -= amount;
    }
    // Keep the open descriptor alive and check the path again. A same-size
    // replacement or in-place rewrite must not be silently trusted.
    let finished = file
        .metadata()
        .map_err(|source| marks_io_error(path, source))?;
    let current = std::fs::symlink_metadata(path).map_err(|source| marks_io_error(path, source))?;
    if !marks_file_unchanged(&opened, &finished) || !marks_file_unchanged(&opened, &current) {
        return Err(marks_io_error(
            path,
            io::Error::new(io::ErrorKind::InvalidData, "marks file changed during read"),
        ));
    }
    Ok(marks)
}

pub(super) fn read_compressed_wide_marks<R: Read + Seek>(
    mut reader: CompressedBlockReader<R>,
    path: &Path,
    format: WideMarkFormat,
) -> Result<Vec<WideMark>, PartError> {
    let record_size = format.record_size();
    let mut marks = Vec::new();
    let mut fragment = [0u8; 24];
    let mut fragment_len = 0;
    let mut decoded_bytes = 0usize;
    // A compressed marks file can contain many frames. Reuse the previous
    // decoded frame's capacity after its records have been consumed.
    let mut decoded_scratch = Vec::new();
    while let Some(block) = reader.next_decoded_reusing(decoded_scratch) {
        let data = block?.data;
        if data.len() > MAX_MARKS_BYTES - decoded_bytes {
            return Err(marks_too_large(path));
        }
        decoded_bytes += data.len();
        // Include the partial record saved from the preceding decoded block.
        // Reserving per block avoids repeated Vec growth on large mark files.
        let records = (fragment_len + data.len()) / record_size;
        marks
            .try_reserve(records)
            .map_err(|_| marks_too_large(path))?;
        let mut bytes = data.as_slice();

        if fragment_len > 0 {
            let take = (record_size - fragment_len).min(bytes.len());
            fragment[fragment_len..fragment_len + take].copy_from_slice(&bytes[..take]);
            fragment_len += take;
            bytes = &bytes[take..];
            if fragment_len < record_size {
                decoded_scratch = data;
                continue;
            }
            marks.push(parse_mark_record(&fragment[..record_size], format));
        }

        let mut chunks = bytes.chunks_exact(record_size);
        marks.extend(
            chunks
                .by_ref()
                .map(|record| parse_mark_record(record, format)),
        );
        let remainder = chunks.remainder();
        fragment[..remainder.len()].copy_from_slice(remainder);
        fragment_len = remainder.len();
        decoded_scratch = data;
    }

    let size = decoded_bytes;
    if size == 0 {
        return Err(PartError::EmptyMarksFile(path.to_path_buf()));
    }
    if fragment_len != 0 {
        return Err(PartError::InvalidMarksLength {
            path: path.to_path_buf(),
            size,
            record_size,
        });
    }
    Ok(marks)
}

pub(super) fn marks_too_large(path: &Path) -> PartError {
    PartError::Io {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, "marks file exceeds 64 MiB"),
    }
}
