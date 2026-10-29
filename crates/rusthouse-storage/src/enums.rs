use crate::{NumericColumn, NumericReadError, read_numeric_column};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::io::Read;

const MAX_BATCH_ROWS: usize = 1_000_000;
const MAX_ENUM8_VARIANTS: usize = 256;
const MAX_ENUM16_VARIANTS: usize = 65_536;

/// Enum8 and Enum16 values use signed integer storage and a type-level label map.
#[derive(Debug, PartialEq, Eq)]
pub struct EnumColumn {
    codes: Vec<i16>,
    variants: BTreeMap<i16, String>,
}

impl EnumColumn {
    pub fn len(&self) -> usize {
        self.codes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.codes.is_empty()
    }

    pub fn codes(&self) -> &[i16] {
        &self.codes
    }

    pub fn variants(&self) -> &BTreeMap<i16, String> {
        &self.variants
    }

    pub fn name(&self, row: usize) -> Option<&str> {
        let code = self.codes.get(row)?;
        self.variants.get(code).map(String::as_str)
    }
}

#[derive(Debug)]
pub enum EnumReadError {
    InvalidType(String),
    TooManyRows(usize),
    Numeric(NumericReadError),
    UnknownValue { row: usize, code: i16 },
}

impl fmt::Display for EnumReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidType(value) => write!(f, "invalid Enum8/Enum16 type: {value}"),
            Self::TooManyRows(rows) => {
                write!(f, "Enum batch exceeds the row limit: {rows}")
            }
            Self::Numeric(source) => write!(f, "unable to read Enum values: {source}"),
            Self::UnknownValue { row, code } => {
                write!(f, "Enum value {code} at row {row} is absent from the type")
            }
        }
    }
}

impl Error for EnumReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Numeric(source) => Some(source),
            _ => None,
        }
    }
}

/// Decode signed Enum values and resolve them to names declared in the type.
/// Reject undefined codes instead of returning a wrong label.
pub fn read_enum_column<R: Read>(
    reader: &mut R,
    data_type: &str,
    rows: usize,
) -> Result<EnumColumn, EnumReadError> {
    let (width, variants) = parse_enum_type(data_type)
        .ok_or_else(|| EnumReadError::InvalidType(data_type.to_owned()))?;
    if rows > MAX_BATCH_ROWS {
        return Err(EnumReadError::TooManyRows(rows));
    }

    let values = read_numeric_column(reader, if width == 8 { "Int8" } else { "Int16" }, rows)
        .map_err(EnumReadError::Numeric)?;
    let codes: Vec<i16> = match values {
        NumericColumn::Int8(values) => values.into_iter().map(i16::from).collect(),
        NumericColumn::Int16(values) => values,
        _ => unreachable!("Enum storage is Int8 or Int16"),
    };

    for (row, &code) in codes.iter().enumerate() {
        if !variants.contains_key(&code) {
            return Err(EnumReadError::UnknownValue { row, code });
        }
    }

    Ok(EnumColumn { codes, variants })
}

fn skip_space(bytes: &[u8], position: &mut usize) {
    while bytes.get(*position).is_some_and(u8::is_ascii_whitespace) {
        *position += 1;
    }
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_enum_escape(bytes: &[u8], position: &mut usize) -> Option<u8> {
    let escaped = *bytes.get(*position)?;
    *position += 1;
    let decoded = match escaped {
        b'\\' => b'\\',
        b'\'' => b'\'',
        b'"' => b'"',
        b'n' => b'\n',
        b'r' => b'\r',
        b't' => b'\t',
        b'0' => 0,
        b'x' => {
            let high = hex_digit(*bytes.get(*position)?)?;
            let low = hex_digit(*bytes.get(*position + 1)?)?;
            *position += 2;
            (high << 4) | low
        }
        _ => return None,
    };
    Some(decoded)
}

fn parse_enum_label(bytes: &[u8], position: &mut usize) -> Option<String> {
    if bytes.get(*position) != Some(&b'\'') {
        return None;
    }
    *position += 1;
    let mut name = Vec::new();
    while let Some(&byte) = bytes.get(*position) {
        *position += 1;
        match byte {
            b'\'' => {
                if bytes.get(*position) == Some(&b'\'') {
                    *position += 1;
                    name.push(b'\'');
                } else {
                    return String::from_utf8(name).ok();
                }
            }
            b'\\' => {
                name.push(decode_enum_escape(bytes, position)?);
            }
            _ => name.push(byte),
        }
    }
    None
}

fn parse_enum_code(inner: &str, bytes: &[u8], position: &mut usize) -> Option<i16> {
    let start = *position;
    if bytes.get(*position) == Some(&b'-') || bytes.get(*position) == Some(&b'+') {
        *position += 1;
    }
    let digit_start = *position;
    while bytes.get(*position).is_some_and(u8::is_ascii_digit) {
        *position += 1;
    }
    if *position == digit_start {
        return None;
    }
    inner[start..*position].parse::<i16>().ok()
}

/// Accept canonical explicit value declarations and common ClickHouse escapes.
/// Implicit Enum numbering is not used for persisted columns.txt type names.
fn parse_enum_type(data_type: &str) -> Option<(u8, BTreeMap<i16, String>)> {
    let (width, inner) = if let Some(inner) = data_type
        .strip_prefix("Enum8(")
        .and_then(|s| s.strip_suffix(')'))
    {
        (8, inner)
    } else {
        (16, data_type.strip_prefix("Enum16(")?.strip_suffix(')')?)
    };

    let bytes = inner.as_bytes();
    let mut position = 0;
    let mut variants = BTreeMap::new();
    let mut names = HashSet::new();
    skip_space(bytes, &mut position);

    while position < bytes.len() {
        let name = parse_enum_label(bytes, &mut position)?;
        skip_space(bytes, &mut position);
        if bytes.get(position) != Some(&b'=') {
            return None;
        }
        position += 1;
        skip_space(bytes, &mut position);

        let code = parse_enum_code(inner, bytes, &mut position)?;
        if width == 8 && !(-128..=127).contains(&code) {
            return None;
        }
        if !names.insert(name.clone()) || variants.insert(code, name).is_some() {
            return None;
        }
        let max_variants = if width == 8 {
        MAX_ENUM8_VARIANTS
    } else {
        MAX_ENUM16_VARIANTS
    };
        if variants.len() > max_variants {
            return None;
        }

        skip_space(bytes, &mut position);
        if position == bytes.len() {
            break;
        }
        if bytes[position] != b',' {
            return None;
        }
        position += 1;
        skip_space(bytes, &mut position);
        if position == bytes.len() {
            return None;
        }
    }

    (!variants.is_empty()).then_some((width, variants))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    struct NoRead;

    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("must reject before reading")
        }
    }

    struct OneByte<R>(R);

    impl<R: Read> Read for OneByte<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(1);
            self.0.read(&mut buffer[..count])
        }
    }

    #[test]
    fn reads_enum8_positive_negative_and_escaped_names() {
        let mut input = OneByte(Cursor::new([255u8, 2, 77]));
        let column =
            read_enum_column(&mut input, r"Enum8('it\'s' = -1, 'ok\x20now' = 2)", 2).unwrap();
        assert_eq!(column.len(), 2);
        assert_eq!(column.codes(), &[-1, 2]);
        assert_eq!(column.name(0), Some("it's"));
        assert_eq!(column.name(1), Some("ok now"));
        assert_eq!(column.name(2), None);
        assert_eq!(
            column.variants().get(&2).map(String::as_str),
            Some("ok now")
        );
        assert_eq!(input.0.position(), 2);
    }

    #[test]
    fn supports_doubled_quote_enum_labels() {
        let mut input = &[255u8, 2][..];
        let column = read_enum_column(&mut input, "Enum8('it''s' = -1, 'don''t' = 2)", 2).unwrap();
        assert_eq!(column.name(0), Some("it's"));
        assert_eq!(column.name(1), Some("don't"));
        assert!(input.is_empty());
    }

    #[test]
    fn reads_enum16_signed_values_and_unicode_labels() {
        let values = [i16::MIN, 32767i16];
        let bytes = values
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();
        let mut input = OneByte(Cursor::new(bytes));
        let column =
            read_enum_column(&mut input, "Enum16('💡' = -32768, 'yes' = 32767)", 2).unwrap();
        assert_eq!(column.codes(), values);
        assert_eq!(column.name(0), Some("💡"));
        assert_eq!(column.name(1), Some("yes"));
        assert_eq!(input.0.position(), 4);
    }

    #[test]
    fn detects_undefined_values_and_short_streams() {
        assert!(matches!(
            read_enum_column(&mut &[7u8][..], "Enum8('one' = 1)", 1),
            Err(EnumReadError::UnknownValue { row: 0, code: 7 })
        ));
        assert!(matches!(
            read_enum_column(&mut &[1u8][..], "Enum16('one' = 1)", 1),
            Err(EnumReadError::Numeric(NumericReadError::Io(error)))
                if error.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn rejects_invalid_declarations_before_reading() {
        for data_type in [
            "Enum8",
            "Enum8()",
            "Enum8('a')",
            "Enum8('a' = 1,)",
            "Enum8('a' = 1, 'a' = 2)",
            "Enum8('a' = 1, 'b' = 1)",
            "Enum8('a' = -129)",
            "Enum8('a' = 128)",
            "Enum8('a' = 256)",
            "Enum16('a' = -32769)",
            "Enum16('a' = 32768)",
            "Enum8('a' = x)",
            "Enum8('a' = 1 extra)",
            r"Enum8('bad\z' = 1)",
            r"Enum8('bad\xZZ' = 1)",
            "Enum8('unterminated = 1)",
            "Enum8('a' = 1",
            "Enum8('a' = 1) ",
        ] {
            assert!(
                matches!(
                    read_enum_column(&mut NoRead, data_type, 1),
                    Err(EnumReadError::InvalidType(_))
                ),
                "{data_type}"
            );
        }
    }

    #[test]
    fn enforces_row_limit_before_reading() {
        assert!(matches!(
            read_enum_column(&mut NoRead, "Enum8('one' = 1)", MAX_BATCH_ROWS + 1),
            Err(EnumReadError::TooManyRows(_))
        ));
    }

    #[test]
    fn empty_batch_preserves_enum_metadata_without_reading() {
        let column = read_enum_column(&mut NoRead, "Enum16('zero' = 0)", 0).unwrap();
        assert!(column.is_empty());
        assert_eq!(column.len(), 0);
        assert_eq!(column.variants().get(&0).map(String::as_str), Some("zero"));
    }
}
