use rusthouse_query::{execute, parse_select};

#[test]
fn nested_text_functions_keep_unicode_and_replacement_semantics() {
    for (sql, expected) in [
        (
            "SELECT CONCAT(UPPER('Ab'), RIGHT('xyz', 2)) AS joined",
            "joined\nAByz\n",
        ),
        (
            "SELECT REPLACE(REPEAT('ab', 3), 'ba', 'X') AS changed",
            "changed\naXXb\n",
        ),
        (
            "SELECT CHAR_LENGTH(SUBSTRING('é😀xyz', 2, 3)) AS chars",
            "chars\n3\n",
        ),
        (
            "SELECT RIGHT(CONCAT('go', REPEAT('!', 3)), 4) AS suffix",
            "suffix\no!!!\n",
        ),
    ] {
        let parsed = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
        let mut output = Vec::new();
        execute(&parsed, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{sql}");
    }
}
