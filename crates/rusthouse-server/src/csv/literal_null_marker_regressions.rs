use super::*;

#[test]
fn distinguishes_literal_backslash_n_from_null_in_csv() {
    let mut csv = CsvWriter::new(Vec::new());
    csv.write_all(b"value\n\\N\n\\\\N\n").unwrap();
    assert_eq!(csv.output, b"value\r\n\\N\r\n\"\\N\"\r\n");
}
#[test]
fn streams_wide_rows_and_rejects_width_changes() {
    let header = std::iter::repeat_n("column", 256)
        .collect::<Vec<_>>()
        .join("\t");
    let row = std::iter::repeat_n("123", 256)
        .collect::<Vec<_>>()
        .join("\t");
    let mut writer = CsvWriter::new(Vec::new());
    writer
        .write_all(format!("{header}\n{row}\n").as_bytes())
        .unwrap();
    assert_eq!(
        writer.output.iter().filter(|&&byte| byte == b'\n').count(),
        2
    );
    assert_eq!(writer.columns, Some(256));
    assert!(writer.write_all(b"one\tcolumn\n").is_err());
}

#[test]
fn csv_plain_utf8_fields_skip_escape_decoding() {
    let mut output = Vec::new();
    write_csv_field(&mut output, "café".as_bytes()).unwrap();
    assert_eq!(output, "café".as_bytes());
    output.clear();
    write_csv_field(&mut output, b"hello,world").unwrap();
    assert_eq!(output, b"\"hello,world\"");
    output.clear();
    assert!(write_csv_field(&mut output, b"\xff").is_err());
}

#[test]
fn wide_plain_csv_rows_reuse_reserved_output_capacity() {
    let large = "x".repeat(4096);
    let mut writer = CsvWriter::new(Vec::new());
    writer
        .write_all(format!("value\n{large}\n").as_bytes())
        .unwrap();
    assert_eq!(writer.output, format!("value\r\n{large}\r\n").as_bytes());
    assert!(writer.encoded.capacity() >= large.len() + 4);
}
