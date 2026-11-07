use super::*;

pub(super) fn decimal_storage_type(precision: u8) -> &'static str {
    if precision <= 9 {
        "Int32"
    } else if precision <= 18 {
        "Int64"
    } else {
        "Int128"
    }
}

pub(super) fn parse_decimal_type_number(value: &str) -> Option<u8> {
    if value.is_empty() {
        return None;
    }
    // Numeric type parameters are tiny, but schema inspection reads them
    // repeatedly. Validate and decode at once without a second parse pass.
    let mut number = 0u8;
    for byte in value.bytes() {
        if !byte.is_ascii_digit() {
            return None;
        }
        number = number.checked_mul(10)?.checked_add(byte - b'0')?;
    }
    Some(number)
}

pub(super) fn checked_decimal_spec(precision: u8, scale: u8) -> Option<(u8, u8, &'static str)> {
    if precision == 0 || precision > 38 || scale > precision {
        return None;
    }
    Some((precision, scale, decimal_storage_type(precision)))
}

pub(super) fn parse_sized_decimal(data_type: &str) -> Option<(u8, u8, &'static str)> {
    let (width, arguments) = data_type.strip_prefix("Decimal")?.split_once('(')?;
    let precision = match width {
        "32" => 9,
        "64" => 18,
        "128" => 38,
        _ => return None,
    };
    let scale = parse_decimal_type_number(arguments.strip_suffix(')')?)?;
    checked_decimal_spec(precision, scale)
}

pub(super) fn parse_general_decimal(inner: &str) -> Option<(u8, u8, &'static str)> {
    let (precision, scale) = match inner.split_once(',') {
        Some((precision, scale)) => (precision.trim(), scale.trim()),
        None => (inner.trim(), "0"),
    };
    let precision = parse_decimal_type_number(precision)?;
    let scale = parse_decimal_type_number(scale)?;
    checked_decimal_spec(precision, scale)
}

pub(super) fn parse_decimal_type(data_type: &str) -> Option<(u8, u8, &'static str)> {
    if data_type == "Decimal" {
        return Some((10, 0, "Int64"));
    }
    if let Some(decimal) = parse_sized_decimal(data_type) {
        return Some(decimal);
    }
    let inner = data_type.strip_prefix("Decimal(")?.strip_suffix(')')?;
    parse_general_decimal(inner)
}

/// Reuse the exact decoder type grammar when verifying physical part widths.
pub(crate) fn decimal_stream_width(data_type: &str) -> Option<u64> {
    let (_, _, storage) = parse_decimal_type(data_type)?;
    match storage {
        "Int32" => Some(4),
        "Int64" => Some(8),
        "Int128" => Some(16),
        _ => None,
    }
}
