use super::*;

pub(super) fn parse_max_rows(value: Option<OsString>) -> Result<usize, String> {
    let value = value.ok_or("missing --max-rows value")?;
    let value = value
        .to_str()
        .ok_or("--max-rows must be a non-negative integer")?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("--max-rows must be a non-negative integer".into());
    }
    value
        .parse::<usize>()
        .map_err(|_| "--max-rows is too large".into())
}

pub(super) fn read_sql_input(
    input: impl std::io::Read,
    source: &str,
    label: &str,
) -> Result<String, String> {
    use std::io::Read;

    let mut bytes = Vec::new();
    input
        .take((MAX_SQL_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {source}: {error}"))?;
    if bytes.len() > MAX_SQL_BYTES {
        return Err(format!(
            "{label} exceeds the {MAX_SQL_BYTES}-byte statement limit"
        ));
    }
    let input = String::from_utf8(bytes).map_err(|_| format!("{label} must be valid UTF-8"))?;
    if let Some(without_bom) = input.strip_prefix('\u{feff}') {
        Ok(without_bom.to_owned())
    } else {
        Ok(input)
    }
}

fn push_sql_statement(statements: &mut Vec<String>, statement: &str) -> Result<(), String> {
    if statement.is_empty() {
        return Ok(());
    }
    if statement.len() > MAX_SQL_BYTES {
        return Err(format!("SQL script statement exceeds {MAX_SQL_BYTES} bytes"));
    }
    if statements.len() >= MAX_SCRIPT_STATEMENTS {
        return Err(format!(
            "SQL script supports at most {MAX_SCRIPT_STATEMENTS} statements"
        ));
    }
    statements.push(statement.to_owned());
    Ok(())
}

pub(super) fn split_sql_script(input: &str) -> Result<Vec<String>, String> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let bytes = input.as_bytes();
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut segment = 0;
    let mut index = 0;
    let mut quoted = false;
    while index < bytes.len() {
        if bytes[index] == b'\\' && quoted && matches!(bytes.get(index + 1), Some(b'\\' | b'\'')) {
            index += 2;
            continue;
        }
        if bytes[index] == b'\'' {
            if quoted && bytes.get(index + 1) == Some(&b'\'') {
                index += 2;
                continue;
            }
            quoted = !quoted;
            index += 1;
            continue;
        }
        if !quoted && bytes[index] == b'-' && bytes.get(index + 1) == Some(&b'-') {
            current.push_str(&input[segment..index]);
            current.push(' ');
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            segment = index;
            continue;
        }
        if !quoted && bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            current.push_str(&input[segment..index]);
            current.push(' ');
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            if index + 1 >= bytes.len() {
                return Err("unterminated SQL block comment".into());
            }
            index += 2;
            segment = index;
            continue;
        }
        if !quoted && bytes[index] == b';' {
            let statement = if current.is_empty() {
                input[segment..index].trim()
            } else {
                current.push_str(&input[segment..index]);
                current.trim()
            };
            push_sql_statement(&mut statements, statement)?;
            current.clear();
            index += 1;
            segment = index;
            continue;
        }
        index += 1;
    }
    if quoted {
        return Err("unterminated SQL string in script".into());
    }
    let statement = if current.is_empty() {
        input[segment..].trim()
    } else {
        current.push_str(&input[segment..]);
        current.trim()
    };
    push_sql_statement(&mut statements, statement)?;
    if statements.is_empty() {
        return Err("SQL script contains no statements".into());
    }
    Ok(statements)
}

pub(super) fn read_script(path: &std::path::Path) -> Result<Vec<String>, String> {
    use std::io::Read;

    let file = std::fs::File::open(path)
        .map_err(|error| format!("cannot open SQL script {}: {error}", path.display()))?;
    let capacity = file
        .metadata()
        .map(|metadata| metadata.len().min(MAX_SCRIPT_BYTES as u64) as usize)
        .unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    file.take((MAX_SCRIPT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read SQL script {}: {error}", path.display()))?;
    if bytes.len() > MAX_SCRIPT_BYTES {
        return Err("SQL script exceeds the 1 MiB limit".into());
    }
    let input =
        String::from_utf8(bytes).map_err(|_| "SQL script must be valid UTF-8".to_string())?;
    split_sql_script(&input)
}

#[cfg(test)]
mod block_comment_tests {
    use super::split_sql_script;

    #[test]
    fn plain_statements_and_semicolons_in_quotes_keep_their_values() {
        assert_eq!(
            split_sql_script(" SELECT 1; SELECT 'a;b'; SELECT 3 ").unwrap(),
            vec!["SELECT 1", "SELECT 'a;b'", "SELECT 3"]
        );
    }

    #[test]
    fn ignores_comment_delimiters_inside_sql_scripts() {
        assert_eq!(
            split_sql_script("SELECT 1 /* ; ; */; /* comment */ SELECT 2;").unwrap(),
            vec!["SELECT 1", "SELECT 2"]
        );
        assert_eq!(
            split_sql_script("SELECT '/* ; */' AS text;").unwrap(),
            vec!["SELECT '/* ; */' AS text"]
        );
    }

    #[test]
    fn rejects_unterminated_sql_block_comments() {
        assert!(split_sql_script("SELECT 1; /* unterminated").is_err());
        assert!(split_sql_script("/* comment only */").is_err());
    }
}

#[cfg(test)]
mod script_size_limit_tests {
    use super::read_script;
    use std::fs;

    #[test]
    fn reads_scripts_larger_than_the_old_256_kib_limit() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-sql-script-size-{}-{}",
            std::process::id(),
            line!()
        ));
        let script = format!("{};", "x".repeat(3500)).repeat(90);
        assert!(script.len() > 256 * 1024);
        fs::write(&path, script).unwrap();
        let result = read_script(&path);
        fs::remove_file(path).unwrap();
        assert_eq!(result.unwrap().len(), 90);
    }
}
