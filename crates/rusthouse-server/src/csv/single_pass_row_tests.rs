use super::*;
#[test]
fn encodes_multi_column_rows_and_rejects_bad_width_without_leaking_output() {
    let mut csv = CsvWriter::new(Vec::new());
    csv.write_all(b"a\tb\n1\t2\n").unwrap();
    assert_eq!(csv.output, b"a,b\r\n1,2\r\n");
    assert_eq!(
        csv.write_all(b"only-one\n").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(csv.output, b"a,b\r\n1,2\r\n");
}
