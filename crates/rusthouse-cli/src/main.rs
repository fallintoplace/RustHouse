use rusthouse_storage::{inspect_part, read_wide_marks};
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "Usage:\n  rusthouse inspect-part <path>\n  rusthouse inspect-marks <path-to-.mrk-or-.mrk2>\n  rusthouse --help\n\nRead local MergeTree metadata and uncompressed wide-part marks.";

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
    if command != "inspect-part" && command != "inspect-marks" {
        eprintln!("unknown command: {command:?}\n{USAGE}");
        return ExitCode::from(2);
    }
    let (Some(path), None) = (args.next(), args.next()) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    let result = if command == "inspect-part" {
        inspect_and_print(path)
    } else {
        inspect_marks_and_print(path)
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

fn inspect_marks_and_print(path: OsString) -> Result<(), rusthouse_storage::PartError> {
    let metadata = read_wide_marks(PathBuf::from(path))?;
    let format = match metadata.format {
        rusthouse_storage::WideMarkFormat::Fixed => "fixed (.mrk)",
        rusthouse_storage::WideMarkFormat::Adaptive => "adaptive (.mrk2)",
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
