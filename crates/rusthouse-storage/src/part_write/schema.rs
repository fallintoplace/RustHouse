use super::*;

pub(super) fn base_type(kind: &str) -> (&str, bool) {
    if let Some(base) = kind
        .strip_prefix("Nullable(")
        .and_then(|v| v.strip_suffix(')'))
    {
        (base, true)
    } else {
        (kind, false)
    }
}

pub(super) fn numeric_array_inner_type(kind: &str) -> Option<&str> {
    let inner = kind.strip_prefix("Array(")?.strip_suffix(')')?;
    matches!(
        inner,
        "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UInt128"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "Int128"
            | "Float32"
            | "Float64"
    )
    .then_some(inner)
}

pub(super) fn map_string_value_type(kind: &str) -> Option<&str> {
    let value = kind.strip_prefix("Map(String, ")?.strip_suffix(')')?;
    matches!(
        value,
        "String"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UInt128"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "Int128"
            | "Float32"
            | "Float64"
    )
    .then_some(value)
}

pub(super) fn supported(kind: &str) -> bool {
    let (base, nullable) = base_type(kind);
    if numeric_array_inner_type(base).is_some() {
        return !nullable;
    }
    if map_string_value_type(base).is_some() {
        return !nullable;
    }
    if base == "UUID" {
        return !nullable;
    }
    matches!(
        base,
        "String"
            | "Date"
            | "Date32"
            | "DateTime"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UInt128"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "Int128"
            | "Float32"
            | "Float64"
    )
}

pub(super) fn fixed_width_bytes_per_row(data_type: &str) -> usize {
    match base_type(data_type).0 {
        "UInt8" | "Int8" => 1,
        "UInt16" | "Int16" | "Date" => 2,
        "UInt32" | "Int32" | "Float32" | "Date32" | "DateTime" => 4,
        "UInt64" | "Int64" | "Float64" => 8,
        "UInt128" | "Int128" | "UUID" => 16,
        _ => 0,
    }
}
