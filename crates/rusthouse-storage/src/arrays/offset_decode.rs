use super::*;

// Keep small and large mark decoders on the same validation path. The
// const mode lets the compiler remove the encoding branch for either caller.
#[inline]
pub(super) fn append_array_offset<const ABSOLUTE: bool>(
    base: u64,
    absolute: &mut u64,
    encoded: u64,
    row: usize,
    offsets: &mut Vec<usize>,
) -> Result<(), ArrayReadError> {
    let next = if ABSOLUTE {
        encoded
    } else {
        absolute
            .checked_add(encoded)
            .ok_or(ArrayReadError::InvalidOffset {
                row,
                reason: "array size overflows u64",
            })?
    };
    if next < *absolute {
        return Err(ArrayReadError::InvalidOffset {
            row,
            reason: "offset moves backwards",
        });
    }
    let elements = next - base;
    if elements > MAX_BATCH_ELEMENTS {
        return Err(ArrayReadError::TooManyElements(elements));
    }
    offsets.push(elements as usize);
    *absolute = next;
    Ok(())
}

#[inline]
pub(super) fn append_encoded_offset_records<const ABSOLUTE: bool>(
    base: u64,
    absolute: &mut u64,
    first_row: usize,
    bytes: &[u8],
    offsets: &mut Vec<usize>,
) -> Result<(), ArrayReadError> {
    for (index, record) in bytes.chunks_exact(OFFSET_ENTRY_BYTES).enumerate() {
        let encoded = u64::from_le_bytes(record.try_into().unwrap());
        append_array_offset::<ABSOLUTE>(base, absolute, encoded, first_row + index, offsets)?;
    }
    Ok(())
}

// Specialize the two offset encodings before the hot decode loop. The
// compiler can eliminate the per-element mode match for million-row arrays.
pub(super) fn decode_array_offsets<const ABSOLUTE: bool, O: Read>(
    offset_stream: &mut O,
    rows: usize,
    base: u64,
) -> Result<ArrayOffsets, ArrayReadError> {
    let mut absolute = base;
    let mut offsets = Vec::with_capacity(rows + 1);
    offsets.push(0);

    // Decode small Array marks without clearing the 64 KiB scratch buffer.
    // A complete 64-row offset batch fits in 512 bytes.
    const SMALL_ROWS: usize = 64;
    if rows <= SMALL_ROWS {
        let mut scratch = [0u8; SMALL_ROWS * OFFSET_ENTRY_BYTES];
        let bytes = rows * OFFSET_ENTRY_BYTES;
        offset_stream
            .read_exact(&mut scratch[..bytes])
            .map_err(ArrayReadError::Offsets)?;
        append_encoded_offset_records::<ABSOLUTE>(
            base,
            &mut absolute,
            0,
            &scratch[..bytes],
            &mut offsets,
        )?;
        return Ok(ArrayOffsets {
            offsets,
            last_absolute_offset: absolute,
        });
    }

    // Typical medium marks fit within 8 KiB. Decode them with one exact
    // read instead of initializing the 64 KiB bulk buffer.
    const MEDIUM_ROWS: usize = 1024;
    if rows <= MEDIUM_ROWS {
        let mut scratch = [0u8; MEDIUM_ROWS * OFFSET_ENTRY_BYTES];
        let bytes = rows * OFFSET_ENTRY_BYTES;
        offset_stream
            .read_exact(&mut scratch[..bytes])
            .map_err(ArrayReadError::Offsets)?;
        append_encoded_offset_records::<ABSOLUTE>(
            base,
            &mut absolute,
            0,
            &scratch[..bytes],
            &mut offsets,
        )?;
        return Ok(ArrayOffsets {
            offsets,
            last_absolute_offset: absolute,
        });
    }

    let mut buffer = [0u8; 64 * 1024];
    let mut row = 0;
    while row < rows {
        let count = (rows - row).min(buffer.len() / OFFSET_ENTRY_BYTES);
        offset_stream
            .read_exact(&mut buffer[..count * OFFSET_ENTRY_BYTES])
            .map_err(ArrayReadError::Offsets)?;

        append_encoded_offset_records::<ABSOLUTE>(
            base,
            &mut absolute,
            row,
            &buffer[..count * OFFSET_ENTRY_BYTES],
            &mut offsets,
        )?;
        row += count;
    }

    Ok(ArrayOffsets {
        offsets,
        last_absolute_offset: absolute,
    })
}
