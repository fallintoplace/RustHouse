use super::{Cell, write_value};
use std::io::{self, Write};

#[derive(Default)]
struct CountingWriter {
    writes: usize,
    bytes: Vec<u8>,
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn preserves_tsv_escaping_across_plain_spans() {
    let mut output = Vec::new();
    write_value(
        &mut output,
        &Cell::Text(b"one\ttwo\\three\n\0\x01".to_vec()),
    )
    .unwrap();
    assert_eq!(output, br"one\ttwo\\three\n\0\x01");
}

#[test]
fn emits_long_unescaped_strings_with_one_write() {
    let mut output = CountingWriter::default();
    write_value(&mut output, &Cell::Text(vec![b'a'; 16 * 1024])).unwrap();
    assert_eq!(output.bytes, vec![b'a'; 16 * 1024]);
    assert_eq!(output.writes, 1);
}
