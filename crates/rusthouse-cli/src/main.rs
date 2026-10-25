use rusthouse_storage::inspect_part;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "Usage:\n  rusthouse inspect-part <path>\n  rusthouse --help\n\nInspect local ClickHouse MergeTree metadata (read-only).";

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
    if command != "inspect-part" {
        eprintln!("unknown command: {command:?}\n{USAGE}");
        return ExitCode::from(2);
    }
    let (Some(path), None) = (args.next(), args.next()) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    match inspect_and_print(path) {
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
