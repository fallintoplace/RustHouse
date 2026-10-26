mod csv;
mod csv_no_header;
mod json_array;
mod jsonl;

use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{
    CompressedBlockReader, NumericColumn, UInt64ColumnReader, inspect_part, read_checksums,
    read_numeric_granule, read_u64_granule, read_wide_marks, validate_wide_marks,
    verify_part_file_sizes, verify_wide_part,
};
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "Usage:\n  rusthouse inspect-part <path>\n  rusthouse verify-part <path-to-wide-part>\n  rusthouse verify-file-sizes <path-to-part>\n  rusthouse inspect-checksums <path-to-checksums.txt>\n  rusthouse inspect-marks <path-to-wide-marks>\n  rusthouse inspect-blocks <path-to-.bin>\n  rusthouse verify-blocks <path-to-.bin>\n  rusthouse validate-marks <path-to-wide-marks> <path-to-.bin>\n  rusthouse read-u64 <data.bin> <marks.mrk2> <mark-index>\n  rusthouse scan-u64 <path-to-UInt64-.bin>\n  rusthouse read-numeric <data.bin> <marks.mrk2> <mark-index> <type>\n  rusthouse sql <SQL> [--part <path-to-wide-part>] [--format tsv|tsv-no-header|csv|csv-no-header|jsonl|json] [--output <path>] [--show-ast]\n  rusthouse sql --stdin [--part <path-to-wide-part>] [--format tsv|tsv-no-header|csv|csv-no-header|jsonl|json] [--output <path>] [--show-ast]\n  rusthouse sql --file <path.sql> [--part <path-to-wide-part>] [--format tsv|tsv-no-header|csv|csv-no-header|jsonl|json] [--output <path>] [--show-ast]\n  rusthouse --help\n\nInspect local MergeTree metadata, marks, and compressed blocks.";

#[derive(Clone, Copy)]
enum SqlOutputFormat {
    Tsv,
    TsvNoHeader,
    Csv,
    CsvNoHeader,
    Jsonl,
    Json,
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
            _ => Err(
                "--format must be tsv, tsv-no-header, csv, csv-no-header, jsonl, or json".into(),
            ),
        }
    }
}

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(command) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    if command == "--help" || command == "-h" {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if command != "inspect-part"
        && command != "verify-part"
        && command != "verify-file-sizes"
        && command != "inspect-checksums"
        && command != "inspect-marks"
        && command != "inspect-blocks"
        && command != "verify-blocks"
        && command != "validate-marks"
        && command != "read-u64"
        && command != "scan-u64"
        && command != "read-numeric"
        && command != "sql"
    {
        eprintln!("unknown command: {command:?}\n{USAGE}");
        return ExitCode::from(2);
    }
    let result = if command == "sql" {
        run_sql_args(args)
    } else {
        let outcome = if command == "read-u64" {
            let (Some(data_path), Some(marks_path), Some(index), None) =
                (args.next(), args.next(), args.next(), args.next())
            else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            let Some(index) = index.to_str().and_then(|index| index.parse::<usize>().ok()) else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            read_u64_and_print(data_path, marks_path, index)
        } else if command == "read-numeric" {
            let (Some(data_path), Some(marks_path), Some(index), Some(data_type), None) = (
                args.next(),
                args.next(),
                args.next(),
                args.next(),
                args.next(),
            ) else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            let Some(index) = index.to_str().and_then(|value| value.parse::<usize>().ok()) else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            let Some(data_type) = data_type.to_str() else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            read_numeric_and_print(data_path, marks_path, index, data_type)
        } else if command == "validate-marks" {
            let (Some(marks_path), Some(data_path), None) = (args.next(), args.next(), args.next())
            else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            validate_marks_and_print(marks_path, data_path)
        } else {
            let (Some(path), None) = (args.next(), args.next()) else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            if command == "inspect-part" {
                inspect_and_print(path)
            } else if command == "verify-part" {
                verify_part_and_print(path)
            } else if command == "verify-file-sizes" {
                verify_file_sizes_and_print(path)
            } else if command == "inspect-checksums" {
                inspect_checksums_and_print(path)
            } else if command == "inspect-marks" {
                inspect_marks_and_print(path)
            } else if command == "inspect-blocks" {
                inspect_blocks_and_print(path)
            } else if command == "verify-blocks" {
                verify_blocks_and_print(path)
            } else {
                scan_u64_and_print(path)
            }
        };
        outcome.map_err(|error| error.to_string())
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn inspect_and_print(path: OsString) -> Result<(), rusthouse_storage::PartError> {
    let metadata = inspect_part(PathBuf::from(path))?;
    println!("Part: {}", metadata.path.display());
    println!("Rows: {}", metadata.rows);
    println!("Columns ({}):", metadata.columns.len());
    for column in metadata.columns {
        println!("  {:?}: {}", column.name, column.data_type);
    }
    Ok(())
}

fn verify_part_and_print(path: OsString) -> Result<(), rusthouse_storage::PartError> {
    let report = verify_wide_part(PathBuf::from(path))?;
    println!(
        "Verified {} streams, {} marks, {} blocks, {} decompressed bytes",
        report.streams, report.marks, report.blocks, report.decompressed_bytes
    );
    println!("Block checksums and mark offsets verified; checksums.txt file hashes not verified");
    Ok(())
}

fn verify_file_sizes_and_print(path: OsString) -> Result<(), rusthouse_storage::PartError> {
    let report = verify_part_file_sizes(PathBuf::from(path))?;
    println!(
        "Verified {} file sizes, {} total bytes (file hashes not verified)",
        report.files, report.total_bytes
    );
    Ok(())
}

fn inspect_checksums_and_print(path: OsString) -> Result<(), rusthouse_storage::PartError> {
    let checksums = read_checksums(PathBuf::from(path))?;
    println!("Format: {}", checksums.version);
    println!("Files: {}", checksums.files.len());
    for entry in checksums.files {
        match entry.uncompressed {
            Some(uncompressed) => println!(
                "  {:?}: {} bytes, uncompressed={} bytes",
                entry.name, entry.size, uncompressed.size
            ),
            None => println!("  {:?}: {} bytes", entry.name, entry.size),
        }
    }
    println!("Checksums recorded, not verified against disk files");
    Ok(())
}

fn inspect_marks_and_print(path: OsString) -> Result<(), rusthouse_storage::PartError> {
    let metadata = read_wide_marks(PathBuf::from(path))?;
    let extension = metadata
        .path
        .extension()
        .and_then(|extension| extension.to_str());
    let format = match (metadata.format, extension) {
        (rusthouse_storage::WideMarkFormat::Fixed, Some("cmrk")) => "fixed (.cmrk)",
        (rusthouse_storage::WideMarkFormat::Adaptive, Some("cmrk2")) => "adaptive (.cmrk2)",
        (rusthouse_storage::WideMarkFormat::Fixed, _) => "fixed (.mrk)",
        (rusthouse_storage::WideMarkFormat::Adaptive, _) => "adaptive (.mrk2)",
    };
    println!("Mark file: {}", metadata.path.display());
    println!("Format: {format}");
    println!("Marks: {}", metadata.marks.len());
    for (index, mark) in metadata.marks.into_iter().enumerate() {
        match mark.rows {
            Some(rows) => println!(
                "  {index}: compressed={}, decompressed={}, rows={rows}",
                mark.compressed_offset, mark.decompressed_offset
            ),
            None => println!(
                "  {index}: compressed={}, decompressed={}",
                mark.compressed_offset, mark.decompressed_offset
            ),
        }
    }
    Ok(())
}

fn inspect_blocks_and_print(path: OsString) -> Result<(), rusthouse_storage::PartError> {
    let path = PathBuf::from(path);
    let reader = CompressedBlockReader::open(&path)?;
    println!("Compressed file: {}", path.display());
    let mut count = 0;
    for block in reader {
        let block = block?;
        println!(
            "  {count}: offset={}, method=0x{:02x}, compressed={}, decompressed={}",
            block.offset, block.method, block.compressed_size, block.decompressed_size
        );
        count += 1;
    }
    println!("Blocks: {count} (headers only; checksums not verified)");
    Ok(())
}

fn verify_blocks_and_print(path: OsString) -> Result<(), rusthouse_storage::PartError> {
    let mut reader = CompressedBlockReader::open(PathBuf::from(path))?;
    let mut count = 0u64;
    let mut total_bytes = 0u64;
    while let Some(block) = reader.next_decoded() {
        total_bytes += block?.data.len() as u64;
        count += 1;
    }
    println!("Verified {count} blocks, {total_bytes} decompressed bytes");
    Ok(())
}

fn validate_marks_and_print(
    marks_path: OsString,
    data_path: OsString,
) -> Result<(), rusthouse_storage::PartError> {
    let marks = read_wide_marks(PathBuf::from(marks_path))?;
    let blocks = CompressedBlockReader::open(PathBuf::from(data_path))?;
    validate_wide_marks(&marks, blocks)?;
    println!(
        "Validated {} mark offsets (compressed block headers only; checksums not verified)",
        marks.marks.len()
    );
    Ok(())
}

fn read_u64_and_print(
    data_path: OsString,
    marks_path: OsString,
    index: usize,
) -> Result<(), rusthouse_storage::PartError> {
    let marks = read_wide_marks(PathBuf::from(marks_path))?;
    let mark = marks
        .marks
        .get(index)
        .ok_or(rusthouse_storage::PartError::InvalidGranuleIndex {
            index,
            marks: marks.marks.len(),
        })?;
    let mut reader = CompressedBlockReader::open(PathBuf::from(data_path))?;
    let values = read_u64_granule(&mut reader, mark)?;
    println!("Rows: {}", values.len());
    for value in values {
        println!("{value}");
    }
    Ok(())
}

fn scan_u64_and_print(path: OsString) -> Result<(), rusthouse_storage::PartError> {
    let reader = UInt64ColumnReader::open(PathBuf::from(path))?;
    let mut count = 0u64;
    let mut min = u64::MAX;
    let mut max = 0u64;
    for value in reader {
        let value = value?;
        count += 1;
        min = min.min(value);
        max = max.max(value);
    }
    println!("Rows: {count}");
    if count > 0 {
        println!("Min: {min}");
        println!("Max: {max}");
    }
    Ok(())
}

fn read_numeric_and_print(
    data_path: OsString,
    marks_path: OsString,
    index: usize,
    data_type: &str,
) -> Result<(), rusthouse_storage::PartError> {
    let marks = read_wide_marks(PathBuf::from(marks_path))?;
    let mark = marks
        .marks
        .get(index)
        .ok_or(rusthouse_storage::PartError::InvalidGranuleIndex {
            index,
            marks: marks.marks.len(),
        })?;
    let mut reader = CompressedBlockReader::open(PathBuf::from(data_path))?;
    let values = read_numeric_granule(&mut reader, mark, data_type)?;
    println!("Rows: {}", values.len());
    match values {
        NumericColumn::UInt8(values) => print_values(values),
        NumericColumn::UInt16(values) => print_values(values),
        NumericColumn::UInt32(values) => print_values(values),
        NumericColumn::UInt64(values) => print_values(values),
        NumericColumn::UInt128(values) => print_values(values),
        NumericColumn::Int8(values) => print_values(values),
        NumericColumn::Int16(values) => print_values(values),
        NumericColumn::Int32(values) => print_values(values),
        NumericColumn::Int64(values) => print_values(values),
        NumericColumn::Int128(values) => print_values(values),
        NumericColumn::Float32(values) => print_values(values),
        NumericColumn::Float64(values) => print_values(values),
    }
    Ok(())
}

fn print_values<T: std::fmt::Display>(values: Vec<T>) {
    for value in values {
        println!("{value}");
    }
}

fn run_sql_args(mut args: impl Iterator<Item = OsString>) -> Result<(), String> {
    use std::io::Read;

    let mut query = None;
    let mut file = None;
    let mut read_stdin = false;
    let mut part = None;
    let mut format = None;
    let mut output_path = None;
    let mut show_ast = false;
    while let Some(arg) = args.next() {
        if arg.to_str() == Some("--show-ast") {
            if show_ast {
                return Err("duplicate --show-ast option".into());
            }
            show_ast = true;
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
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read SQL from stdin: {error}"))?;
        if bytes.len() > 4096 {
            return Err("SQL from stdin exceeds the 4096-byte statement limit".into());
        }
        String::from_utf8(bytes).map_err(|_| "SQL from stdin must be valid UTF-8".to_string())?
    } else if let Some(file) = file {
        let mut input = std::fs::File::open(&file)
            .map_err(|error| format!("cannot open {}: {error}", file.display()))?;
        let mut bytes = Vec::new();
        input
            .by_ref()
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
        if bytes.len() > 4096 {
            return Err("SQL file exceeds the 4096-byte statement limit".into());
        }
        String::from_utf8(bytes).map_err(|_| "SQL file must be valid UTF-8".to_string())?
    } else {
        query.ok_or("missing SQL statement")?
    };
    let select = parse_select(&query).map_err(|error| error.to_string())?;
    if show_ast {
        if output_path.is_some() {
            return Err("--output cannot be combined with --show-ast".into());
        }
        println!("{select:#?}");
        return Ok(());
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
    if result.is_err() {
        drop(output);
        if let Some(path) = output_path {
            let _ = std::fs::remove_file(path);
        }
    }
    result.map_err(|error| error.to_string())
}
