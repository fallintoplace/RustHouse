use crate::{csv, csv_no_header, json_array, jsonl, markdown, vertical};
use rusthouse_query::{execute, explain_select, parse_ddl, parse_select};
use std::ffi::OsString;
use std::path::PathBuf;

const MAX_SQL_BYTES: usize = 4096;

#[derive(Clone, Copy)]
enum SqlOutputFormat {
    Tsv,
    TsvNoHeader,
    Csv,
    CsvNoHeader,
    Jsonl,
    Json,
    Markdown,
    Vertical,
}

impl SqlOutputFormat {
    fn from_arg(value: Option<OsString>) -> Result<Self, String> {
        match value.and_then(|value| value.into_string().ok()).as_deref() {
            Some("tsv") => Ok(Self::Tsv),
            Some("tsv-no-header") => Ok(Self::TsvNoHeader),
            Some("csv") => Ok(Self::Csv),
            Some("csv-no-header") => Ok(Self::CsvNoHeader),
            Some("jsonl") => Ok(Self::Jsonl),
            Some("json") => Ok(Self::Json),
            Some("markdown") => Ok(Self::Markdown),
            Some("vertical") => Ok(Self::Vertical),
            _ => Err(
                "--format must be tsv, tsv-no-header, csv, csv-no-header, jsonl, json, markdown, or vertical"
                    .into(),
            ),
        }
    }
}

fn read_sql_input(input: impl std::io::Read, source: &str, label: &str) -> Result<String, String> {
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
    String::from_utf8(bytes).map_err(|_| format!("{label} must be valid UTF-8"))
}

pub(super) fn run_sql_args(mut args: impl Iterator<Item = OsString>) -> Result<(), String> {
    let mut query = None;
    let mut file = None;
    let mut read_stdin = false;
    let mut part = None;
    let mut catalog = None;
    let mut format = None;
    let mut output_path = None;
    let mut show_ast = false;
    let mut explain = false;
    let mut time = false;
    let mut max_rows = None;
    while let Some(arg) = args.next() {
        if arg.to_str() == Some("--explain") {
            if explain {
                return Err("duplicate --explain option".into());
            }
            explain = true;
            continue;
        }
        if arg.to_str() == Some("--show-ast") {
            if show_ast {
                return Err("duplicate --show-ast option".into());
            }
            show_ast = true;
            continue;
        }
        if arg.to_str() == Some("--time") {
            if time {
                return Err("duplicate --time option".into());
            }
            time = true;
            continue;
        }
        if arg.to_str() == Some("--stdin") {
            if read_stdin {
                return Err("duplicate --stdin option".into());
            }
            read_stdin = true;
            continue;
        }
        if arg.to_str() == Some("--format") {
            if format.is_some() {
                return Err("duplicate --format option".into());
            }
            format = Some(SqlOutputFormat::from_arg(args.next())?);
        } else if arg.to_str() == Some("--max-rows") {
            if max_rows.is_some() {
                return Err("duplicate --max-rows option".into());
            }
            let value = args.next().ok_or("missing --max-rows value")?;
            let value = value
                .to_str()
                .ok_or("--max-rows must be a non-negative integer")?;
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("--max-rows must be a non-negative integer".into());
            }
            max_rows = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| "--max-rows is too large")?,
            );
        } else if arg.to_str() == Some("--output") {
            if output_path.is_some() {
                return Err("duplicate --output option".into());
            }
            output_path = Some(PathBuf::from(args.next().ok_or("missing --output path")?));
        } else if arg.to_str() == Some("--part") {
            if part.is_some() {
                return Err("duplicate --part option".into());
            }
            part = Some(PathBuf::from(
                args.next().ok_or("missing --part directory")?,
            ));
        } else if arg.to_str() == Some("--catalog") {
            if catalog.is_some() {
                return Err("duplicate --catalog option".into());
            }
            catalog = Some(PathBuf::from(
                args.next().ok_or("missing --catalog directory")?,
            ));
        } else if arg.to_str() == Some("--file") {
            if file.is_some() || query.is_some() {
                return Err("provide SQL text or --file, not both".into());
            }
            file = Some(PathBuf::from(args.next().ok_or("missing SQL file")?));
        } else if query.is_none() && file.is_none() {
            query = Some(
                arg.into_string()
                    .map_err(|_| "SQL statement must be valid UTF-8")?,
            );
        } else {
            return Err("unexpected SQL argument".into());
        }
    }
    if read_stdin && (query.is_some() || file.is_some()) {
        return Err("provide SQL text, --file, or --stdin, not both".into());
    }
    let query = if read_stdin {
        read_sql_input(std::io::stdin(), "SQL from stdin", "SQL from stdin")?
    } else if let Some(file) = file {
        let input = std::fs::File::open(&file)
            .map_err(|error| format!("cannot open {}: {error}", file.display()))?;
        read_sql_input(input, &file.display().to_string(), "SQL file")?
    } else {
        query.ok_or("missing SQL statement")?
    };
    if let Some(catalog_path) = catalog {
        if explain {
            return Err("--explain is only supported for SELECT".into());
        }
        if time {
            return Err("--time cannot be combined with --catalog".into());
        }
        if let Some(ddl) = parse_ddl(&query).map_err(|error| error.to_string())? {
            if part.is_some() || output_path.is_some() || format.is_some() || max_rows.is_some() {
                return Err("DDL cannot use --part, --output, --format, or --max-rows".into());
            }
            if show_ast {
                println!("{ddl:#?}");
                return Ok(());
            }
            return crate::catalog::apply(&catalog_path, &ddl);
        }
        return Err("--catalog is only supported for DDL statements".into());
    }
    let mut select = parse_select(&query).map_err(|error| error.to_string())?;
    if explain {
        if show_ast || time || format.is_some() || output_path.is_some() {
            return Err(
                "--explain cannot be combined with --show-ast, --time, --format, or --output"
                    .into(),
            );
        }
        if let Some(cap) = max_rows {
            select.limit = Some(select.limit.map_or(cap, |limit| limit.min(cap)));
        }
        println!("Logical plan (no data read):\n{}", explain_select(&select));
        return Ok(());
    }
    if show_ast {
        if time {
            return Err("--time cannot be combined with --show-ast".into());
        }
        if output_path.is_some() {
            return Err("--output cannot be combined with --show-ast".into());
        }
        println!("{select:#?}");
        return Ok(());
    }
    if let Some(cap) = max_rows {
        select.limit = Some(select.limit.map_or(cap, |limit| limit.min(cap)));
    }
    let mut output: Box<dyn std::io::Write> = match &output_path {
        Some(path) => Box::new(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|error| format!("cannot create {}: {error}", path.display()))?,
        ),
        None => Box::new(std::io::stdout()),
    };
    let start = std::time::Instant::now();
    let result = match format.unwrap_or(SqlOutputFormat::Tsv) {
        SqlOutputFormat::Tsv => execute(&select, part.as_deref(), &mut output),
        SqlOutputFormat::TsvNoHeader => execute(
            &select,
            part.as_deref(),
            &mut csv_no_header::WithoutHeader::new(&mut output),
        ),
        SqlOutputFormat::Csv => execute(
            &select,
            part.as_deref(),
            &mut csv::CsvWriter::new(&mut output),
        ),
        SqlOutputFormat::CsvNoHeader => execute(
            &select,
            part.as_deref(),
            &mut csv::CsvWriter::new(csv_no_header::WithoutHeader::new(&mut output)),
        ),
        SqlOutputFormat::Markdown => execute(
            &select,
            part.as_deref(),
            &mut markdown::MarkdownWriter::new(&mut output),
        ),
        SqlOutputFormat::Vertical => execute(
            &select,
            part.as_deref(),
            &mut vertical::VerticalWriter::new(&mut output),
        ),
        SqlOutputFormat::Jsonl => execute(
            &select,
            part.as_deref(),
            &mut jsonl::JsonlWriter::new(&mut output),
        ),
        SqlOutputFormat::Json => {
            let mut writer = json_array::JsonArrayWriter::new(&mut output);
            execute(&select, part.as_deref(), &mut writer).and_then(|()| {
                writer
                    .finish()
                    .map(|_| ())
                    .map_err(rusthouse_query::ExecutionError::Io)
            })
        }
    };
    if time {
        eprintln!("Elapsed: {:.3} ms", start.elapsed().as_secs_f64() * 1_000.0);
    }
    if result.is_err() {
        drop(output);
        if let Some(path) = output_path {
            let _ = std::fs::remove_file(path);
        }
    }
    result.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "SQL file exceeds the 4096-byte statement limit"
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
            "markdown",
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
