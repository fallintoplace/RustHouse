//! Parsed physical column types shared by wide-part readers and writers.
//! Reading and writing deliberately retain separate capability checks.

use crate::numeric::numeric_storage_type;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PhysicalType<'a> {
    Numeric(&'a str),
    Text,
    Uuid,
    NullableNumeric(&'a str),
    NullableText,
    NumericArray(&'a str),
    StringMap(&'a str),
}

fn wrapped_inner<'a>(kind: &'a str, prefix: &str) -> Option<&'a str> {
    kind.strip_prefix(prefix)?.strip_suffix(')')
}

pub(crate) fn base_type(kind: &str) -> (&str, bool) {
    if let Some(base) = wrapped_inner(kind, "Nullable(") {
        (base, true)
    } else {
        (kind, false)
    }
}

pub(crate) fn numeric_scalar_width(kind: &str) -> Option<usize> {
    match kind {
        "UInt8" | "Int8" => Some(1),
        "UInt16" | "Int16" => Some(2),
        "UInt32" | "Int32" | "Float32" => Some(4),
        "UInt64" | "Int64" | "Float64" => Some(8),
        "UInt128" | "Int128" => Some(16),
        _ => None,
    }
}

pub(crate) fn numeric_array_inner_type(kind: &str) -> Option<&str> {
    let inner = wrapped_inner(kind, "Array(")?;
    numeric_scalar_width(inner).map(|_| inner)
}

pub(crate) fn map_string_value_type(kind: &str) -> Option<&str> {
    let value = wrapped_inner(kind, "Map(String, ")?;
    (value == "String" || numeric_scalar_width(value).is_some()).then_some(value)
}

impl<'a> PhysicalType<'a> {
    pub(crate) fn parse(name: &'a str) -> Option<Self> {
        let (base, nullable) = base_type(name);
        if nullable {
            return match base {
                "String" => Some(Self::NullableText),
                other if numeric_storage_type(other).is_some() => {
                    Some(Self::NullableNumeric(other))
                }
                _ => None,
            };
        }
        if let Some(inner) = numeric_array_inner_type(base) {
            return Some(Self::NumericArray(inner));
        }
        if let Some(value) = map_string_value_type(base) {
            return Some(Self::StringMap(value));
        }
        match base {
            "String" => Some(Self::Text),
            "UUID" => Some(Self::Uuid),
            other if numeric_storage_type(other).is_some() => Some(Self::Numeric(other)),
            _ => None,
        }
    }

    /// The writer supports a narrower set of physical numeric types than the scanner.
    pub(crate) fn writable(self) -> bool {
        match self {
            Self::Numeric(kind) | Self::NullableNumeric(kind) => {
                numeric_scalar_width(kind).is_some()
                    || matches!(kind, "Date" | "Date32" | "DateTime")
            }
            Self::Text | Self::NullableText | Self::Uuid | Self::NumericArray(_) | Self::StringMap(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_and_writer_share_nullable_array_and_map_shape() {
        assert_eq!(PhysicalType::parse("Nullable(Int32)"), Some(PhysicalType::NullableNumeric("Int32")));
        assert_eq!(PhysicalType::parse("Array(UInt64)"), Some(PhysicalType::NumericArray("UInt64")));
        assert_eq!(PhysicalType::parse("Map(String, String)"), Some(PhysicalType::StringMap("String")));
        assert_eq!(PhysicalType::parse("Nullable(Map(String, UInt8))"), None);
    }

    #[test]
    fn reader_accepts_timestamp_extensions_without_expanding_writer_support() {
        let timestamp = PhysicalType::parse("DateTime64(3)").unwrap();
        assert_eq!(timestamp, PhysicalType::Numeric("DateTime64(3)"));
        assert!(!timestamp.writable());
        assert!(PhysicalType::parse("Nullable(Date)").unwrap().writable());
        assert!(PhysicalType::parse("Nullable(UUID)").is_none());
    }
}
