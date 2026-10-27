//! Summarize a frozen set of compatible active wide MergeTree parts.
use crate::part_json::{json_string, print_columns_json};
use rusthouse_storage::{PartError, PartMetadata, inspect_query_source, query_parts};
use std::path::Path;

/// Accept a single part or a frozen active-part directory and reject invalid layouts.
pub(crate) fn inspect_and_print(path: &Path, json: bool) -> Result<(), PartError> {
    // inspect_query_source validates that all active parts have identical schemas and
    // checked row counts; query_parts also rejects overlaps and malformed part names.
    let metadata = inspect_query_source(path)?;
    let part_count = query_parts(path)?.len();

    if json {
        print_snapshot_json(path, part_count, &metadata);
    } else {
        print_snapshot_text(part_count, &metadata);
    }
    Ok(())
}


fn print_snapshot_json(path: &Path, part_count: usize, metadata: &PartMetadata) {
    print!(
        "{{\"path\":{},\"parts\":{},\"rows\":{},\"columns\":[",
        json_string(&path.to_string_lossy()),
        part_count,
        metadata.rows
    );
    print_columns_json(&metadata.columns);
    println!("]}}");
}

fn print_snapshot_text(part_count: usize, metadata: &PartMetadata) {
    println!("Parts: {part_count}");
    println!("Rows: {}", metadata.rows);
    println!("Columns: {}", metadata.columns.len());
    for column in &metadata.columns {
        println!("  \"{}\": {}", column.name, column.data_type);
    }
}
