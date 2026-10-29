use super::*;

#[test]
fn doubles_adjacent_quotes_in_quoted_csv_fields() {
    let mut output = Vec::new();
    write_quoted_field(&mut output, b"a\"\"b").unwrap();
    assert_eq!(output, b"\"a\"\"\"\"b\"");
}

#[test]
fn decodes_escaped_rows_and_quotes_csv_special_characters() {
    let mut output = Vec::new();
    {
        let mut csv = CsvWriter::new(&mut output);
        csv.write_all(b"name\tmessage\tmissing\n").unwrap();
        csv.write_all(b"service\tcomma,quote\"\n").unwrap_err(); // row width mismatch
    }
    assert_eq!(output, b"name,message,missing\r\n");

    let mut csv = CsvWriter::new(Vec::new());
    csv.write_all(b"message\tnull\n").unwrap();
    csv.write_all(b"hello\\tworld, \"yes\"\t\\N\n").unwrap();
    assert_eq!(
        csv.output,
        b"message,null\r\n\"hello\tworld, \"\"yes\"\"\",\\N\r\n"
    );
}

#[test]
fn handles_fragmented_tsv_input_and_rejects_bad_escapes() {
    let mut csv = CsvWriter::new(Vec::new());
    csv.write_all(b"a\tb\nleft\\").unwrap();
    csv.write_all(b"nright\t42\n").unwrap();
    assert_eq!(csv.output, b"a,b\r\n\"left\nright\",42\r\n");

    let mut csv = CsvWriter::new(Vec::new());
    assert!(csv.write_all(b"column\ninvalid\\q\n").is_err());
}

#[test]
fn quotes_decoded_tabs_as_one_csv_field() {
    let mut writer = CsvWriter::new(Vec::new());
    writer.write_all(b"value\nleft\\tright\n").unwrap();
    assert_eq!(writer.output, b"value\r\n\"left\tright\"\r\n");
}

#[test]
fn writes_null_markers_and_plain_fields() {
    let mut output = Vec::new();
    write_csv_field(&mut output, b"\\N").unwrap();
    write_csv_field(&mut output, b"plain").unwrap();
    assert_eq!(output, b"\\Nplain");
}

#[test]
fn writes_large_plain_fields_in_one_bulk_call() {
    struct CountingWriter {
        calls: usize,
        bytes: Vec<u8>,
    }
    impl Write for CountingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut output = CountingWriter {
        calls: 0,
        bytes: Vec::new(),
    };
    let field = vec![b'x'; 32 * 1024];
    write_csv_field(&mut output, &field).unwrap();
    assert_eq!(output.calls, 1);
    assert_eq!(output.bytes, field);
}

#[test]
fn invalid_late_field_does_not_write_partial_csv_row() {
    let mut writer = CsvWriter::new(Vec::new());
    writer.write_all(b"first\tsecond\n").unwrap();
    let good = writer.output.clone();
    assert_eq!(
        writer.write_all(b"ok\tbad\\q\n").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(writer.output, good);

    let mut writer = CsvWriter::new(Vec::new());
    assert!(writer.write_all(b"bad\\q\n").is_err());
    assert!(writer.output.is_empty());
    assert_eq!(writer.columns, None);
}

#[test]
fn reuses_encoded_capacity_across_csv_rows() {
    let mut writer = CsvWriter::new(Vec::new());
    let row = vec![b'x'; 32 * 1024];
    writer.write_all(&row).unwrap();
    writer.write_all(b"\n").unwrap();
    let capacity = writer.encoded.capacity();
    writer.write_all(b"short\n").unwrap();
    assert_eq!(writer.encoded.capacity(), capacity);
    assert!(writer.output.ends_with(b"short\r\n"));
}

#[test]
fn caps_incomplete_rows_to_one_mib() {
    let mut csv = CsvWriter::new(Vec::new());
    assert!(csv.write_all(&vec![b'x'; MAX_LINE_BYTES + 1]).is_err());
}
