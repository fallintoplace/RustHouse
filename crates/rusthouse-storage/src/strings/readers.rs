use super::*;

pub(super) fn checked_rows(rows: usize) -> Result<(), StringReadError> {
    if rows > MAX_BATCH_ROWS {
        Err(StringReadError::TooManyRows(rows))
    } else {
        Ok(())
    }
}

pub(super) fn checked_end(current: usize, length: u64) -> Result<usize, StringReadError> {
    let available = MAX_BATCH_BYTES - current;
    if length > available as u64 {
        return Err(StringReadError::TooManyBytes);
    }
    Ok(current + length as usize)
}

pub fn read_legacy_strings<R: Read>(
    reader: &mut R,
    rows: usize,
) -> Result<StringColumn, StringReadError> {
    checked_rows(rows)?;
    let mut values = StringColumn::new(rows);
    for _ in 0..rows {
        let length = read_varuint(reader)?;
        values.push(reader, length)?;
    }
    Ok(values)
}

/// Read legacy String rows directly from the current verified block slice.
/// A buffered source avoids a virtual Read call for every varint byte and
/// copies each payload directly into the final StringColumn allocation.
pub(crate) fn read_legacy_strings_buffered<R: BufRead>(
    reader: &mut R,
    rows: usize,
) -> Result<StringColumn, StringReadError> {
    checked_rows(rows)?;
    let mut values = StringColumn::new(rows);
    for _ in 0..rows {
        // Most String rows have a one-byte length and fit in one block.
        // Read the prefix and payload from one BufRead slice in that case.
        let in_buffer = {
            let chunk = reader.fill_buf()?;
            if let Some((&length, rest)) = chunk.split_first() {
                let length = usize::from(length);
                if length < 128 && rest.len() >= length {
                    let next = checked_end(values.data.len(), length as u64)?;
                    values.data.extend_from_slice(&rest[..length]);
                    values.offsets.push(next);
                    Some(length + 1)
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(consumed) = in_buffer {
            reader.consume(consumed);
            continue;
        }
        let length = read_varuint_buffered(reader)?;
        let next = checked_end(values.data.len(), length)?;
        while values.data.len() < next {
            let chunk = reader.fill_buf()?;
            if chunk.is_empty() {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
            }
            let take = (next - values.data.len()).min(chunk.len());
            values.data.extend_from_slice(&chunk[..take]);
            reader.consume(take);
        }
        values.offsets.push(next);
    }
    Ok(values)
}

pub(super) fn read_varuint_buffered<R: BufRead>(reader: &mut R) -> Result<u64, StringReadError> {
    let mut result = 0u64;
    let mut group = 0usize;
    while group < 10 {
        // Decode every available varint group from this buffer before
        // calling fill_buf again. Most string lengths fit in one buffer.
        let (consumed, finished, invalid) = {
            let bytes = reader.fill_buf()?;
            if group == 0 && bytes.first().is_some_and(|byte| *byte < 0x80) {
                let value = u64::from(bytes[0]);
                reader.consume(1);
                return Ok(value);
            }
            if bytes.is_empty() {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
            }
            let mut consumed = 0usize;
            let mut finished = false;
            let mut invalid = false;
            for &byte in bytes.iter().take(10 - group) {
                let position = group + consumed;
                consumed += 1;
                if position == 9 && byte > 1 {
                    invalid = true;
                    break;
                }
                let value = u64::from(byte & 0x7f);
                result |= value << (position * 7);
                if byte & 0x80 == 0 {
                    invalid = position != 0 && value == 0;
                    finished = !invalid;
                    break;
                }
            }
            (consumed, finished, invalid)
        };
        reader.consume(consumed);
        if invalid {
            return Err(StringReadError::InvalidVarUInt);
        }
        if finished {
            return Ok(result);
        }
        group += consumed;
    }
    Err(StringReadError::InvalidVarUInt)
}

pub(super) fn append_sized_offsets(values: &mut StringColumn, bytes: &[u8]) -> Result<(), StringReadError> {
    let mut end = *values.offsets.last().unwrap();
    for encoded in bytes.chunks_exact(SIZE_ENTRY_BYTES) {
        let length = u64::from_le_bytes(encoded.try_into().unwrap());
        end = checked_end(end, length)?;
        values.offsets.push(end);
    }
    Ok(())
}

pub fn read_sized_strings<S: Read, D: Read>(
    sizes: &mut S,
    data: &mut D,
    rows: usize,
) -> Result<StringColumn, StringReadError> {
    checked_rows(rows)?;
    let mut values = StringColumn::new(rows);
    // Most marks have few rows. Avoid initializing a full 64 KiB size
    // buffer when their entire offset stream fits in a small stack array.
    let size_bytes = rows * SIZE_ENTRY_BYTES;
    if size_bytes <= 512 {
        let mut buffer = [0u8; 512];
        sizes.read_exact(&mut buffer[..size_bytes])?;
        append_sized_offsets(&mut values, &buffer[..size_bytes])?;
    } else if size_bytes <= 8192 {
        let mut buffer = [0u8; 8192];
        sizes.read_exact(&mut buffer[..size_bytes])?;
        append_sized_offsets(&mut values, &buffer[..size_bytes])?;
    } else {
        // Large batches retain the bounded sequential read path and never
        // consume bytes belonging to the following size batch.
        let mut buffer = [0u8; SIZE_ENTRY_BYTES * 8192];
        let mut decoded_rows = 0;
        while decoded_rows < rows {
            let batch_rows = (rows - decoded_rows).min(buffer.len() / SIZE_ENTRY_BYTES);
            sizes.read_exact(&mut buffer[..batch_rows * SIZE_ENTRY_BYTES])?;
            append_sized_offsets(&mut values, &buffer[..batch_rows * SIZE_ENTRY_BYTES])?;
            decoded_rows += batch_rows;
        }
    }
    let bytes = *values.offsets.last().unwrap();
    values.data.resize(bytes, 0);
    data.read_exact(&mut values.data)?;
    Ok(values)
}

pub(super) fn read_varuint<R: Read>(reader: &mut R) -> Result<u64, StringReadError> {
    let mut result = 0u64;
    for group in 0..10 {
        let mut byte = [0; 1];
        reader.read_exact(&mut byte)?;
        if group == 9 && byte[0] > 1 {
            return Err(StringReadError::InvalidVarUInt);
        }
        let value = u64::from(byte[0] & 0x7f);
        result |= value << (group * 7);
        if byte[0] & 0x80 == 0 {
            if group != 0 && value == 0 {
                return Err(StringReadError::InvalidVarUInt);
            }
            return Ok(result);
        }
    }
    Err(StringReadError::InvalidVarUInt)
}
