use rusthouse_storage::{PartError, inspect_part};
use std::fmt::Write as _;
use std::path::PathBuf;

pub fn inspect_and_print_json(path: PathBuf) -> Result<(), PartError> {
    let metadata = inspect_part(path)?;
    print!(
        "{{\"path\":{},\"rows\":{},\"columns\":[",
        json_string(&metadata.path.to_string_lossy()),
        metadata.rows
    );
    print_columns_json(&metadata.columns);
    println!("]}}");
    Ok(())
}

/// Write the shared JSON representation of a part's columns.
pub(crate) fn print_columns_json(columns: &[rusthouse_storage::Column]) {
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            print!(",");
        }
        print!(
            "{{\"name\":{},\"type\":{}}}",
            json_string(&column.name),
            json_string(&column.data_type)
        );
    }
}

pub(crate) fn json_string(value: &str) -> String {
    let mut escaped_json = String::with_capacity(value.len() + 2);
    escaped_json.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped_json.push_str("\\\""),
            '\\' => escaped_json.push_str("\\\\"),
            '\n' => escaped_json.push_str("\\n"),
            '\r' => escaped_json.push_str("\\r"),
            '\t' => escaped_json.push_str("\\t"),
            '\u{08}' => escaped_json.push_str("\\b"),
            '\u{0c}' => escaped_json.push_str("\\f"),
            character if character <= '\u{1f}' => {
                write!(&mut escaped_json, "\\u{:04x}", character as u32)
                    .expect("writing to String cannot fail");
            }
            character => escaped_json.push(character),
        }
    }
    escaped_json.push('"');
    escaped_json
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_quotes_backslashes_controls_and_utf8() {
        assert_eq!(
            json_string("a\"b\\c\n\t\r\u{07}é"),
            "\"a\\\"b\\\\c\\n\\t\\r\\u0007é\""
        );
    }

    #[test]
    fn escapes_nul_and_form_feed() {
        assert_eq!(json_string("\0\u{0c}"), "\"\\u0000\\f\"");
    }
}
