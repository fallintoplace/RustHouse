use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{
    CompressedBlockReader, NumericColumn, UInt64ColumnReader, inspect_part, read_checksums,
    read_numeric_granule, read_u64_granule, read_wide_marks, validate_wide_marks,
};
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "Usage:\n  rusthouse inspect-part <path>\n  rusthouse inspect-checksums <path-to-checksums.txt>\n  rusthouse inspect-marks <path-to-wide-marks>\n  rusthouse inspect-blocks <path-to-.bin>\n  rusthouse verify-blocks <path-to-.bin>\n  rusthouse validate-marks <path-to-wide-marks> <path-to-.bin>\n  rusthouse read-u64 <data.bin> <marks.mrk2> <mark-index>\n  rusthouse scan-u64 <path-to-UInt64-.bin>\n  rusthouse read-numeric <data.bin> <marks.mrk2> <mark-index> <type>\n  rusthouse sql <SQL> [--part <path-to-wide-part>]\n  rusthouse --help\n\nInspect local MergeTree metadata, marks, and compressed blocks.";

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
        NumericColumn::Int8(values) => print_values(values),
        NumericColumn::Int16(values) => print_values(values),
        NumericColumn::Int32(values) => print_values(values),
        NumericColumn::Int64(values) => print_values(values),
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
    let mut query = None;
    let mut part = None;
    while let Some(arg) = args.next() {
        if arg.to_str() == Some("--part") {
            if part.is_some() {
                return Err("duplicate --part option".into());
            }
            part = Some(PathBuf::from(
                args.next().ok_or("missing --part directory")?,
            ));
        } else if query.is_none() {
            query = Some(
                arg.into_string()
                    .map_err(|_| "SQL statement must be valid UTF-8")?,
            );
        } else {
            return Err("unexpected SQL argument".into());
        }
    }
    let query = query.ok_or("missing SQL statement")?;
    let select = parse_select(&query).map_err(|error| error.to_string())?;
    execute(&select, part.as_deref(), &mut std::io::stdout()).map_err(|error| error.to_string())
}
