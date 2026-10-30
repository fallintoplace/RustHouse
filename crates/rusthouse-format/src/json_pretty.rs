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

fn is_empty_json_container_pair(open: u8, close: u8) -> bool {
    matches!((open, close), (b'{', b'}') | (b'[', b']'))
}

/// Pretty-print a validated JSON row, preserving punctuation inside strings.
pub fn write_pretty_json_row<W: Write>(output: &mut W, row: &[u8]) -> io::Result<()> {
    let mut inside_json_string = false;
    let mut previous_byte_was_escape = false;
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, &byte) in row.iter().enumerate() {
        if inside_json_string {
            if previous_byte_was_escape {
                previous_byte_was_escape = false;
            } else if byte == b'\\' {
                previous_byte_was_escape = true;
            } else if byte == b'"' {
                inside_json_string = false;
            }
            continue;
        }
        match byte {
            b'"' => inside_json_string = true,
            b'{' | b'[' => {
                output.write_all(&row[start..=index])?;
                depth += 1;
                let empty = row
                    .get(index + 1)
                    .is_some_and(|&next| is_empty_json_container_pair(byte, next));
                if !empty {
                    write_indent(output, depth)?;
                }
                start = index + 1;
            }
            b'}' | b']' => {
                output.write_all(&row[start..index])?;
                depth = depth.saturating_sub(1);
                let empty = index > 0 && is_empty_json_container_pair(row[index - 1], byte);
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
                if span.len() < 256 {
                    // One writer call for longer JSON keys (up to 254 bytes
                    // plus a colon) and the required whitespace.
                    let mut buffer = [0u8; 256];
                    buffer[..span.len()].copy_from_slice(span);
                    buffer[span.len()] = b' ';
                    output.write_all(&buffer[..span.len() + 1])?;
                } else if span.len() < 1024 {
                    // Keep a separate medium-key path so short keys do not
                    // pay to initialize a larger stack buffer.
                    let mut buffer = [0u8; 1024];
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
    fn medium_json_key_and_colon_are_batched_together() {
        struct Writes(Vec<Vec<u8>>);
        impl Write for Writes {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.push(bytes.to_vec());
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        for length in [64, 128, 250] {
            let key = "k".repeat(length);
            let row = format!("{{\"{key}\":0}}");
            let mut output = Writes(Vec::new());
            write_pretty_json_row(&mut output, row.as_bytes()).unwrap();
            let wanted = format!("\"{key}\": ");
            assert!(
                output.0.iter().any(|chunk| chunk.as_slice() == wanted.as_bytes()),
                "medium key was not emitted in one write"
            );
        }
    }

    #[test]
    fn medium_long_json_keys_still_need_one_output_write() {
        struct Calls(Vec<Vec<u8>>);
        impl Write for Calls {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.push(bytes.to_vec());
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        for length in [254, 255, 256, 512, 1020] {
            let key = "a".repeat(length);
            let input = format!("{{\"{key}\":1}}");
            let mut output = Calls(Vec::new());
            write_pretty_json_row(&mut output, input.as_bytes()).unwrap();
            let encoded = format!("\"{key}\": ");
            assert!(
                output.0.iter().any(|bytes| bytes == encoded.as_bytes()),
                "key of {length} bytes was split into multiple writes",
            );
        }
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
