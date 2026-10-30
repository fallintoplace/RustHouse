mod catalog;
mod copy_from;
mod csv;
mod csv_no_header;
mod database;
mod doctor;
mod html;
mod insert_select;
mod join;
mod json_array;
mod json_compact_each_row;
mod json_seq;
mod jsonl;
mod markdown;
mod part_json;
mod snapshot;
mod sql;
mod sql_values;
mod tsv_escape;
mod vertical;

use rusthouse_storage::{
    CompressedBlockReader, NumericColumn, UInt64ColumnReader, inspect_part, read_checksums,
    read_fixed_primary_index, read_numeric_granule, read_u64_granule, read_wide_marks,
    validate_wide_marks, verify_part_file_hashes, verify_part_file_hashes_full,
    verify_part_file_sizes, verify_wide_part,
};
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "Usage:\n  rusthouse inspect-part <path> [--json]\n  rusthouse inspect-snapshot <frozen-part-or-active-parts-directory> [--json]\n  rusthouse verify-part <path-to-wide-part>\n  rusthouse verify-file-sizes <path-to-part> [--hashes|--full-hashes]\n  rusthouse inspect-checksums <path-to-checksums.txt> [--summary]\n  rusthouse inspect-marks <path-to-wide-marks>\n  rusthouse inspect-primary-index <primary.idx|primary.cidx> <field-width> [field-width ...]\n  rusthouse inspect-blocks <path-to-.bin>\n  rusthouse verify-blocks <path-to-.bin>\n  rusthouse validate-marks <path-to-wide-marks> <path-to-.bin>\n  rusthouse read-u64 <data.bin> <marks.mrk2> <mark-index>\n  rusthouse scan-u64 <path-to-UInt64-.bin>\n  rusthouse read-numeric <data.bin> <marks.mrk2> <mark-index> <type>\n  rusthouse sql <SQL> [--part <path-to-wide-part>] [--format tsv|tsv-no-header|csv|csv-no-header|jsonl|json|json-pretty|json-compact-each-row|json-seq|markdown|html|sql-values|vertical] [--output <path>] [--max-rows <n>] [--show-ast|--explain] [--time]\n  rusthouse sql --stdin [--part <path-to-wide-part>] [--format tsv|tsv-no-header|csv|csv-no-header|jsonl|json|json-pretty|json-compact-each-row|json-seq|markdown|html|sql-values|vertical] [--output <path>] [--max-rows <n>] [--show-ast|--explain] [--time]\n  rusthouse sql --file <path.sql> [--part <path-to-wide-part>] [--format tsv|tsv-no-header|csv|csv-no-header|jsonl|json|json-pretty|json-compact-each-row|json-seq|markdown|html|sql-values|vertical] [--output <path>] [--max-rows <n>] [--show-ast|--explain] [--time]\n  rusthouse sql <DDL> --catalog <directory> [--show-ast]\n  rusthouse sql <DDL|INSERT> --database <directory> [--show-ast]\n  rusthouse sql --script <path.sql> --database <directory>\n  rusthouse sql 'EXPLAIN SELECT ...' [--part <path>|--database <directory>]\n  rusthouse doctor --database <directory> [--full-hashes] [--all-generations]\n  rusthouse --version\n  rusthouse --help\n\nInspect local MergeTree metadata, marks, and compressed blocks.";

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(command) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    if command == "--version" || command == "-V" {
        if args.next().is_some() {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
        println!("rusthouse {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if command == "--help" || command == "-h" {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if command != "inspect-part"
        && command != "inspect-snapshot"
        && command != "verify-part"
        && command != "verify-file-sizes"
        && command != "inspect-checksums"
        && command != "inspect-marks"
        && command != "inspect-primary-index"
        && command != "inspect-blocks"
        && command != "verify-blocks"
        && command != "validate-marks"
        && command != "read-u64"
        && command != "scan-u64"
        && command != "read-numeric"
        && command != "sql"
        && command != "doctor"
    {
        eprintln!("unknown command: {command:?}\n{USAGE}");
        return ExitCode::from(2);
    }
    let result = if command == "sql" {
        sql::run_sql_args(args)
    } else if command == "doctor" {
        doctor::run(args)
    } else if command == "inspect-primary-index" {
        let Some(path) = args.next() else {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        };
        inspect_primary_index_and_print(path, args)
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
        } else if command == "inspect-part" {
            let (Some(path), format, None) = (args.next(), args.next(), args.next()) else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            if format.is_none() {
                inspect_and_print(path)
            } else if format.as_ref().and_then(|arg| arg.to_str()) == Some("--json") {
                part_json::inspect_and_print_json(PathBuf::from(path))
            } else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        } else if command == "inspect-snapshot" {
            let (Some(path), option, None) = (args.next(), args.next(), args.next()) else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            let json = match option.as_deref() {
                None => false,
                Some(value) if value == "--json" => true,
                _ => {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            };
            snapshot::inspect_and_print(&PathBuf::from(path), json)
        } else if command == "inspect-checksums" {
            let (Some(path), option, None) = (args.next(), args.next(), args.next()) else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            if option.is_none() {
                inspect_checksums_and_print(path)
            } else if option.as_ref().and_then(|arg| arg.to_str()) == Some("--summary") {
                inspect_checksum_sizes_and_print(path)
            } else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        } else if command == "verify-file-sizes" {
            let (Some(path), option, None) = (args.next(), args.next(), args.next()) else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            let mode = match option.as_deref() {
                None => 0,
                Some(value) if value == "--hashes" => 1,
                Some(value) if value == "--full-hashes" => 2,
                _ => {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            };
            verify_file_sizes_and_print(path, mode)
        } else {
            let (Some(path), None) = (args.next(), args.next()) else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            if command == "verify-part" {
                verify_part_and_print(path)
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

const MAX_PREVIEW_MARKS: usize = 16;
const MAX_PREVIEW_FIELDS: usize = 8;
const MAX_PREVIEW_FIELD_BYTES: usize = 16;

fn inspect_primary_index_and_print(
    path: OsString,
    widths: impl Iterator<Item = OsString>,
) -> Result<(), String> {
    let widths = widths
        .map(|value| {
            let width = value
                .to_str()
                .ok_or("primary-index field widths must be positive integers")?
                .parse::<usize>()
                .map_err(|_| "primary-index field widths must be positive integers")?;
            if width == 0 {
                return Err("primary-index field widths must be positive integers");
            }
            Ok(width)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if widths.is_empty() {
        return Err("provide at least one primary-index field width".into());
    }
    let index = read_fixed_primary_index(PathBuf::from(path), &widths)
        .map_err(|error| error.to_string())?;
    println!("Marks: {}", index.len());
    println!("Fields: {}", widths.len());
    // Keep inspection output bounded even for large or adversarial indices.
    for mark in 0..index.len().min(MAX_PREVIEW_MARKS) {
        print!("Mark {mark}:");
        for field in 0..widths.len().min(MAX_PREVIEW_FIELDS) {
            let bytes = index
                .field(mark, field)
                .expect("validated fixed-width tuple");
            print!(" ");
            for byte in bytes.iter().take(MAX_PREVIEW_FIELD_BYTES) {
                print!("{byte:02x}");
            }
            if bytes.len() > MAX_PREVIEW_FIELD_BYTES {
                print!("...");
            }
        }
        if widths.len() > MAX_PREVIEW_FIELDS {
            print!(" ... ({} more fields)", widths.len() - MAX_PREVIEW_FIELDS);
        }
        println!();
    }
    if index.len() > MAX_PREVIEW_MARKS {
        println!("... {} more marks", index.len() - MAX_PREVIEW_MARKS);
    }
    Ok(())
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

fn verify_file_sizes_and_print(
    path: OsString,
    mode: u8,
) -> Result<(), rusthouse_storage::PartError> {
    let path = PathBuf::from(path);
    let (report, status) = match mode {
        0 => (verify_part_file_sizes(&path)?, "file hashes not verified"),
        1 => (
            verify_part_file_hashes(&path)?,
            "stored hashes verified; uncompressed hashes not verified",
        ),
        2 => (
            verify_part_file_hashes_full(&path)?,
            "stored and uncompressed hashes verified",
        ),
        _ => unreachable!(),
    };
    println!(
        "Verified {} file sizes, {} total bytes ({status})",
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

fn inspect_checksum_sizes_and_print(path: OsString) -> Result<(), rusthouse_storage::PartError> {
    let manifest = read_checksums(PathBuf::from(path))?;
    let sizes = manifest.size_summary()?;
    println!("Files: {}", sizes.files);
    println!("Stored bytes: {}", sizes.stored_bytes);
    println!("Compressed files: {}", sizes.compressed_files);
    println!("Compressed stored bytes: {}", sizes.compressed_stored_bytes);
    println!(
        "Uncompressed bytes (compressed files only): {}",
        sizes.compressed_original_bytes
    );
    println!("Manifest sizes only; local files and hashes not verified");
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
