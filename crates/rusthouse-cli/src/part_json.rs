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
    for (index, column) in metadata.columns.iter().enumerate() {
        if index > 0 {
            print!(",");
        }
        print!(
            "{{\"name\":{},\"type\":{}}}",
            json_string(&column.name),
            json_string(&column.data_type)
        );
    }
    println!("]}}");
    Ok(())
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0c}' => escaped.push_str("\\f"),
            character if character <= '\u{1f}' => {
                write!(&mut escaped, "\\u{:04x}", character as u32)
                    .expect("writing to String cannot fail");
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
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
