mod exists;
mod input;
mod runner;
mod scalar_derived_cross;
mod scalar_subquery;

pub(super) use runner::run_sql_args;

use self::input::*;

use crate::{
    csv, csv_no_header, html, json_array, json_compact_each_row, json_seq, jsonl, markdown,
    sql_values, vertical,
};
use rusthouse_query::{
    ParseError, Source, execute, explain_join, explain_select, has_cte_max_join,
    has_derived_join_count, has_explicit_join, has_implicit_join, has_join_correlated_avg,
    has_join_correlated_min, has_join_correlated_sum, has_nested_join_sum, has_top_level_exists,
    parse_correlated_exists, parse_cte_max_join, parse_ddl, parse_delete,
    parse_derived_join_count_with_schema, parse_derived_join_sum_with_schema, parse_insert,
    parse_join, parse_join_correlated_avg_with_schema, parse_join_correlated_exists_with_schema,
    parse_join_correlated_min_with_schema, parse_join_correlated_sum_with_schema,
    parse_join_with_schema, parse_nested_join_sum_with_schema, parse_select, parse_update,
    split_clickhouse_format,
};
use std::ffi::OsString;
use std::path::PathBuf;

const MAX_SQL_BYTES: usize = 64 * 1024;
const MAX_SCRIPT_BYTES: usize = 1024 * 1024;
const MAX_SCRIPT_STATEMENTS: usize = 512;

#[derive(Clone, Copy)]
pub(super) enum SqlOutputFormat {
    Tsv,
    TsvNoHeader,
    Csv,
    CsvNoHeader,
    Jsonl,
    Json,
    JsonPretty,
    JsonCompactEachRow,
    JsonSeq,
    Markdown,
    Html,
    SqlValues,
    Vertical,
}

impl SqlOutputFormat {
    fn from_arg(value: Option<OsString>) -> Result<Self, String> {
        match value.and_then(|value| value.into_string().ok()).as_deref() {
            Some("tsv") => Ok(Self::Tsv),
            Some("TabSeparatedWithNames") => Ok(Self::Tsv),
            Some("tsv-no-header") => Ok(Self::TsvNoHeader),
            Some("TabSeparated") => Ok(Self::TsvNoHeader),
            Some("csv") => Ok(Self::Csv),
            Some("CSVWithNames") => Ok(Self::Csv),
            Some("csv-no-header") => Ok(Self::CsvNoHeader),
            Some("CSV") => Ok(Self::CsvNoHeader),
            Some("jsonl") => Ok(Self::Jsonl),
            Some("JSONStringsEachRow") => Ok(Self::Jsonl),
            Some("json") => Ok(Self::Json),
            Some("json-pretty") => Ok(Self::JsonPretty),
            Some("json-compact-each-row") => Ok(Self::JsonCompactEachRow),
            Some("JSONCompactStringsEachRow") => Ok(Self::JsonCompactEachRow),
            Some("json-seq") => Ok(Self::JsonSeq),
            Some("markdown") => Ok(Self::Markdown),
            Some("html") => Ok(Self::Html),
            Some("sql-values") => Ok(Self::SqlValues),
            Some("vertical") => Ok(Self::Vertical),
            _ => Err(
                "--format must be tsv, tsv-no-header, csv, csv-no-header, jsonl, json, json-pretty, json-compact-each-row, json-seq, markdown, html, sql-values, or vertical"
                    .into(),
            ),
        }
    }
}

/// Split a bounded SQL script on semicolons outside quoted strings.
/// Strip line comments; the single-statement SQL parser intentionally does not accept them.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_splitter_respects_strings_escapes_and_comments() {
        assert_eq!(
            split_sql_script(
                "-- header\nINSERT INTO t VALUES ('a;b', 'it''s'); -- skip\nSELECT 1;"
            )
            .unwrap(),
            ["INSERT INTO t VALUES ('a;b', 'it''s')", "SELECT 1"]
        );
        assert_eq!(
            split_sql_script(r"INSERT INTO t VALUES ('it\'s;x'); SELECT 2").unwrap(),
            [r"INSERT INTO t VALUES ('it\'s;x')", "SELECT 2"]
        );
        assert!(split_sql_script("-- only comment").is_err());
        assert!(split_sql_script("SELECT 'unfinished").is_err());
        assert!(split_sql_script(&"x".repeat(4097)).is_ok());
        assert!(split_sql_script(&"x".repeat(MAX_SQL_BYTES + 1)).is_err());
        assert_eq!(
            split_sql_script(&"SELECT 1;".repeat(256)).unwrap().len(),
            256
        );
        assert!(split_sql_script(&"SELECT 1;".repeat(MAX_SCRIPT_STATEMENTS + 1)).is_err());
    }

    #[test]
    fn row_cap_parser_preserves_boundary_and_error_rules() {
        assert_eq!(parse_max_rows(Some(OsString::from("0003"))), Ok(3));
        assert_eq!(parse_max_rows(Some(OsString::from("0"))), Ok(0));
        assert_eq!(
            parse_max_rows(None).unwrap_err(),
            "missing --max-rows value"
        );
        for invalid in ["", "-1", "+2", "1.5", "no"] {
            assert_eq!(
                parse_max_rows(Some(OsString::from(invalid))).unwrap_err(),
                "--max-rows must be a non-negative integer"
            );
        }
    }

    #[test]
    fn bounded_input_accepts_exact_limit_and_rejects_invalid_input() {
        assert_eq!(
            read_sql_input(&vec![b'x'; MAX_SQL_BYTES][..], "SQL file", "SQL file")
                .unwrap()
                .len(),
            MAX_SQL_BYTES
        );
        assert_eq!(
            read_sql_input(&vec![b'x'; MAX_SQL_BYTES + 1][..], "SQL file", "SQL file").unwrap_err(),
            "SQL file exceeds the 65536-byte statement limit"
        );
        assert_eq!(
            read_sql_input(&[0xff][..], "SQL file", "SQL file").unwrap_err(),
            "SQL file must be valid UTF-8"
        );
        assert_eq!(
            read_sql_input(&[0xff][..], "SQL from stdin", "SQL from stdin").unwrap_err(),
            "SQL from stdin must be valid UTF-8"
        );
    }

    #[test]
    fn format_parser_accepts_all_supported_formats() {
        for value in [
            "tsv",
            "tsv-no-header",
            "csv",
            "csv-no-header",
            "jsonl",
            "json",
            "json-pretty",
            "json-compact-each-row",
            "markdown",
            "sql-values",
            "vertical",
        ] {
            assert!(SqlOutputFormat::from_arg(Some(value.into())).is_ok());
        }
    }

    #[test]
    fn format_parser_rejects_missing_and_unknown_formats() {
        assert!(SqlOutputFormat::from_arg(None).is_err());
        assert!(SqlOutputFormat::from_arg(Some("parquet".into())).is_err());
    }

    #[test]
    fn sql_frontend_rejects_duplicate_flags_and_invalid_row_caps() {
        fn error(args: &[&str]) -> String {
            run_sql_args(args.iter().copied().map(OsString::from)).unwrap_err()
        }

        assert_eq!(
            error(&["SELECT 1", "--format", "json", "--format", "csv"]),
            "duplicate --format option"
        );
        assert_eq!(
            error(&["SELECT 1", "--max-rows", "-1"]),
            "--max-rows must be a non-negative integer"
        );
        assert_eq!(
            error(&["SELECT 1", "--time", "--time"]),
            "duplicate --time option"
        );
        assert_eq!(
            error(&["SELECT 1", "--show-ast", "--time"]),
            "--time cannot be combined with --show-ast"
        );
        assert_eq!(
            error(&["CREATE TABLE t (id UInt64)", "--catalog", ".", "--time"]),
            "--time cannot be combined with --catalog"
        );
        assert_eq!(
            error(&["SELECT 1", "--file", "other.sql"]),
            "provide SQL text or --file, not both"
        );
    }
}

#[cfg(test)]
mod utf8_bom_regressions {
    use super::*;

    #[test]
    fn accepts_bom_prefixed_sql_files_and_scripts() {
        assert_eq!(
            read_sql_input("\u{feff}SELECT 42".as_bytes(), "test", "test").unwrap(),
            "SELECT 42"
        );
        assert_eq!(
            split_sql_script("\u{feff}SELECT 1; SELECT 2;").unwrap(),
            vec!["SELECT 1", "SELECT 2"]
        );
    }

    #[test]
    fn keeps_bom_inside_quoted_sql_data() {
        assert_eq!(
            split_sql_script("SELECT '\u{feff}value';").unwrap(),
            vec!["SELECT '\u{feff}value'"]
        );
    }
}

#[cfg(test)]
mod clickhouse_query_flag_parity_tests {
    use super::*;
    #[test]
    fn supports_clickhouse_query_flag_and_short_alias() {
        assert!(run_sql_args(["--query", "SELECT 42"].into_iter().map(OsString::from)).is_ok());
        assert!(run_sql_args(["-q", "SELECT 42"].into_iter().map(OsString::from)).is_ok());
    }
    #[test]
    fn rejects_conflicting_and_missing_query_values() {
        assert_eq!(
            run_sql_args(["--query"].into_iter().map(OsString::from)).unwrap_err(),
            "missing --query SQL text"
        );
        assert!(
            run_sql_args(
                ["--query", "SELECT 1", "-q", "SELECT 2"]
                    .into_iter()
                    .map(OsString::from)
            )
            .is_err()
        );
        assert!(
            run_sql_args(
                ["SELECT 1", "--query", "SELECT 2"]
                    .into_iter()
                    .map(OsString::from)
            )
            .is_err()
        );
        assert!(
            run_sql_args(
                ["--query", "SELECT 1", "--file", "query.sql"]
                    .into_iter()
                    .map(OsString::from)
            )
            .is_err()
        );
    }
}
