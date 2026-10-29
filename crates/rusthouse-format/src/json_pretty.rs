//! Shared pretty-printing for complete, validated JSONL rows.
use std::io::{self, Write};

const INDENT_CHUNK_BYTES: usize = 64;

fn write_spaces<W: Write>(output: &mut W, mut remaining: usize) -> io::Result<()> {
    let spaces = [b' '; INDENT_CHUNK_BYTES];
    while remaining > 0 {
        let count = remaining.min(spaces.len());
        output.write_all(&spaces[..count])?;
        remaining -= count;
    }
    Ok(())
}

fn write_indent<W: Write>(output: &mut W, depth: usize) -> io::Result<()> {
    const SHORT_INDENT: &[u8; INDENT_CHUNK_BYTES + 1] =
        b"\n                                                                ";
    let spaces = (depth + 1) * 2;
    if spaces <= INDENT_CHUNK_BYTES {
        output.write_all(&SHORT_INDENT[..spaces + 1])
    } else {
        output.write_all(b"\n")?;
        write_spaces(output, spaces)
    }
}

fn is_empty_pair(open: u8, close: u8) -> bool {
    matches!((open, close), (b'{', b'}') | (b'[', b']'))
}

/// Pretty-print a validated JSON row, preserving punctuation inside strings.
pub fn write_pretty_json_row<W: Write>(output: &mut W, row: &[u8]) -> io::Result<()> {
    let mut quoted = false;
    let mut escaped = false;
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, &byte) in row.iter().enumerate() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => {
                output.write_all(&row[start..=index])?;
                depth += 1;
                let empty = row
                    .get(index + 1)
                    .is_some_and(|&next| is_empty_pair(byte, next));
                if !empty {
                    write_indent(output, depth)?;
                }
                start = index + 1;
            }
            b'}' | b']' => {
                output.write_all(&row[start..index])?;
                depth = depth.saturating_sub(1);
                let empty = index > 0 && is_empty_pair(row[index - 1], byte);
                if !empty {
                    write_indent(output, depth)?;
                }
                output.write_all(&row[index..=index])?;
                start = index + 1;
            }
            b',' => {
                output.write_all(&row[start..=index])?;
                write_indent(output, depth)?;
                start = index + 1;
            }
            b':' => {
                let span = &row[start..=index];
                if span.len() < 64 {
                    let mut buffer = [0u8; 64];
                    buffer[..span.len()].copy_from_slice(span);
                    buffer[span.len()] = b' ';
                    output.write_all(&buffer[..span.len() + 1])?;
                } else {
                    output.write_all(span)?;
                    output.write_all(b" ")?;
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    output.write_all(&row[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_indentation_across_fixed_chunks() {
        for depth in [0usize, 1, 40] {
            let mut output = Vec::new();
            write_indent(&mut output, depth).unwrap();
            assert_eq!(output[0], b'\n');
            assert_eq!(output.len(), 1 + (depth + 1) * 2);
            assert!(output[1..].iter().all(|&byte| byte == b' '));
        }
    }

    #[test]
    fn short_indent_uses_one_write() {
        struct CountingWriter {
            writes: usize,
            bytes: Vec<u8>,
        }
        impl Write for CountingWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut output = CountingWriter {
            writes: 0,
            bytes: Vec::new(),
        };
        write_indent(&mut output, 1).unwrap();
        assert_eq!(output.writes, 1);
        assert_eq!(output.bytes, b"\n    ");
    }

    #[test]
    fn keeps_empty_containers_on_one_line() {
        let mut output = Vec::new();
        write_pretty_json_row(&mut output, br#"{"empty":{},"items":[]}"#).unwrap();
        assert_eq!(output, b"{\n    \"empty\": {},\n    \"items\": []\n  }");
    }

    #[test]
    fn leaves_json_string_punctuation_unchanged() {
        let mut output = Vec::new();
        write_pretty_json_row(&mut output, br#"{"text":"a,{}","ok":null}"#).unwrap();
        assert_eq!(output, b"{\n    \"text\": \"a,{}\",\n    \"ok\": null\n  }");
    }

    #[test]
    fn short_json_key_and_colon_use_one_write() {
        struct Calls(Vec<Vec<u8>>);
        impl Write for Calls {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.push(buf.to_vec());
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut output = Calls(Vec::new());
        write_pretty_json_row(&mut output, br#"{"a":1}"#).unwrap();
        assert!(output.0.iter().any(|chunk| chunk.as_slice() == b"\"a\": "));
        assert_eq!(output.0.concat(), b"{\n    \"a\": 1\n  }");
    }
}
