//! Shared pretty-printing for complete, validated JSONL rows.
use std::io::{self, Write};

fn write_spaces<W: Write>(output: &mut W, mut remaining: usize) -> io::Result<()> {
    let spaces = [b' '; 64];
    while remaining > 0 {
        let count = remaining.min(spaces.len());
        output.write_all(&spaces[..count])?;
        remaining -= count;
    }
    Ok(())
}

fn write_indent<W: Write>(output: &mut W, depth: usize) -> io::Result<()> {
    const SHORT_INDENT: &[u8; 65] =
        b"\n                                                                ";
    let spaces = (depth + 1) * 2;
    if spaces <= 64 {
        output.write_all(&SHORT_INDENT[..spaces + 1])
    } else {
        output.write_all(b"\n")?;
        write_spaces(output, spaces)
    }
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
                let empty = (byte == b'{' && row.get(index + 1) == Some(&b'}'))
                    || (byte == b'[' && row.get(index + 1) == Some(&b']'));
                if !empty {
                    write_indent(output, depth)?;
                }
                start = index + 1;
            }
            b'}' | b']' => {
                output.write_all(&row[start..index])?;
                depth = depth.saturating_sub(1);
                let empty = index > 0
                    && ((byte == b'}' && row[index - 1] == b'{')
                        || (byte == b']' && row[index - 1] == b'['));
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
                output.write_all(&row[start..=index])?;
                output.write_all(b" ")?;
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
}
