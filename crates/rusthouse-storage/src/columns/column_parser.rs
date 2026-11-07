use super::*;

pub(super) fn parse_column_count(header: &str) -> Option<usize> {
    header
        .strip_suffix(" columns:")
        .filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|value| value.parse::<usize>().ok())
}

pub(super) fn invalid_columns_format_error(line: usize, message: &'static str) -> PartError {
    PartError::InvalidColumns { line, message }
}

pub(super) fn hex_identifier_digit(
    chars: &mut std::str::Chars<'_>,
    line_number: usize,
) -> Result<u32, PartError> {
    chars
        .next()
        .and_then(|digit| digit.to_digit(16))
        .ok_or_else(|| invalid_columns_format_error(line_number, "invalid hex identifier escape"))
}

pub(super) fn append_utf8_character(name: &mut Vec<u8>, character: char) {
    let mut encoded = [0; 4];
    name.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
}

pub(super) fn append_identifier_escape(
    chars: &mut std::str::Chars<'_>,
    name: &mut Vec<u8>,
    line_number: usize,
) -> Result<(), PartError> {
    let escaped = chars
        .next()
        .ok_or_else(|| invalid_columns_format_error(line_number, "trailing identifier escape"))?;
    let decoded = match escaped {
        'x' => {
            let high = hex_identifier_digit(chars, line_number)?;
            let low = hex_identifier_digit(chars, line_number)?;
            (high * 16 + low) as u8
        }
        'N' => return Ok(()),
        '\\' => b'\\',
        '`' => b'`',
        '\'' => b'\'',
        '"' => b'"',
        '/' => b'/',
        '=' => b'=',
        'n' => b'\n',
        'r' => b'\r',
        't' => b'\t',
        '0' => b'\0',
        'a' => b'\x07',
        'b' => b'\x08',
        'e' => b'\x1b',
        'f' => b'\x0c',
        'v' => b'\x0b',
        _ => {
            name.push(b'\\');
            append_utf8_character(name, escaped);
            return Ok(());
        }
    };
    name.push(decoded);
    Ok(())
}

pub(super) fn finish_column(name: Vec<u8>, data_type: &str, line_number: usize) -> Result<Column, PartError> {
    if name.is_empty() || data_type.is_empty() || data_type.trim() != data_type {
        return Err(invalid_columns_format_error(
            line_number,
            "empty column name or invalid type",
        ));
    }
    let name = String::from_utf8(name)
        .map_err(|_| invalid_columns_format_error(line_number, "column name is not valid UTF-8"))?;
    Ok(Column {
        name,
        data_type: data_type.to_owned(),
    })
}

pub(super) fn parse_column(line: &str, line_number: usize) -> Result<Column, PartError> {
    let mut chars = line
        .strip_prefix('`')
        .ok_or_else(|| {
            invalid_columns_format_error(line_number, "expected a backtick-quoted column name")
        })?
        .chars();
    let mut name = Vec::new();

    while let Some(ch) = chars.next() {
        match ch {
            '`' if chars.as_str().starts_with('`') => {
                chars.next();
                name.push(b'`');
            }
            '`' => {
                let data_type = chars.as_str().strip_prefix(' ').ok_or_else(|| {
                    invalid_columns_format_error(line_number, "expected a space after column name")
                })?;
                return finish_column(name, data_type, line_number);
            }
            '\\' => append_identifier_escape(&mut chars, &mut name, line_number)?,
            _ => {
                append_utf8_character(&mut name, ch);
            }
        }
    }

    Err(invalid_columns_format_error(
        line_number,
        "unterminated column name",
    ))
}

