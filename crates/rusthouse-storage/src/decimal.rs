use crate::{NumericColumn, NumericReadError, read_numeric_column};
use std::error::Error;
use std::fmt;
use std::io::Read;

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
                write!(f, "Decimal value at row {row} exceeds precision {precision}")
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

fn decimal_storage_type(precision: u8) -> &'static str {
    if precision <= 9 {
        "Int32"
    } else if precision <= 18 {
        "Int64"
    } else {
        "Int128"
    }
}

fn parse_decimal_component(value: &str) -> Option<u8> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u8>().ok()
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
    let scale = parse_decimal_component(arguments.strip_suffix(')')?)?;
    checked_decimal_spec(precision, scale)
}

fn parse_general_decimal(inner: &str) -> Option<(u8, u8, &'static str)> {
    let (precision, scale) = match inner.split_once(',') {
        Some((precision, scale)) => (precision.trim(), scale.trim()),
        None => (inner.trim(), "0"),
    };
    let precision = parse_decimal_component(precision)?;
    let scale = parse_decimal_component(scale)?;
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

        let value = i128::MIN;
        let column =
            read_decimal_column(&mut value.to_le_bytes().as_slice(), "Decimal128(9)", 1).unwrap();
        assert_eq!(column.precision(), 38);
        assert_eq!(column.scale(), 9);
        assert_eq!(
            column.unscaled_values(),
            &NumericColumn::Int128(vec![value])
        );
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
        assert_eq!(read_decimal_column(&mut good.as_slice(), "Decimal(2,0)", 3).unwrap().len(), 3);
        for invalid in [100i32, -100i32] {
            assert!(matches!(
                read_decimal_column(&mut invalid.to_le_bytes().as_slice(), "Decimal(2,0)", 1),
                Err(DecimalReadError::ValueExceedsPrecision { row: 0, precision: 2 })
            ));
        }
    }

    #[test]
    fn validates_large_decimal_128_values() {
        let valid = 10i128.pow(19) - 1;
        assert!(read_decimal_column(&mut valid.to_le_bytes().as_slice(), "Decimal(19,0)", 1).is_ok());
        let invalid = 10i128.pow(19);
        assert!(matches!(
            read_decimal_column(&mut invalid.to_le_bytes().as_slice(), "Decimal(19,0)", 1),
            Err(DecimalReadError::ValueExceedsPrecision { row: 0, precision: 19 })
        ));
    }
}
