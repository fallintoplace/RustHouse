//! Typed ClickHouse HTTP query parameter substitution for read-only SQL.
//! Parameter names, declared types, bounds, and quoted values are validated
//! before SQL parsing. A value is never inserted as raw SQL syntax.

const MAX_SQL_BYTES: usize = 64 * 1024;
const MAX_BINDINGS: usize = 128;

pub(super) fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && name.len() <= 64
}

fn literal(kind: &str, input: &str) -> Result<String, &'static str> {
    let integer_bounds: Option<(i128, i128)> = match kind {
        "Int8" => Some((i8::MIN.into(), i8::MAX.into())),
        "Int16" => Some((i16::MIN.into(), i16::MAX.into())),
        "Int32" => Some((i32::MIN.into(), i32::MAX.into())),
        "Int64" => Some((i64::MIN.into(), i64::MAX.into())),
        "UInt8" => Some((0, u8::MAX.into())),
        "UInt16" => Some((0, u16::MAX.into())),
        "UInt32" => Some((0, u32::MAX.into())),
        // Current scalar SQL expressions are bounded to Int64. Reject
        // UInt64 values beyond this range instead of losing precision.
        "UInt64" => Some((0, i64::MAX.into())),
        "String" => None,
        _ => return Err("unsupported typed SQL parameter type"),
    };
    if kind == "String" {
        if input.contains('\0') {
            return Err("NUL is unsupported in string SQL parameters");
        }
        let mut quoted = String::with_capacity(input.len().saturating_add(2));
        quoted.push('\'');
        for ch in input.chars() {
            match ch {
                '\'' => quoted.push_str("''"),
                '\\' => quoted.push_str("\\\\"),
                _ => quoted.push(ch),
            }
            if quoted.len() > MAX_SQL_BYTES {
                return Err("bound SQL statement too large");
            }
        }
        quoted.push('\'');
        return Ok(quoted);
    }
    if input.is_empty()
        || input.len() > 40
        || !input
            .bytes()
            .enumerate()
            .all(|(index, byte)| byte.is_ascii_digit() || (index == 0 && byte == b'-'))
    {
        return Err("invalid integer SQL parameter value");
    }
    let value = input
        .parse::<i128>()
        .map_err(|_| "invalid integer SQL parameter value")?;
    let (minimum, maximum) = integer_bounds.expect("non-string types have integer bounds");
    if value < minimum || value > maximum {
        return Err("integer SQL parameter outside declared type");
    }
    Ok(value.to_string())
}

/// Substitute only placeholders outside SQL string literals and comments.
pub(super) fn bind(sql: &str, params: &[(String, String)]) -> Result<String, &'static str> {
    if params.len() > MAX_BINDINGS {
        return Err("too many typed SQL parameters");
    }
    let bytes = sql.as_bytes();
    let mut output = String::with_capacity(sql.len());
    let mut used = vec![false; params.len()];
    let mut cursor = 0;
    let mut copied = 0;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' => {
                cursor += 1;
                let mut closed = false;
                while cursor < bytes.len() {
                    if bytes[cursor] == b'\\'
                        && matches!(bytes.get(cursor + 1), Some(b'\'' | b'\\'))
                    {
                        cursor += 2;
                    } else if bytes[cursor] == b'\''
                        && bytes.get(cursor + 1) == Some(&b'\'')
                    {
                        cursor += 2;
                    } else if bytes[cursor] == b'\'' {
                        cursor += 1;
                        closed = true;
                        break;
                    } else {
                        cursor += 1;
                    }
                }
                if !closed {
                    return Err("unterminated SQL string literal");
                }
            }
            b'-' if bytes.get(cursor + 1) == Some(&b'-') => {
                cursor += 2;
                while cursor < bytes.len() && bytes[cursor] != b'\n' {
                    cursor += 1;
                }
            }
            b'#' => {
                while cursor < bytes.len() && bytes[cursor] != b'\n' {
                    cursor += 1;
                }
            }
            b'/' if bytes.get(cursor + 1) == Some(&b'*') => {
                cursor += 2;
                let mut closed = false;
                while cursor + 1 < bytes.len() {
                    if bytes[cursor] == b'*' && bytes[cursor + 1] == b'/' {
                        cursor += 2;
                        closed = true;
                        break;
                    }
                    cursor += 1;
                }
                if !closed {
                    return Err("unterminated SQL block comment");
                }
            }
            b'{' => {
                let end = bytes[cursor + 1..]
                    .iter()
                    .position(|byte| *byte == b'}')
                    .map(|index| cursor + 1 + index)
                    .ok_or("unclosed typed SQL parameter")?;
                let declaration = &sql[cursor + 1..end];
                let (name, kind) = declaration
                    .split_once(':')
                    .ok_or("typed SQL parameter requires a type")?;
                if !valid_name(name) {
                    return Err("invalid typed SQL parameter name");
                }
                let index = params
                    .iter()
                    .position(|(candidate, _)| candidate == name)
                    .ok_or("missing typed SQL parameter value")?;
                let replacement = literal(kind, &params[index].1)?;
                output.push_str(&sql[copied..cursor]);
                output.push_str(&replacement);
                used[index] = true;
                cursor = end + 1;
                copied = cursor;
                if output.len() > MAX_SQL_BYTES {
                    return Err("bound SQL statement too large");
                }
            }
            _ => cursor += 1,
        }
    }
    if used.iter().any(|used| !used) {
        return Err("unused typed SQL parameter value");
    }
    output.push_str(&sql[copied..]);
    if output.len() > MAX_SQL_BYTES {
        return Err("bound SQL statement too large");
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_bound_values_without_interpreting_sql() {
        assert_eq!(
            bind(
                "SELECT {id:UInt8}, {name:String}",
                &[("id".into(), "23".into()), ("name".into(), "O'Reilly\\x".into())],
            )
            .unwrap(),
            "SELECT 23, 'O''Reilly\\\\x'"
        );
    }

    #[test]
    fn checks_unsigned_signed_and_parameter_usage() {
        for (sql, bindings) in [
            ("SELECT {n:UInt8}", vec![("n".into(), "-1".into())]),
            ("SELECT {n:Int8}", vec![("n".into(), "128".into())]),
            ("SELECT {n:UInt64}", vec![("n".into(), "18446744073709551615".into())]),
            ("SELECT {n:Float64}", vec![("n".into(), "1".into())]),
            ("SELECT {n:Int32}", vec![]),
            ("SELECT 1", vec![("n".into(), "1".into())]),
            ("SELECT {name", vec![("name".into(), "x".into())]),
        ] {
            assert!(bind(sql, &bindings).is_err(), "{sql}");
        }
    }

    #[test]
    fn skips_placeholders_in_strings_and_comments() {
        let sql = "SELECT '{a:Int8}' AS label /* {a:Int8} */ -- {a:Int8}\n";
        assert_eq!(bind(sql, &[]).unwrap(), sql);
    }
}
