use super::*;

// Validate eight independent 0/1 flags per word. The repeated mask is
// endian-independent and checks every bit except each byte's low bit.
pub(super) fn valid_null_flags(null_map: &[u8]) -> bool {
    let mut chunks = null_map.chunks_exact(8);
    if !chunks.by_ref().all(|bytes| {
        u64::from_ne_bytes(bytes.try_into().expect("eight null flags")) & 0xfefe_fefe_fefe_fefe == 0
    }) {
        return false;
    }
    chunks.remainder().iter().all(|&value| value <= 1)
}

pub fn read_null_map<R: Read>(reader: &mut R, rows: usize) -> Result<Vec<u8>, NullableReadError> {
    if rows > MAX_BATCH_ROWS {
        return Err(NullableReadError::TooManyRows(rows));
    }
    let mut null_map = vec![0; rows];
    reader
        .read_exact(&mut null_map)
        .map_err(NullableReadError::Io)?;
    if !valid_null_flags(&null_map) {
        return Err(NullableReadError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "Nullable null map must contain only 0 or 1",
        )));
    }
    Ok(null_map)
}

pub(super) fn nullable_inner_type(data_type: &str) -> Result<&str, NullableReadError> {
    data_type
        .strip_prefix("Nullable(")
        .and_then(|inner| inner.strip_suffix(')'))
        .filter(|inner| !inner.is_empty())
        .ok_or_else(|| NullableReadError::InvalidType(data_type.to_owned()))
}

pub(super) fn read_nullable_values<N: Read, T, E>(
    nulls: &mut N,
    rows: usize,
    read_values: impl FnOnce() -> Result<T, E>,
    map_error: impl FnOnce(E) -> NullableReadError,
) -> Result<NullableColumn<T>, NullableReadError> {
    let null_map = read_null_map(nulls, rows)?;
    let values = read_values().map_err(map_error)?;
    Ok(NullableColumn { null_map, values })
}

pub fn read_nullable_numeric_column<N: Read, D: Read>(
    nulls: &mut N,
    data: &mut D,
    data_type: &str,
    rows: usize,
) -> Result<NullableColumn<NumericColumn>, NullableReadError> {
    let nested_type = nullable_inner_type(data_type)?;
    // Type validation is a lookup, not a zero-row decode per nullable batch.
    if crate::numeric::numeric_storage_type(nested_type).is_none() {
        return Err(NullableReadError::InvalidType(data_type.to_owned()));
    }
    read_nullable_values(
        nulls,
        rows,
        || read_numeric_column(data, nested_type, rows),
        NullableReadError::Numeric,
    )
}

pub fn read_nullable_fixed_strings<N: Read, D: Read>(
    nulls: &mut N,
    data: &mut D,
    data_type: &str,
    rows: usize,
) -> Result<NullableColumn<FixedStringColumn>, NullableReadError> {
    let nested_type = nullable_inner_type(data_type)?;
    if !nested_type.starts_with("FixedString(") || !nested_type.ends_with(')') {
        return Err(NullableReadError::InvalidType(data_type.to_owned()));
    }
    crate::fixed_strings::checked_size(nested_type, rows)
        .map_err(NullableReadError::FixedString)?;

    read_nullable_values(
        nulls,
        rows,
        || read_fixed_strings(data, nested_type, rows),
        NullableReadError::FixedString,
    )
}

pub(super) fn read_nullable_strings<N: Read>(
    nulls: &mut N,
    rows: usize,
    read_values: impl FnOnce() -> Result<StringColumn, StringReadError>,
) -> Result<NullableColumn<StringColumn>, NullableReadError> {
    read_nullable_values(nulls, rows, read_values, NullableReadError::String)
}

pub fn read_nullable_legacy_strings<N: Read, D: Read>(
    nulls: &mut N,
    data: &mut D,
    rows: usize,
) -> Result<NullableColumn<StringColumn>, NullableReadError> {
    read_nullable_strings(nulls, rows, || read_legacy_strings(data, rows))
}

pub fn read_nullable_sized_strings<N: Read, S: Read, D: Read>(
    nulls: &mut N,
    sizes: &mut S,
    data: &mut D,
    rows: usize,
) -> Result<NullableColumn<StringColumn>, NullableReadError> {
    read_nullable_strings(nulls, rows, || read_sized_strings(sizes, data, rows))
}
