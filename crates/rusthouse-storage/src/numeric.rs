use crate::fixed_width::MAX_BATCH_BYTES;
use std::error::Error;
use std::fmt;
use std::io::{self, Read};

const READ_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, PartialEq)]
pub enum NumericColumn {
    UInt8(Vec<u8>),
    UInt16(Vec<u16>),
    UInt32(Vec<u32>),
    UInt64(Vec<u64>),
    UInt128(Vec<u128>),
    Int8(Vec<i8>),
    Int16(Vec<i16>),
    Int32(Vec<i32>),
    Int64(Vec<i64>),
    Int128(Vec<i128>),
    Float32(Vec<f32>),
    Float64(Vec<f64>),
}

impl NumericColumn {
    pub fn len(&self) -> usize {
        match self {
            Self::UInt8(values) => values.len(),
            Self::UInt16(values) => values.len(),
            Self::UInt32(values) => values.len(),
            Self::UInt64(values) => values.len(),
            Self::UInt128(values) => values.len(),
            Self::Int8(values) => values.len(),
            Self::Int16(values) => values.len(),
            Self::Int32(values) => values.len(),
            Self::Int64(values) => values.len(),
            Self::Int128(values) => values.len(),
            Self::Float32(values) => values.len(),
            Self::Float64(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug)]
pub enum NumericReadError {
    UnsupportedType(String),
    BatchTooLarge { rows: usize, width: usize },
    Io(io::Error),
}

impl fmt::Display for NumericReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedType(data_type) => {
                write!(f, "unsupported plain numeric type: {data_type}")
            }
            Self::BatchTooLarge { rows, width } => write!(
                f,
                "{rows} rows of {width}-byte values exceed the 64 MiB batch limit"
            ),
            Self::Io(source) => write!(f, "unable to read numeric column: {source}"),
        }
    }
}

impl Error for NumericReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(source) => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for NumericReadError {
    fn from(source: io::Error) -> Self {
        Self::Io(source)
    }
}

fn check_batch_size(rows: usize, width: usize) -> Result<(), NumericReadError> {
    if rows > MAX_BATCH_BYTES / width {
        return Err(NumericReadError::BatchTooLarge { rows, width });
    }
    Ok(())
}

pub(crate) fn numeric_storage_type(data_type: &str) -> Option<&str> {
    match data_type {
        "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128" | "Int8" | "Int16" | "Int32"
        | "Int64" | "Int128" | "Float32" | "Float64" => Some(data_type),
        "Date" => Some("UInt16"),
        "Date32" => Some("Int32"),
        "DateTime" => Some("UInt32"),
        _ => {
            if let Some(zone) = data_type
                .strip_prefix("DateTime('")
                .and_then(|value| value.strip_suffix("')"))
            {
                return (!zone.is_empty() && !zone.contains('\'')).then_some("UInt32");
            }

            let value = data_type.strip_prefix("DateTime64(")?.strip_suffix(')')?;
            let (scale, zone) = match value.split_once(',') {
                Some((scale, zone)) => (scale, Some(zone.trim())),
                None => (value, None),
            };
            if scale.parse::<u8>().ok()? > 9 {
                return None;
            }
            if let Some(zone) = zone {
                let zone = zone
                    .strip_prefix('\'')
                    .and_then(|zone| zone.strip_suffix('\''))?;
                if zone.is_empty() || zone.contains('\'') {
                    return None;
                }
            }
            Some("Int64")
        }
    }
}

pub fn read_numeric_column<R: Read>(
    reader: &mut R,
    data_type: &str,
    rows: usize,
) -> Result<NumericColumn, NumericReadError> {
    macro_rules! decode {
        ($value:ty, $variant:ident) => {{
            let width = std::mem::size_of::<$value>();
            check_batch_size(rows, width)?;
            Ok(NumericColumn::$variant(read_fixed(
                reader,
                rows,
                <$value>::from_le_bytes,
            )?))
        }};
    }

    match numeric_storage_type(data_type).unwrap_or(data_type) {
        "UInt8" => {
            check_batch_size(rows, 1)?;
            // A byte-wide column already has its final in-memory representation.
            // Read directly into the output instead of copying through 16 KiB chunks.
            let mut values = vec![0u8; rows];
            reader.read_exact(&mut values)?;
            Ok(NumericColumn::UInt8(values))
        }
        "UInt16" => decode!(u16, UInt16),
        "UInt32" => decode!(u32, UInt32),
        "UInt64" => decode!(u64, UInt64),
        "UInt128" => decode!(u128, UInt128),
        "Int8" => decode!(i8, Int8),
        "Int16" => decode!(i16, Int16),
        "Int32" => decode!(i32, Int32),
        "Int64" => decode!(i64, Int64),
        "Int128" => decode!(i128, Int128),
        "Float32" => decode!(f32, Float32),
        "Float64" => decode!(f64, Float64),
        _ => Err(NumericReadError::UnsupportedType(data_type.to_owned())),
    }
}

fn read_fixed<R: Read, T, const WIDTH: usize>(
    reader: &mut R,
    rows: usize,
    parse: fn([u8; WIDTH]) -> T,
) -> io::Result<Vec<T>> {
    if rows == 0 {
        return Ok(Vec::new());
    }
    let mut values = Vec::with_capacity(rows);
    let mut buffer = [0u8; READ_BUFFER_BYTES];

    while values.len() < rows {
        let count = (rows - values.len()).min(buffer.len() / WIDTH);
        let size = count * WIDTH;
        reader.read_exact(&mut buffer[..size])?;

        values.extend(
            buffer[..size]
                .chunks_exact(WIDTH)
                .map(|bytes| parse(bytes.try_into().expect("exact fixed-width numeric chunk"))),
        );
    }

    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn decode(data_type: &str, bytes: &[u8], rows: usize) -> NumericColumn {
        read_numeric_column(&mut Cursor::new(bytes), data_type, rows).unwrap()
    }

    #[test]
    fn numeric_batch_limit_scales_with_element_width() {
        for width in [1, 2, 4, 8, 16] {
            assert!(check_batch_size(MAX_BATCH_BYTES / width, width).is_ok());
            assert!(matches!(
                check_batch_size(MAX_BATCH_BYTES / width + 1, width),
                Err(NumericReadError::BatchTooLarge { width: actual, .. }) if actual == width
            ));
        }
    }

    #[test]
    fn decodes_signed_and_unsigned_values_in_little_endian_order() {
        assert_eq!(
            decode("UInt8", &[0, 255], 2),
            NumericColumn::UInt8(vec![0, 255])
        );
        assert_eq!(
            decode("Int8", &[0, 255], 2),
            NumericColumn::Int8(vec![0, -1])
        );
        assert_eq!(
            decode("UInt16", &[0x34, 0x12, 0xff, 0xff], 2),
            NumericColumn::UInt16(vec![0x1234, u16::MAX])
        );
        assert_eq!(
            decode("Int16", &[0x00, 0x80, 0xff, 0x7f], 2),
            NumericColumn::Int16(vec![i16::MIN, i16::MAX])
        );
        assert_eq!(
            decode("UInt32", &0x1234_5678u32.to_le_bytes(), 1),
            NumericColumn::UInt32(vec![0x1234_5678])
        );
        assert_eq!(
            decode("Int32", &(-70_000i32).to_le_bytes(), 1),
            NumericColumn::Int32(vec![-70_000])
        );
        assert_eq!(
            decode("UInt64", &u64::MAX.to_le_bytes(), 1),
            NumericColumn::UInt64(vec![u64::MAX])
        );
        assert_eq!(
            decode("Int64", &i64::MIN.to_le_bytes(), 1),
            NumericColumn::Int64(vec![i64::MIN])
        );
    }

    #[test]
    fn reads_128_bit_integers_in_little_endian_order() {
        let unsigned = [0x1122_3344_5566_7788_99aa_bbcc_ddee_ff00u128, u128::MAX];
        let bytes = unsigned
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(
            decode("UInt128", &bytes, 2),
            NumericColumn::UInt128(unsigned.to_vec())
        );

        let signed = [i128::MIN, -1, i128::MAX];
        let bytes = signed
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(
            decode("Int128", &bytes, 3),
            NumericColumn::Int128(signed.to_vec())
        );
    }

    #[test]
    fn preserves_float_bits_including_negative_zero_and_nan_payloads() {
        let negative_zero = f32::from_bits(0x8000_0000);
        let nan = f32::from_bits(0x7fc0_1234);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&negative_zero.to_bits().to_le_bytes());
        bytes.extend_from_slice(&nan.to_bits().to_le_bytes());

        let NumericColumn::Float32(values) = decode("Float32", &bytes, 2) else {
            panic!("wrong type");
        };
        assert_eq!(values[0].to_bits(), negative_zero.to_bits());
        assert_eq!(values[1].to_bits(), nan.to_bits());

        let value = f64::from_bits(0xfff0_0000_0000_0000);
        let NumericColumn::Float64(values) = decode("Float64", &value.to_bits().to_le_bytes(), 1)
        else {
            panic!("wrong type");
        };
        assert_eq!(values[0].to_bits(), value.to_bits());
    }

    #[test]
    fn decodes_date_and_timestamp_physical_values() {
        assert_eq!(
            decode("Date", &19_000u16.to_le_bytes(), 1),
            NumericColumn::UInt16(vec![19_000])
        );
        assert_eq!(
            decode("Date32", &(-100i32).to_le_bytes(), 1),
            NumericColumn::Int32(vec![-100])
        );
        for ty in ["DateTime", "DateTime('UTC')", "DateTime('Europe/Berlin')"] {
            assert_eq!(
                decode(ty, &1_700_000_000u32.to_le_bytes(), 1),
                NumericColumn::UInt32(vec![1_700_000_000]),
                "{ty}"
            );
        }
        for ty in ["DateTime64(3)", "DateTime64(9, 'UTC')"] {
            assert_eq!(
                decode(ty, &(-1_234_567i64).to_le_bytes(), 1),
                NumericColumn::Int64(vec![-1_234_567]),
                "{ty}"
            );
        }
    }

    #[test]
    fn rejects_invalid_date_time_precision_and_timezones() {
        for ty in [
            "DateTime('')",
            "DateTime('UTC' extra)",
            "DateTime64()",
            "DateTime64(-1)",
            "DateTime64(10)",
            "DateTime64(foo)",
            "DateTime64(3,)",
            "DateTime64(3, '')",
            "DateTime64(3, UTC)",
            "DateTime64(3, 'UTC' extra)",
        ] {
            assert!(
                matches!(
                    read_numeric_column(&mut &[][..], ty, 0),
                    Err(NumericReadError::UnsupportedType(_))
                ),
                "{ty}"
            );
        }
    }

    #[test]
    fn leaves_remaining_bytes_for_the_next_batch() {
        let mut reader = Cursor::new([11u8, 12, 13]);
        assert_eq!(
            read_numeric_column(&mut reader, "UInt8", 2).unwrap(),
            NumericColumn::UInt8(vec![11, 12])
        );
        assert_eq!(reader.position(), 2);
        assert_eq!(
            read_numeric_column(&mut reader, "UInt8", 1).unwrap(),
            NumericColumn::UInt8(vec![13])
        );
    }

    struct OneByteReader<R>(R);

    impl<R: Read> Read for OneByteReader<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let count = buf.len().min(1);
            self.0.read(&mut buf[..count])
        }
    }

    #[test]
    fn reads_values_across_short_reads_and_buffer_boundaries() {
        let expected = (0..2_100).map(|n| n as u64 * 1000).collect::<Vec<_>>();
        let bytes = expected
            .iter()
            .flat_map(|number| number.to_le_bytes())
            .collect::<Vec<_>>();
        let mut reader = OneByteReader(Cursor::new(bytes));
        let column = read_numeric_column(&mut reader, "UInt64", expected.len()).unwrap();
        assert_eq!(column, NumericColumn::UInt64(expected));
    }

    #[test]
    fn rejects_truncated_128_bit_values() {
        let mut data = i128::MIN.to_le_bytes().to_vec();
        data.extend_from_slice(&[1, 2]);
        let error = read_numeric_column(&mut data.as_slice(), "Int128", 2).unwrap_err();
        assert!(matches!(
            error,
            NumericReadError::Io(source) if source.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn rejects_truncated_values() {
        let mut bytes = 7u64.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[1, 2, 3]);
        let error = read_numeric_column(&mut Cursor::new(bytes), "UInt64", 2).unwrap_err();
        assert!(matches!(
            error,
            NumericReadError::Io(source) if source.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn bounds_allocation_and_rejects_unsupported_types_without_reading() {
        struct NoRead;

        impl Read for NoRead {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("must reject before reading")
            }
        }

        assert!(matches!(
            read_numeric_column(&mut NoRead, "UInt64", MAX_BATCH_BYTES / 8 + 1),
            Err(NumericReadError::BatchTooLarge { width: 8, .. })
        ));
        assert!(matches!(
            read_numeric_column(&mut NoRead, "UInt128", MAX_BATCH_BYTES / 16 + 1),
            Err(NumericReadError::BatchTooLarge { width: 16, .. })
        ));
        assert!(matches!(
            read_numeric_column(&mut NoRead, "UInt8", MAX_BATCH_BYTES + 1),
            Err(NumericReadError::BatchTooLarge { width: 1, .. })
        ));
        assert!(matches!(
            read_numeric_column(&mut NoRead, "Nullable(UInt8)", 1),
            Err(NumericReadError::UnsupportedType(_))
        ));
    }

    #[test]
    fn handles_empty_batches_without_reading() {
        let column = read_numeric_column(&mut Cursor::new(Vec::<u8>::new()), "Int32", 0).unwrap();
        assert!(column.is_empty());
        assert_eq!(column.len(), 0);
    }
    #[test]
    fn reads_wide_numeric_values_in_one_64k_batch() {
        struct Counting<'a> {
            inner: &'a [u8],
            reads: usize,
        }
        impl Read for Counting<'_> {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                self.reads += 1;
                self.inner.read(out)
            }
        }
        let values = vec![1234u16; 32_768];
        let bytes = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let mut reader = Counting {
            inner: &bytes,
            reads: 0,
        };
        assert_eq!(
            read_numeric_column(&mut reader, "UInt16", values.len()).unwrap(),
            NumericColumn::UInt16(values)
        );
        assert_eq!(reader.reads, 1);
    }

    #[test]
    fn reads_large_u8_columns_directly_with_one_bulk_read() {
        struct CountingReader<R> {
            inner: R,
            calls: usize,
        }

        impl<R: Read> Read for CountingReader<R> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.calls += 1;
                self.inner.read(buf)
            }
        }

        let input = vec![42u8; 65_536];
        let mut reader = CountingReader {
            inner: Cursor::new(input.as_slice()),
            calls: 0,
        };
        let column = read_numeric_column(&mut reader, "UInt8", input.len()).unwrap();
        assert_eq!(reader.calls, 1);
        assert_eq!(column, NumericColumn::UInt8(input));
    }

    #[test]
    fn zero_length_u8_batch_does_not_read() {
        let mut reader = Cursor::new(&[][..]);
        assert_eq!(
            read_numeric_column(&mut reader, "UInt8", 0).unwrap(),
            NumericColumn::UInt8(vec![])
        );
    }

    #[test]
    fn direct_fixed_width_decode_keeps_signed_endian_bytes() {
        let numbers = [i64::MIN, -1, 0, i64::MAX];
        let raw = numbers
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(
            read_numeric_column(&mut raw.as_slice(), "Int64", numbers.len()).unwrap(),
            NumericColumn::Int64(numbers.to_vec())
        );
    }
}

#[cfg(test)]
mod zero_row_numeric_tests {
    use super::*;

    #[test]
    fn zero_rows_do_not_read_the_numeric_stream() {
        struct NoRead;
        impl Read for NoRead {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("zero-row numeric read consumed input");
            }
        }
        let values = read_numeric_column(&mut NoRead, "UInt64", 0).unwrap();
        assert!(matches!(values, NumericColumn::UInt64(v) if v.is_empty()));
    }
}
