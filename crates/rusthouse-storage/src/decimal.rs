use crate::{NumericColumn, NumericReadError, read_numeric_column};
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};

/// Decimal values are stored as signed integers scaled by 10^scale.
/// No floating-point conversion is performed.
#[derive(Debug, PartialEq)]
pub struct DecimalColumn {
    precision: u8,
    scale: u8,
    unscaled: NumericColumn,
}

impl DecimalColumn {
    pub fn len(&self) -> usize {
        self.unscaled.len()
    }

    pub fn is_empty(&self) -> bool {
        self.unscaled.is_empty()
    }

    pub fn precision(&self) -> u8 {
        self.precision
    }

    pub fn scale(&self) -> u8 {
        self.scale
    }

    pub fn unscaled_values(&self) -> &NumericColumn {
        &self.unscaled
    }
}

#[derive(Debug)]
pub enum DecimalReadError {
    InvalidType(String),
    ValueExceedsPrecision { row: usize, precision: u8 },
    Numeric(NumericReadError),
}

impl fmt::Display for DecimalReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidType(value) => write!(f, "unsupported Decimal type: {value}"),
            Self::Numeric(source) => write!(f, "unable to read Decimal values: {source}"),
            Self::ValueExceedsPrecision { row, precision } => {
                write!(
                    f,
                    "Decimal value at row {row} exceeds precision {precision}"
                )
            }
        }
    }
}

impl Error for DecimalReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidType(_) | Self::ValueExceedsPrecision { .. } => None,
            Self::Numeric(source) => Some(source),
        }
    }
}

/// Read the physical values of Decimal32, Decimal64, Decimal128, or Decimal(P, S).
/// Decimal256 is intentionally unsupported until 256-bit decoding is available.
pub fn read_decimal_column<R: Read>(
    reader: &mut R,
    data_type: &str,
    rows: usize,
) -> Result<DecimalColumn, DecimalReadError> {
    let (precision, scale, storage) = parse_decimal_type(data_type)
        .ok_or_else(|| DecimalReadError::InvalidType(data_type.to_owned()))?;
    let unscaled = read_numeric_column(reader, storage, rows).map_err(DecimalReadError::Numeric)?;
    let limit = 10i128.pow(u32::from(precision));
    let outside = |value: i128| value <= -limit || value >= limit;
    let invalid_row = match &unscaled {
        NumericColumn::Int32(values) => values.iter().position(|&v| outside(i128::from(v))),
        NumericColumn::Int64(values) => values.iter().position(|&v| outside(i128::from(v))),
        NumericColumn::Int128(values) => values.iter().position(|&v| outside(v)),
        _ => None,
    };
    if let Some(row) = invalid_row {
        return Err(DecimalReadError::ValueExceedsPrecision { row, precision });
    }
    Ok(DecimalColumn {
        precision,
        scale,
        unscaled,
    })
}

/// Encode unscaled signed Decimal values without a whole-column buffer.
///
/// Validate precision and destination width across the complete batch
/// before writing any bytes. The scale belongs to the physical type;
/// callers supply integer values that are already scaled accordingly.
pub fn write_decimal_values<W: Write>(
    output: &mut W,
    data_type: &str,
    values: &[i128],
) -> io::Result<()> {
    let (precision, _scale, storage) = parse_decimal_type(data_type)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid Decimal type"))?;
    let width = match storage {
        "Int32" => 4,
        "Int64" => 8,
        "Int128" => 16,
        _ => return Err(io::Error::other("unsupported Decimal storage width")),
    };
    let bytes = values.len().checked_mul(width).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "Decimal batch byte size overflow")
    })?;
    if bytes > crate::fixed_width::MAX_BATCH_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Decimal batch exceeds 64 MiB",
        ));
    }
    let limit = 10i128.pow(u32::from(precision));
    if values.iter().any(|&value| value <= -limit || value >= limit) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Decimal value exceeds declared precision",
        ));
    }

    let mut buffer = [0u8; 16 * 1024];
    for chunk in values.chunks(buffer.len() / width) {
        match storage {
            "Int32" => {
                for (index, &value) in chunk.iter().enumerate() {
                    buffer[index * 4..index * 4 + 4]
                        .copy_from_slice(&(value as i32).to_le_bytes());
                }
            }
            "Int64" => {
                for (index, &value) in chunk.iter().enumerate() {
                    buffer[index * 8..index * 8 + 8]
                        .copy_from_slice(&(value as i64).to_le_bytes());
                }
            }
            "Int128" => {
                for (index, &value) in chunk.iter().enumerate() {
                    buffer[index * 16..index * 16 + 16]
                        .copy_from_slice(&value.to_le_bytes());
                }
            }
            _ => unreachable!("Decimal storage was validated"),
        }
        output.write_all(&buffer[..chunk.len() * width])?;
    }
    Ok(())
}

fn decimal_storage_type(precision: u8) -> &'static str {
    if precision <= 9 {
        "Int32"
    } else if precision <= 18 {
        "Int64"
    } else {
        "Int128"
    }
}

fn parse_decimal_type_number(value: &str) -> Option<u8> {
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

fn checked_decimal_spec(precision: u8, scale: u8) -> Option<(u8, u8, &'static str)> {
    if precision == 0 || precision > 38 || scale > precision {
        return None;
    }
    Some((precision, scale, decimal_storage_type(precision)))
}

fn parse_sized_decimal(data_type: &str) -> Option<(u8, u8, &'static str)> {
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

fn parse_general_decimal(inner: &str) -> Option<(u8, u8, &'static str)> {
    let (precision, scale) = match inner.split_once(',') {
        Some((precision, scale)) => (precision.trim(), scale.trim()),
        None => (inner.trim(), "0"),
    };
    let precision = parse_decimal_type_number(precision)?;
    let scale = parse_decimal_type_number(scale)?;
    checked_decimal_spec(precision, scale)
}

fn parse_decimal_type(data_type: &str) -> Option<(u8, u8, &'static str)> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    struct OneByte<R>(R);

    impl<R: Read> Read for OneByte<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(1);
            self.0.read(&mut buffer[..count])
        }
    }

    struct NoRead;

    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("invalid types and sizes must fail before reading")
        }
    }

    #[test]
    fn reads_signed_decimal_widths_without_losing_scale() {
        let values = [-12_345i32, 0, 89];
        let mut bytes = values
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect::<Vec<_>>();
        bytes.push(77);
        let mut input = OneByte(Cursor::new(bytes));
        let column = read_decimal_column(&mut input, "Decimal32(2)", 3).unwrap();
        assert_eq!(column.precision(), 9);
        assert_eq!(column.scale(), 2);
        assert_eq!(column.len(), 3);
        assert!(!column.is_empty());
        assert_eq!(
            column.unscaled_values(),
            &NumericColumn::Int32(vec![-12_345, 0, 89])
        );
        assert_eq!(input.0.position(), 12);

        let column =
            read_decimal_column(&mut (-13i64).to_le_bytes().as_slice(), "Decimal(12, 3)", 1)
                .unwrap();
        assert_eq!(column.precision(), 12);
        assert_eq!(column.scale(), 3);
        assert_eq!(column.unscaled_values(), &NumericColumn::Int64(vec![-13]));

        // Decimal128 supports 38 digits, not the full 128-bit signed
        // integer range. Test the last valid negative value explicitly.
        let value = -(10i128.pow(38) - 1);
        let column =
            read_decimal_column(&mut value.to_le_bytes().as_slice(), "Decimal128(9)", 1).unwrap();
        assert_eq!(column.precision(), 38);
        assert_eq!(column.scale(), 9);
        assert_eq!(
            column.unscaled_values(),
            &NumericColumn::Int128(vec![value])
        );
        assert!(matches!(
            read_decimal_column(&mut i128::MIN.to_le_bytes().as_slice(), "Decimal128(9)", 1),
            Err(DecimalReadError::ValueExceedsPrecision {
                row: 0,
                precision: 38
            })
        ));
    }

    #[test]
    fn maps_general_decimal_precision_to_the_correct_storage_width() {
        for (data_type, precision, scale, storage) in [
            ("Decimal(1, 0)", 1, 0, "Int32"),
            ("Decimal(9, 9)", 9, 9, "Int32"),
            ("Decimal(10, 0)", 10, 0, "Int64"),
            ("Decimal(18)", 18, 0, "Int64"),
            ("Decimal(19, 2)", 19, 2, "Int128"),
            ("Decimal(38, 38)", 38, 38, "Int128"),
            ("Decimal", 10, 0, "Int64"),
            ("Decimal64(18)", 18, 18, "Int64"),
        ] {
            assert_eq!(
                parse_decimal_type(data_type),
                Some((precision, scale, storage)),
                "{data_type}"
            );
        }
    }

    #[test]
    fn keeps_explicit_width_and_precision_boundaries() {
        for (data_type, expected) in [
            ("Decimal32(9)", Some((9, 9, "Int32"))),
            ("Decimal64(18)", Some((18, 18, "Int64"))),
            ("Decimal128(38)", Some((38, 38, "Int128"))),
            ("Decimal(19)", Some((19, 0, "Int128"))),
            ("Decimal(38, 39)", None),
        ] {
            assert_eq!(parse_decimal_type(data_type), expected);
        }
    }

    #[test]
    fn rejects_invalid_and_unsupported_declarations_without_reading() {
        for data_type in [
            "Decimal32",
            "Decimal32(-1)",
            "Decimal32(+1)",
            "Decimal64(+4)",
            "Decimal(+9, 1)",
            "Decimal(9, +1)",
            "Decimal32(10)",
            "Decimal64(19)",
            "Decimal128(39)",
            "Decimal256(2)",
            "Decimal16(2)",
            "Decimal256(38, 2)",
            "Decimal(0, 0)",
            "Decimal(39, 0)",
            "Decimal(9, 10)",
            "Decimal(-1, 0)",
            "Decimal(9, -1)",
            "Decimal(9, 1, 2)",
            "Decimal(, 2)",
            "Decimal(3,)",
            "Decimal(Float64)",
        ] {
            assert!(
                matches!(
                    read_decimal_column(&mut NoRead, data_type, 1),
                    Err(DecimalReadError::InvalidType(_))
                ),
                "{data_type}"
            );
        }
    }

    #[test]
    fn rejects_oversized_and_truncated_batches() {
        assert!(matches!(
            read_decimal_column(&mut NoRead, "Decimal64(2)", 64 * 1024 * 1024 / 8 + 1),
            Err(DecimalReadError::Numeric(NumericReadError::BatchTooLarge {
                width: 8,
                ..
            }))
        ));
        assert!(matches!(
            read_decimal_column(&mut &[1, 2, 3][..], "Decimal32(0)", 1),
            Err(DecimalReadError::Numeric(NumericReadError::Io(error)))
                if error.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn empty_batch_does_not_read_values() {
        let column = read_decimal_column(&mut NoRead, "Decimal(38, 0)", 0).unwrap();
        assert!(column.is_empty());
        assert_eq!(column.unscaled_values(), &NumericColumn::Int128(Vec::new()));
    }
}

#[cfg(test)]
mod precision_validation_tests {
    use super::*;

    #[test]
    fn enforces_declared_decimal_precision_without_float_conversion() {
        let good = [99i32, -99i32, 0i32]
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect::<Vec<_>>();
        assert_eq!(
            read_decimal_column(&mut good.as_slice(), "Decimal(2,0)", 3)
                .unwrap()
                .len(),
            3
        );
        for invalid in [100i32, -100i32] {
            assert!(matches!(
                read_decimal_column(&mut invalid.to_le_bytes().as_slice(), "Decimal(2,0)", 1),
                Err(DecimalReadError::ValueExceedsPrecision {
                    row: 0,
                    precision: 2
                })
            ));
        }
    }

    #[test]
    fn validates_large_decimal_128_values() {
        let valid = 10i128.pow(19) - 1;
        assert!(
            read_decimal_column(&mut valid.to_le_bytes().as_slice(), "Decimal(19,0)", 1).is_ok()
        );
        let invalid = 10i128.pow(19);
        assert!(matches!(
            read_decimal_column(&mut invalid.to_le_bytes().as_slice(), "Decimal(19,0)", 1),
            Err(DecimalReadError::ValueExceedsPrecision {
                row: 0,
                precision: 19
            })
        ));
    }
}

#[cfg(test)]
mod ascii_decimal_type_parameters_tests {
    use super::*;

    #[test]
    fn checked_parser_preserves_exact_u8_bounds_and_strict_syntax() {
        for (text, expected) in [("0", 0), ("09", 9), ("38", 38), ("255", 255)] {
            assert_eq!(parse_decimal_type_number(text), Some(expected));
        }
        for invalid in ["", "+1", "-1", "1 ", "1.0", "256", "999999999999"] {
            assert_eq!(parse_decimal_type_number(invalid), None, "{invalid:?}");
        }
        assert_eq!(
            parse_decimal_type("Decimal(38, 18)"),
            Some((38, 18, "Int128"))
        );
        assert!(parse_decimal_type("Decimal(256, 1)").is_none());
    }
}

#[cfg(test)]
mod physical_decimal_writer_tests {
    use super::*;

    struct PartialSink(Vec<u8>);

    impl Write for PartialSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = bytes.len().min(11);
            self.0.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    #[test]
    fn signed_decimal_families_round_trip_across_multiple_blocks() {
        for (declaration, value, width) in [
            ("Decimal32(4)", -123456789i128, 4),
            ("Decimal64(10)", 123456789012345678i128, 8),
            ("Decimal128(20)", -123456789012345678901234567890i128, 16),
        ] {
            let values = vec![value; 4097];
            let mut output = PartialSink(Vec::new());
            write_decimal_values(&mut output, declaration, &values).unwrap();
            assert_eq!(output.0.len(), values.len() * width);
            let decoded = read_decimal_column(&mut output.0.as_slice(), declaration, values.len()).unwrap();
            match decoded.unscaled_values() {
                NumericColumn::Int32(inner) => assert!(inner.iter().all(|&v| i128::from(v) == value)),
                NumericColumn::Int64(inner) => assert!(inner.iter().all(|&v| i128::from(v) == value)),
                NumericColumn::Int128(inner) => assert!(inner.iter().all(|&v| v == value)),
                _ => panic!("unexpected physical Decimal type"),
            }
        }
    }

    #[test]
    fn precision_limits_fail_before_any_partial_column_output() {
        for (declaration, valid, invalid) in [
            ("Decimal32(3)", 999_999_999i128, 1_000_000_000),
            ("Decimal64(4)", -999_999_999_999_999_999i128, 1_000_000_000_000_000_000),
            ("Decimal128(0)", 10i128.pow(38) - 1, 10i128.pow(38)),
        ] {
            let mut output = Vec::new();
            write_decimal_values(&mut output, declaration, &[valid]).unwrap();
            output.clear();
            assert_eq!(
                write_decimal_values(&mut output, declaration, &[valid, invalid])
                    .unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            assert!(output.is_empty());
        }
    }

    #[test]
    fn invalid_type_and_size_preflight_do_not_write() {
        let mut output = Vec::new();
        assert!(write_decimal_values(&mut output, "Decimal256(7)", &[1]).is_err());
        assert!(write_decimal_values(&mut output, "Decimal32(10)", &[1]).is_err());
        assert!(write_decimal_values(&mut output, "Decimal(0,0)", &[1]).is_err());
        assert!(output.is_empty());
        write_decimal_values(&mut output, "Decimal64(2)", &[]).unwrap();
        assert!(output.is_empty());
    }
}
