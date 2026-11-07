//! Shared limits and identifier checks for local schema metadata.
//! Keep catalog discovery and read-only HTTP table enumeration consistent.

pub const MAX_SCHEMA_FILE_BYTES: u64 = 32 * 1024;
pub const MAX_SCHEMA_COLUMN_COUNT: usize = 128;
const MAX_SCHEMA_IDENTIFIER_BYTES: usize = 128;

/// Accept only portable, unquoted ASCII SQL identifiers in schema filenames.
pub fn valid_schema_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= MAX_SCHEMA_IDENTIFIER_BYTES
        && matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_portable_schema_identifiers() {
        for name in ["x", "events_2026", "_metadata", &"z".repeat(128)] {
            assert!(valid_schema_identifier(name), "{name}");
        }
    }

    #[test]
    fn rejects_names_outside_the_schema_contract() {
        for name in [
            "", "9events", "has-dash", "has.dot", "two words", "../secret",
            "not\\valid", "é", &"z".repeat(129),
        ] {
            assert!(!valid_schema_identifier(name), "{name}");
        }
    }

    #[test]
    fn schema_bounds_remain_stable() {
        assert_eq!(MAX_SCHEMA_FILE_BYTES, 32 * 1024);
        assert_eq!(MAX_SCHEMA_COLUMN_COUNT, 128);
    }
}
