use super::*;

pub(super) fn push_varuint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 128 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

pub(super) fn push_cell(kind: &str, value: Option<&str>, output: &mut Vec<u8>) -> Result<bool, PartError> {
    let (base, nullable) = base_type(kind);
    let is_null = value.is_none();
    if is_null && !nullable {
        return Err(invalid(format!("NULL is not permitted for {kind}")));
    }
    let value = value.unwrap_or(if base == "String" { "" } else { "0" });
    macro_rules! numeric {
        ($ty:ty) => {{
            let number = value
                .parse::<$ty>()
                .map_err(|_| invalid(format!("invalid {kind} value: {value}")))?;
            output.extend_from_slice(&number.to_le_bytes());
        }};
    }
    match base {
        "String" => {
            push_varuint(value.len() as u64, output);
            output.extend_from_slice(value.as_bytes());
        }
        "Date" => {
            let days = crate::parse_date(value)
                .or_else(|| value.parse::<u16>().ok())
                .ok_or_else(|| invalid(format!("invalid Date value: {value}")))?;
            output.extend_from_slice(&days.to_le_bytes());
        }
        "DateTime" => {
            let seconds = crate::parse_datetime(value)
                .or_else(|| value.parse::<u32>().ok())
                .ok_or_else(|| invalid(format!("invalid DateTime value: {value}")))?;
            output.extend_from_slice(&seconds.to_le_bytes());
        }
        "UInt8" => numeric!(u8),
        "UInt16" => numeric!(u16),
        "UInt32" => numeric!(u32),
        "UInt64" => numeric!(u64),
        "UInt128" => numeric!(u128),
        "Int8" => numeric!(i8),
        "Int16" => numeric!(i16),
        "Int32" => numeric!(i32),
        "Int64" => numeric!(i64),
        "Int128" => numeric!(i128),
        "Float32" => {
            let number = value
                .parse::<f32>()
                .map_err(|_| invalid(format!("invalid {kind} value: {value}")))?;
            if !number.is_finite() {
                return Err(invalid("non-finite float values are unsupported"));
            }
            output.extend_from_slice(&number.to_le_bytes());
        }
        "Float64" => {
            let number = value
                .parse::<f64>()
                .map_err(|_| invalid(format!("invalid {kind} value: {value}")))?;
            if !number.is_finite() {
                return Err(invalid("non-finite float values are unsupported"));
            }
            output.extend_from_slice(&number.to_le_bytes());
        }
        _ => return Err(invalid(format!("unsupported writable column type: {kind}"))),
    }
    if output.len() > MAX_STREAM_BYTES {
        return Err(invalid("part column exceeds 8 MiB"));
    }
    Ok(is_null)
}

pub(super) fn checksum_hash(data: &[u8]) -> u128 {
    const HASH_BLOCK_BYTES: usize = 2048;
    let mut seed = U128::new(0, 0);
    let mut chunks = data.chunks_exact(HASH_BLOCK_BYTES);
    for chunk in chunks.by_ref() {
        seed = cityhash128_with_seed(chunk, seed);
    }
    if !chunks.remainder().is_empty() {
        seed = cityhash128_with_seed(chunks.remainder(), seed);
    }
    seed.into()
}

pub(super) fn encode_checksums(files: &[FileChecksum]) -> Vec<u8> {
    let mut bytes = b"checksums format version: 3\n".to_vec();
    push_varuint(files.len() as u64, &mut bytes);
    for file in files {
        push_varuint(file.name.len() as u64, &mut bytes);
        bytes.extend_from_slice(file.name.as_bytes());
        push_varuint(file.size, &mut bytes);
        bytes.extend_from_slice(&file.hash.to_le_bytes());
        match &file.uncompressed {
            Some(original) => {
                bytes.push(1);
                push_varuint(original.size, &mut bytes);
                bytes.extend_from_slice(&original.hash.to_le_bytes());
            }
            None => bytes.push(0),
        }
    }
    bytes
}
