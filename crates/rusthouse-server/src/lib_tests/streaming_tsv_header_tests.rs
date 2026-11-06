use super::*;

#[test]
fn suppresses_split_tsv_header_without_buffering_or_draining_rows() {
    let mut output = LimitedWriter::new();
    {
        let mut without_header = WithoutHeader::new(&mut output);
        without_header.write_all(b"first\tsecond").unwrap();
        without_header.write_all(b"\n1\tone\n2\ttwo\n").unwrap();
    }
    assert_eq!(output.data, b"1\tone\n2\ttwo\n");
    assert!(!output.exceeded);
}
