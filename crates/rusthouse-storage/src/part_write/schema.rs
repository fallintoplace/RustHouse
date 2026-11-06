fn wrapped_inner<'a>(kind: &'a str, prefix: &str) -> Option<&'a str> {
    kind.strip_prefix(prefix)?.strip_suffix(')')
}

pub(super) fn base_type(kind: &str) -> (&str, bool) {
    if let Some(base) = wrapped_inner(kind, "Nullable(") {
        (base, true)
    } else {
        (kind, false)
    }
}

fn numeric_scalar_width(kind: &str) -> Option<usize> {
    match kind {
        "UInt8" | "Int8" => Some(1),
        "UInt16" | "Int16" => Some(2),
        "UInt32" | "Int32" | "Float32" => Some(4),
        "UInt64" | "Int64" | "Float64" => Some(8),
        "UInt128" | "Int128" => Some(16),
        _ => None,
    }
}

fn numeric_scalar(kind: &str) -> bool {
    numeric_scalar_width(kind).is_some()
}

pub(super) fn numeric_array_inner_type(kind: &str) -> Option<&str> {
    let inner = wrapped_inner(kind, "Array(")?;
    numeric_scalar(inner).then_some(inner)
}

pub(super) fn map_string_value_type(kind: &str) -> Option<&str> {
    let value = wrapped_inner(kind, "Map(String, ")?;
    (value == "String" || numeric_scalar(value)).then_some(value)
}

pub(super) fn supported(kind: &str) -> bool {
    let (base, nullable) = base_type(kind);
    if numeric_array_inner_type(base).is_some()
        || map_string_value_type(base).is_some()
        || base == "UUID"
    {
        return !nullable;
    }
    matches!(base, "String" | "Date" | "Date32" | "DateTime") || numeric_scalar(base)
}

pub(super) fn fixed_width_bytes_per_row(data_type: &str) -> usize {
    let kind = base_type(data_type).0;
    match kind {
        "Date" => 2,
        "Date32" | "DateTime" => 4,
        "UUID" => 16,
        _ => numeric_scalar_width(kind).unwrap_or(0),
    }
}
