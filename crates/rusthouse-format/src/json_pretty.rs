//! Shared pretty-printing for complete, validated JSONL rows.
use std::io::{self, Write};

const MAX_PRETTY_JSON_DEPTH: usize = 256;

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
    const CACHED_INDENT: [u8; MAX_PRETTY_JSON_DEPTH * 2 + 3] = {
        let mut bytes = [b' '; MAX_PRETTY_JSON_DEPTH * 2 + 3];
        bytes[0] = b'\n';
        bytes
    };
    let spaces = (depth + 1) * 2;
    if spaces < CACHED_INDENT.len() {
        output.write_all(&CACHED_INDENT[..spaces + 1])
    } else {
        output.write_all(b"\n")?;
        write_spaces(output, spaces)
    }
}

fn write_padded_span<const N: usize, W: Write>(output: &mut W, span: &[u8]) -> io::Result<()> {
    let mut buffer = [0u8; N];
    buffer[..span.len()].copy_from_slice(span);
    buffer[span.len()] = b' ';
    output.write_all(&buffer[..span.len() + 1])
}

fn is_empty_json_container_pair(open: u8, close: u8) -> bool {
    matches!((open, close), (b'{', b'}') | (b'[', b']'))
}

fn validate_pretty_depth(row: &[u8]) -> io::Result<()> {
    let mut inside_string = false;
    let mut escaped = false;
    let mut depth = 0usize;
    // Match each closing delimiter against its opener before writing any bytes.
    let mut containers = [0u8; MAX_PRETTY_JSON_DEPTH];
    for &byte in row {
        if inside_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                inside_string = false;
            }
            continue;
        }
        match byte {
            b'"' => inside_string = true,
            b'{' | b'[' => {
                if depth >= MAX_PRETTY_JSON_DEPTH {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "pretty JSON nesting exceeds 256 levels",
                    ));
                }
                containers[depth] = byte;
                depth += 1;
            }
            b'}' | b']' => {
                if depth == 0 || !is_empty_json_container_pair(containers[depth - 1], byte) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unbalanced pretty JSON delimiter",
                    ));
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    if depth != 0 || inside_string {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unterminated pretty JSON container or string",
        ));
    }
    Ok(())
}

// Batch punctuation-heavy JSON output without a heap allocation per row.
struct RowWriter<'a, W: Write> {
    output: &'a mut W,
    buffer: [u8; 4096],
    used: usize,
}

impl<'a, W: Write> RowWriter<'a, W> {
    fn new(output: &'a mut W) -> Self {
        Self {
            output,
            buffer: [0; 4096],
            used: 0,
        }
    }

    fn finish(self) -> io::Result<()> {
        self.output.write_all(&self.buffer[..self.used])
    }

    fn drain(&mut self) -> io::Result<()> {
        self.output.write_all(&self.buffer[..self.used])?;
        self.used = 0;
        Ok(())
    }
}

impl<W: Write> Write for RowWriter<'_, W> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        // Large spans already have useful writer granularity. Forward
        // them intact to avoid copying and preserve long-key write shapes.
        if input.len() >= 1024 {
            if self.used != 0 {
                self.drain()?;
            }
            self.output.write_all(input)?;
            return Ok(input.len());
        }
        if input.len() > self.buffer.len() - self.used {
            self.drain()?;
        }
        self.buffer[self.used..self.used + input.len()].copy_from_slice(input);
        self.used += input.len();
        Ok(input.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drain()?;
        self.output.flush()
    }
}

/// Pretty-print a validated JSON row, preserving punctuation inside strings.
pub fn write_pretty_json_row<W: Write>(output: &mut W, row: &[u8]) -> io::Result<()> {
    // Fail before writing a prefix that would leave the destination invalid.
    validate_pretty_depth(row)?;
    let mut writer = RowWriter::new(output);
    write_pretty_json_row_into(&mut writer, row)?;
    writer.finish()
}

fn write_pretty_json_row_into<W: Write>(output: &mut W, row: &[u8]) -> io::Result<()> {
    let mut inside_json_string = false;
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut index = 0usize;
    while index < row.len() {
        if inside_json_string {
            // JSON string values may be megabytes long. Skip ordinary bytes
            // in a single span while still respecting escaped quote pairs.
            let next = row[index..]
                .iter()
                .position(|&byte| byte == b'\\' || byte == b'"')
                .unwrap_or(row.len() - index);
            index += next;
            if index == row.len() {
                break;
            }
            if row[index] == b'\\' {
                index = (index + 2).min(row.len());
            } else {
                inside_json_string = false;
                index += 1;
            }
            continue;
        }
        // Large numeric, boolean and null values contain no formatting
        // punctuation. Skip their whole span instead of matching every byte
        // through the JSON indentation state machine.
        let Some(distance) = row[index..]
            .iter()
            .position(|&byte| matches!(byte, b'"' | b'{' | b'[' | b'}' | b']' | b',' | b':'))
        else {
            break;
        };
        index += distance;
        let byte = row[index];
        match byte {
            b'"' => inside_json_string = true,
            b'{' | b'[' => {
                if depth >= MAX_PRETTY_JSON_DEPTH {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "pretty JSON nesting exceeds 256 levels",
                    ));
                }
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
                    write_padded_span::<256, _>(output, span)?;
                } else if span.len() < 1024 {
                    // Keep a separate medium-key path so short keys do not
                    // pay to initialize a larger stack buffer.
                    write_padded_span::<1024, _>(output, span)?;
                } else if span.len() < 4096 {
                    // Long JSON keys should not make two writer calls to
                    // append their single required space.
                    write_padded_span::<4096, _>(output, span)?;
                } else {
                    output.write_all(span)?;
                    output.write_all(b" ")?;
                }
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
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
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        for length in [64, 128, 250] {
            let key = "k".repeat(length);
            let row = format!("{{\"{key}\":0}}");
            let mut output = Writes(Vec::new());
            write_pretty_json_row_into(&mut output, row.as_bytes()).unwrap();
            let wanted = format!("\"{key}\": ");
            assert!(
                output
                    .0
                    .iter()
                    .any(|chunk| chunk.as_slice() == wanted.as_bytes()),
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
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        for length in [254, 255, 256, 512, 1020] {
            let key = "a".repeat(length);
            let input = format!("{{\"{key}\":1}}");
            let mut output = Calls(Vec::new());
            write_pretty_json_row_into(&mut output, input.as_bytes()).unwrap();
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
        write_pretty_json_row_into(&mut output, br#"{"a":1}"#).unwrap();
        assert!(output.0.iter().any(|chunk| chunk.as_slice() == b"\"a\": "));
        assert_eq!(output.0.concat(), b"{\n    \"a\": 1\n  }");
    }
}

#[cfg(test)]
mod pretty_depth_limit_tests {
    use super::*;

    #[test]
    fn accepts_depth_limit_and_rejects_one_more_level() {
        let input = format!("{}0{}", "[".repeat(256), "]".repeat(256));
        let mut output = Vec::new();
        write_pretty_json_row(&mut output, input.as_bytes()).unwrap();
        assert!(output.len() > input.len());

        let excess = format!("{}0{}", "[".repeat(257), "]".repeat(257));
        let error = write_pretty_json_row(&mut Vec::new(), excess.as_bytes()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[cfg(test)]
mod deep_json_indent_fast_path_tests {
    use super::*;

    #[test]
    fn deep_valid_indent_levels_need_a_single_writer_call() {
        struct Writer {
            writes: usize,
            bytes: Vec<u8>,
        }
        impl Write for Writer {
            fn write(&mut self, data: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        for depth in [0, 1, 31, 32, 33, 40, 128, 255, 256] {
            let mut writer = Writer {
                writes: 0,
                bytes: Vec::new(),
            };
            write_indent(&mut writer, depth).unwrap();
            assert_eq!(writer.writes, 1, "{depth}");
            assert_eq!(writer.bytes.len(), 1 + (depth + 1) * 2);
            assert_eq!(writer.bytes[0], b'\n');
            assert!(writer.bytes[1..].iter().all(|&byte| byte == b' '));
        }
    }
}

#[cfg(test)]
mod long_pretty_json_key_batch_tests {
    use super::*;

    struct Calls(Vec<Vec<u8>>);

    impl Write for Calls {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.push(bytes.to_vec());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn long_colon_spans_use_one_write_until_four_kibibytes() {
        for length in [1021, 1022, 2048, 4092] {
            let key = "k".repeat(length);
            let source = format!("{{\"{key}\":42}}");
            let mut writer = Calls(Vec::new());
            write_pretty_json_row_into(&mut writer, source.as_bytes()).unwrap();
            let colon = format!("\"{key}\": ");
            assert!(
                writer.0.iter().any(|call| call == colon.as_bytes()),
                "{length}"
            );
            assert_eq!(
                writer.0.concat(),
                format!("{{\n    \"{key}\": 42\n  }}").as_bytes(),
            );
        }
    }

    #[test]
    fn larger_keys_preserve_output_without_fixed_size_stack_overflow() {
        let key = "a".repeat(12_000);
        let source = format!("{{\"{key}\":1}}");
        let mut rendered = Vec::new();
        write_pretty_json_row(&mut rendered, source.as_bytes()).unwrap();
        assert_eq!(rendered, format!("{{\n    \"{key}\": 1\n  }}").as_bytes());
    }
}

#[cfg(test)]
mod atomic_pretty_depth_tests {
    use super::*;

    #[test]
    fn rejected_depth_never_leaves_partial_json_in_sink() {
        let row = format!("{}0{}", "[".repeat(257), "]".repeat(257));
        let mut output = Vec::new();
        let error = write_pretty_json_row(&mut output, row.as_bytes()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(output.is_empty());
    }

    #[test]
    fn quoted_brackets_and_escaped_quotes_do_not_count_as_depth() {
        let row = format!("{}\"[]{{}}\\\"[]\"{}", "[".repeat(256), "]".repeat(256));
        let mut output = Vec::new();
        write_pretty_json_row(&mut output, row.as_bytes()).unwrap();
        assert!(output.starts_with(b"[\n"));
    }
}

#[cfg(test)]
mod row_write_batch_tests {
    use super::*;

    struct Counted {
        calls: usize,
        bytes: Vec<u8>,
    }

    impl Write for Counted {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn punctuation_heavy_json_uses_bounded_output_writes() {
        let row = format!(
            "{{\"values\":[{}]}}",
            (0..200)
                .map(|index| format!("{{\"key\":{index}}}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut expected = Vec::new();
        write_pretty_json_row_into(&mut expected, row.as_bytes()).unwrap();
        let mut counted = Counted {
            calls: 0,
            bytes: Vec::new(),
        };
        write_pretty_json_row(&mut counted, row.as_bytes()).unwrap();
        assert_eq!(counted.bytes, expected);
        assert!(counted.calls <= 2, "writes: {}", counted.calls);
    }

    #[test]
    fn batch_writer_preserves_long_literals_and_escaped_strings() {
        let row = format!(
            "{{\"text\":\"{}\",\"escape\":\"a\\\\b\\\"c\"}}",
            "x".repeat(20_000)
        );
        let mut expected = Vec::new();
        write_pretty_json_row_into(&mut expected, row.as_bytes()).unwrap();
        let mut actual = Vec::new();
        write_pretty_json_row(&mut actual, row.as_bytes()).unwrap();
        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod large_json_string_span_tests {
    use super::*;

    #[test]
    fn preserves_long_punctuation_and_escaped_quotes_in_values() {
        let long = "café,{}[]:東京".repeat(4096);
        let escaped = r#"\"quoted\"\\path\n"#;
        let input = format!(r#"{{"value":"{long}{escaped}{long}","n":1}}"#);
        let expected = format!("{{\n    \"value\": \"{long}{escaped}{long}\",\n    \"n\": 1\n  }}");
        let mut actual = Vec::new();
        write_pretty_json_row(&mut actual, input.as_bytes()).unwrap();
        assert_eq!(actual, expected.as_bytes());
    }

    #[test]
    fn repeated_escaped_backslashes_and_quotes_do_not_end_string_early() {
        let escaped = r#"\\\""#.repeat(4096);
        let input = format!(r#"{{"payload":"{escaped}","next":true}}"#);
        let expected = format!("{{\n    \"payload\": \"{escaped}\",\n    \"next\": true\n  }}");
        let mut actual = Vec::new();
        write_pretty_json_row(&mut actual, input.as_bytes()).unwrap();
        assert_eq!(actual, expected.as_bytes());
    }
}

#[cfg(test)]
mod bulk_scalar_span_tests {
    use super::*;

    #[test]
    fn skips_large_scalar_runs_without_changing_pretty_output() {
        let large_number = "1234567890".repeat(10_000);
        let row = format!("{{\"n\":{large_number},\"ok\":true,\"empty\":null}}");
        let mut formatted = Vec::new();
        write_pretty_json_row(&mut formatted, row.as_bytes()).unwrap();
        assert_eq!(
            formatted,
            format!("{{\n    \"n\": {large_number},\n    \"ok\": true,\n    \"empty\": null\n  }}")
                .as_bytes()
        );
    }

    #[test]
    fn quoted_punctuation_survives_bulk_scalar_skipping() {
        let row = br#"{"name":"a,b:{[]}","data":[0,-12345,2.75e-4,false,null]}"#;
        let mut formatted = Vec::new();
        write_pretty_json_row(&mut formatted, row).unwrap();
        assert_eq!(
            formatted,
            b"{\n    \"name\": \"a,b:{[]}\",\n    \"data\": [\n      0,\n      -12345,\n      2.75e-4,\n      false,\n      null\n    ]\n  }"
        );
    }
}

#[cfg(test)]
mod atomic_pretty_json_validation_tests {
    use super::*;

    #[test]
    fn rejects_mismatched_delimiters_and_truncated_strings_before_output() {
        for malformed in [
            br#"{]"#.as_slice(),
            br#"{"items":[1,2}"#,
            br#"{"unclosed":"text"#,
            br#"{"escaped":"text\\"#,
            br#"]"#,
            br#"{ {"#,
        ] {
            let mut rendered = Vec::new();
            assert!(write_pretty_json_row(&mut rendered, malformed).is_err());
            assert!(rendered.is_empty(), "rejected JSON must not emit a prefix");
        }
        let mut nested = Vec::new();
        for _ in 0..256 {
            nested.push(b'[');
        }
        for _ in 0..256 {
            nested.push(b']');
        }
        let mut rendered = Vec::new();
        write_pretty_json_row(&mut rendered, &nested).unwrap();
        assert!(rendered.starts_with(b"["));
    }
}
