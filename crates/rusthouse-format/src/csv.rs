use std::io::{self, Write};

fn write_buffered_unescaped_field<const N: usize, W: Write>(
    output: &mut W,
    field: &[u8],
) -> io::Result<()> {
    let mut buffer = [0u8; N];
    buffer[0] = b'"';
    buffer[1..field.len() + 1].copy_from_slice(field);
    buffer[field.len() + 1] = b'"';
    output.write_all(&buffer[..field.len() + 2])
}

fn write_short_quoted_csv_field<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    // Worst case: every byte is a doubled quote plus two delimiters.
    let mut buffer = [0u8; 130];
    let mut used = 1;
    buffer[0] = b'"';
    for &byte in field {
        buffer[used] = byte;
        used += 1;
        if byte == b'"' {
            buffer[used] = b'"';
            used += 1;
        }
    }
    buffer[used] = b'"';
    output.write_all(&buffer[..used + 1])
}

fn write_long_escaped_field<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    // Copy ordinary spans in bulk. Only actual quotes need byte-wise work.
    // This also keeps the number of writes bounded for quote-heavy values.
    let mut buffer = [0u8; 4096];
    let mut used = 1;
    buffer[0] = b'"';
    let mut remaining = field;
    while !remaining.is_empty() {
        let plain_len = remaining
            .iter()
            .position(|&byte| byte == b'"')
            .unwrap_or(remaining.len());
        let mut plain = &remaining[..plain_len];
        while !plain.is_empty() {
            if used == buffer.len() {
                output.write_all(&buffer)?;
                used = 0;
            }
            let count = plain.len().min(buffer.len() - used);
            buffer[used..used + count].copy_from_slice(&plain[..count]);
            used += count;
            plain = &plain[count..];
        }
        remaining = &remaining[plain_len..];
        if let Some((_, rest)) = remaining.split_first() {
            if buffer.len() - used < 2 {
                output.write_all(&buffer[..used])?;
                used = 0;
            }
            buffer[used..used + 2].copy_from_slice(b"\"\"");
            used += 2;
            remaining = rest;
        }
    }
    if used == buffer.len() {
        output.write_all(&buffer)?;
        used = 0;
    }
    buffer[used] = b'"';
    output.write_all(&buffer[..used + 1])
}

/// Write one RFC 4180 quoted CSV field, doubling embedded quotes.
pub fn write_quoted_csv_field<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    if field.is_empty() {
        return output.write_all(b"\"\"");
    }
    // Short quote-free fields dominate tabular output. Emit both quotes and
    // the payload in a single write without a heap allocation.
    let has_quote = field.contains(&b'"');
    if !has_quote {
        if field.len() <= 2048 {
            // Keep the most common unescaped fields in one buffered write.
            return write_buffered_unescaped_field::<2050, _>(output, field);
        }
        if field.len() <= 4094 {
            return write_buffered_unescaped_field::<4096, _>(output, field);
        }
        if field.len() <= 8190 {
            return write_buffered_unescaped_field::<8192, _>(output, field);
        }
        // Socket writers can frame a large plain field in one vectored I/O
        // without copying its bytes into a temporary buffer.
        let slices = [
            io::IoSlice::new(b"\""),
            io::IoSlice::new(field),
            io::IoSlice::new(b"\""),
        ];
        let written = loop {
            match output.write_vectored(&slices) {
                Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
                Ok(written) => break written,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        };
        if written < field.len() + 2 {
            let payload_written = written.saturating_sub(1).min(field.len());
            output.write_all(&field[payload_written..])?;
            output.write_all(b"\"")?;
        }
        return Ok(());
    }
    if field.len() <= 64 {
        write_short_quoted_csv_field(output, field)
    } else {
        write_long_escaped_field(output, field)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_heavy_fields_preserve_all_buffer_thresholds() {
        for len in [1, 64, 65, 2048, 4094, 4095] {
            let input = vec![b'"'; len];
            let mut output = Vec::new();
            write_quoted_csv_field(&mut output, &input).unwrap();
            let mut expected = Vec::with_capacity(len * 2 + 2);
            expected.push(b'"');
            for _ in 0..len {
                expected.extend_from_slice(b"\"\"");
            }
            expected.push(b'"');
            assert_eq!(output, expected, "length {len}");
        }
    }

    #[test]
    fn doubles_embedded_and_adjacent_quotes() {
        let mut output = Vec::new();
        write_quoted_csv_field(&mut output, b"a\"\"b").unwrap();
        assert_eq!(output, b"\"a\"\"\"\"b\"");
    }

    #[test]
    fn empty_field_requires_one_writer_call() {
        struct Calls(usize);
        impl Write for Calls {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0 += 1;
                assert_eq!(bytes, b"\"\"");
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut sink = Calls(0);
        write_quoted_csv_field(&mut sink, b"").unwrap();
        assert_eq!(sink.0, 1);
    }

    #[test]
    fn short_unescaped_field_is_emitted_with_one_write() {
        struct Calls {
            count: usize,
            data: Vec<u8>,
        }

        impl Write for Calls {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.count += 1;
                self.data.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut sink = Calls {
            count: 0,
            data: Vec::new(),
        };
        write_quoted_csv_field(&mut sink, b"hello").unwrap();
        assert_eq!(sink.count, 1);
        assert_eq!(sink.data, b"\"hello\"");
    }

    #[test]
    fn four_kibibyte_plain_csv_field_uses_one_write() {
        struct Counting {
            calls: usize,
            bytes: Vec<u8>,
        }
        impl Write for Counting {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut sink = Counting {
            calls: 0,
            bytes: Vec::new(),
        };
        let value = vec![b'x'; 4094];
        write_quoted_csv_field(&mut sink, &value).unwrap();
        assert_eq!(sink.calls, 1);
        assert_eq!(sink.bytes.len(), 4096);
        assert_eq!(sink.bytes[0], b'"');
        assert_eq!(sink.bytes[4095], b'"');
        assert_eq!(&sink.bytes[1..4095], value.as_slice());
    }

    #[test]
    fn two_kibibyte_quote_free_fields_use_one_write() {
        struct Counting {
            calls: usize,
            bytes: Vec<u8>,
        }
        impl Write for Counting {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        for length in [257usize, 1024, 2048] {
            let mut writer = Counting {
                calls: 0,
                bytes: Vec::new(),
            };
            write_quoted_csv_field(&mut writer, &vec![b'x'; length]).unwrap();
            assert_eq!(writer.calls, 1);
            assert_eq!(writer.bytes.len(), length + 2);
            assert!(writer.bytes[1..length + 1].iter().all(|byte| *byte == b'x'));
        }
    }

    #[test]
    fn medium_plain_field_uses_one_write() {
        struct Count {
            calls: usize,
            bytes: Vec<u8>,
        }
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let plain = vec![b'x'; 200];
        let mut sink = Count {
            calls: 0,
            bytes: Vec::new(),
        };
        write_quoted_csv_field(&mut sink, &plain).unwrap();
        assert_eq!(sink.calls, 1);
        assert_eq!(sink.bytes.len(), 202);
        assert!(sink.bytes.starts_with(b"\"") && sink.bytes.ends_with(b"\""));
    }

    #[test]
    fn quotes_empty_and_plain_fields() {
        let mut output = Vec::new();
        write_quoted_csv_field(&mut output, b"").unwrap();
        write_quoted_csv_field(&mut output, b"plain").unwrap();
        assert_eq!(output, b"\"\"\"plain\"");
    }
    #[test]
    fn short_quote_heavy_fields_need_only_one_writer_call() {
        struct CountWrites {
            writes: usize,
            bytes: Vec<u8>,
        }
        impl Write for CountWrites {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut output = CountWrites {
            writes: 0,
            bytes: Vec::new(),
        };
        write_quoted_csv_field(&mut output, b"a\"\"b").unwrap();
        assert_eq!(output.writes, 1);
        assert_eq!(output.bytes, b"\"a\"\"\"\"b\"");
    }
}

#[cfg(test)]
mod quote_batching_tests {
    use super::*;
    struct Counted {
        writes: usize,
        bytes: Vec<u8>,
    }
    impl Write for Counted {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn quote_heavy_large_field_uses_bounded_writes() {
        let mut writer = Counted {
            writes: 0,
            bytes: Vec::new(),
        };
        let input = vec![b'"'; 10_000];
        write_quoted_csv_field(&mut writer, &input).unwrap();
        assert_eq!(writer.bytes.len(), 20_002);
        assert_eq!(writer.bytes.first(), Some(&b'"'));
        assert_eq!(writer.bytes.last(), Some(&b'"'));
        assert!(writer.bytes[1..20_001].iter().all(|&b| b == b'"'));
        assert!(writer.writes <= 6, "{} writes", writer.writes);
    }
    #[test]
    fn chunk_boundary_preserves_every_quote_and_binary_byte() {
        let input = [vec![b'a'; 4094], b"\"\\b\n".to_vec(), vec![b'"'; 5000]].concat();
        let mut output = Vec::new();
        write_quoted_csv_field(&mut output, &input).unwrap();
        let mut expected = vec![b'"'];
        for &byte in &input {
            expected.push(byte);
            if byte == b'"' {
                expected.push(b'"');
            }
        }
        expected.push(b'"');
        assert_eq!(output, expected);
    }
}

#[cfg(test)]
mod eight_kib_csv_field_tests {
    use super::*;

    struct CountWrites {
        calls: usize,
        bytes: Vec<u8>,
    }

    impl Write for CountWrites {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            self.bytes.extend_from_slice(input);
            Ok(input.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn medium_quote_free_fields_do_not_make_three_writes() {
        for width in [4095, 5000, 8190] {
            let input = vec![b'z'; width];
            let mut output = CountWrites {
                calls: 0,
                bytes: Vec::new(),
            };
            write_quoted_csv_field(&mut output, &input).unwrap();
            assert_eq!(output.calls, 1, "width={width}");
            assert_eq!(output.bytes.len(), width + 2);
            assert_eq!(&output.bytes[1..width + 1], input.as_slice());
            assert_eq!(output.bytes[0], b'"');
            assert_eq!(output.bytes[width + 1], b'"');
        }
    }
}

#[cfg(test)]
mod quote_free_threshold_tests {
    use super::*;

    #[test]
    fn all_unescaped_buffer_boundaries_preserve_csv_quoting() {
        for width in [2048, 2049, 4094, 4095, 8190, 8191] {
            let field = vec![b'a'; width];
            let mut output = Vec::new();
            write_quoted_csv_field(&mut output, &field).unwrap();
            assert_eq!(output.len(), width + 2);
            assert_eq!(output.first(), Some(&b'"'));
            assert_eq!(output.last(), Some(&b'"'));
            assert_eq!(&output[1..output.len() - 1], field.as_slice());
        }
    }
}

#[cfg(test)]
mod sparse_quoted_field_tests {
    use super::*;

    #[test]
    fn copies_large_plain_spans_and_doubles_quotes_across_chunks() {
        let input = [
            vec![b'x'; 32 * 1024 - 1],
            b"\"".to_vec(),
            vec![0xff; 32 * 1024],
            b"\"\"".to_vec(),
            vec![b'z'; 32 * 1024 + 1],
        ]
        .concat();
        let mut output = Vec::new();
        write_quoted_csv_field(&mut output, &input).unwrap();
        let mut expected = vec![b'"'];
        for &byte in &input {
            expected.push(byte);
            if byte == b'"' {
                expected.push(b'"');
            }
        }
        expected.push(b'"');
        assert_eq!(output, expected);
    }

    #[test]
    fn sparse_quoted_fields_keep_write_calls_bounded() {
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
        let input = [vec![b'a'; 100_000], b"\"".to_vec(), vec![b'b'; 100_000]].concat();
        let mut output = Counted {
            calls: 0,
            bytes: Vec::new(),
        };
        write_quoted_csv_field(&mut output, &input).unwrap();
        assert_eq!(output.bytes.len(), input.len() + 3);
        assert!(output.calls <= 51, "unexpected writes: {}", output.calls);
    }
}

#[cfg(test)]
mod vectored_csv_plain_field_tests {
    use super::*;
    use std::io::IoSlice;

    struct ShortVectoredWriter {
        data: Vec<u8>,
        max_per_call: usize,
        vector_calls: usize,
    }

    impl Write for ShortVectoredWriter {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            let bytes = input.len().min(self.max_per_call);
            self.data.extend_from_slice(&input[..bytes]);
            Ok(bytes)
        }

        fn write_vectored(&mut self, slices: &[IoSlice<'_>]) -> io::Result<usize> {
            self.vector_calls += 1;
            let mut total = 0;
            for input in slices {
                let count = input.len().min(self.max_per_call - total);
                self.data.extend_from_slice(&input[..count]);
                total += count;
                if total == self.max_per_call {
                    break;
                }
            }
            Ok(total)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn large_unescaped_fields_use_one_vectored_call_with_correct_fallback() {
        let payload = vec![b'x'; 32 * 1024];
        let mut expected = vec![b'"'];
        expected.extend_from_slice(&payload);
        expected.push(b'"');
        for max_per_call in [1, 23, 1024, usize::MAX] {
            let mut output = ShortVectoredWriter {
                data: Vec::new(),
                max_per_call,
                vector_calls: 0,
            };
            write_quoted_csv_field(&mut output, &payload).unwrap();
            assert_eq!(output.data, expected, "max_per_call={max_per_call}");
            assert_eq!(output.vector_calls, 1);
        }
        let mut fallback = Vec::new();
        write_quoted_csv_field(&mut fallback, &payload).unwrap();
        assert_eq!(fallback, expected);
    }

    #[test]
    fn vectored_write_zero_fails_without_emitting_a_false_complete_field() {
        let mut output = ShortVectoredWriter {
            data: Vec::new(), max_per_call: 0, vector_calls: 0,
        };
        assert_eq!(
            write_quoted_csv_field(&mut output, &vec![b'a'; 8191])
                .unwrap_err().kind(),
            io::ErrorKind::WriteZero
        );
        assert!(output.data.is_empty());
    }
}
