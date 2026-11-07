//! Shared Markdown output primitives for CLI and HTTP responses.
use std::io::{self, Write};

/// Write a separator for the supplied number of Markdown columns.
///
/// Keep the common <=64-column path in a stack buffer to minimize writes.
pub fn write_markdown_header_separator<W: Write>(
    output: &mut W,
    columns: usize,
) -> io::Result<()> {
    if columns <= 64 {
        let mut framed = [0u8; 1 + 64 * 6 + 1];
        framed[0] = b'|';
        let mut used = 1;
        for _ in 0..columns {
            framed[used..used + 6].copy_from_slice(b" --- |");
            used += 6;
        }
        framed[used] = b'\n';
        return output.write_all(&framed[..used + 1]);
    }
    output.write_all(b"|")?;
    for _ in 0..columns {
        output.write_all(b" --- |")?;
    }
    output.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_both_separator_paths() {
        for width in [0, 1, 8, 64, 65, 130] {
            let mut out = Vec::new();
            write_markdown_header_separator(&mut out, width).unwrap();
            assert_eq!(out, format!("|{}\n", " --- |".repeat(width)).into_bytes());
        }
    }

    #[test]
    fn propagates_write_errors() {
        struct Fail;
        impl Write for Fail {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("injected failure"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        for width in [1, 64, 65] {
            assert!(write_markdown_header_separator(&mut Fail, width).is_err());
        }
    }
}
