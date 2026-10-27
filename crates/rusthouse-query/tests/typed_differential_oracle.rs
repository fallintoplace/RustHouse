//! Deterministic differential checks for persisted typed wide parts.
//! Expected results come from an independent Rust row model.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const ROWS: u64 = 4161;
static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-typed-oracle-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let columns = [
        Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "dept".into(),
            data_type: "String".into(),
        },
        Column {
            name: "amount".into(),
            data_type: "Nullable(Int64)".into(),
        },
        Column {
            name: "note".into(),
            data_type: "Nullable(String)".into(),
        },
    ];
    let names = ["red", "blue", "green"];
    for (index, start) in (0..ROWS).step_by(4096).enumerate() {
        let end = (start + 4096).min(ROWS);
        let records = (start..end)
            .map(|id| {
                vec![
                    Some(id.to_string()),
                    Some(names[(id % 3) as usize].into()),
                    (id % 7 != 0).then(|| ((id % 51) as i64 - 25).to_string()),
                    (id % 13 != 0).then(|| format!("note-{id}")),
                ]
            })
            .collect::<Vec<_>>();
        write_wide_part(
            &root.join(format!("all_{index}_{index}_0")),
            &columns,
            &records,
        )
        .unwrap();
    }
    root
}

fn run(sql: &str, source: &Path) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, Some(source), &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn seeded_typed_predicates_and_pagination_match_reference_rows() {
    // Includes a 1024-row granule boundary and a 4096-row part boundary.
    let root = fixture();
    let names = ["red", "blue", "green"];
    let mut seed = 0xd1ff_2026_1027_f00du64;
    for case in 0..60 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let department = seed % 3;
        let cutoff = (seed >> 12) % (ROWS + 50);
        let limit = ((seed >> 29) % 25 + 1) as usize;
        let offset = ((seed >> 43) % 17) as usize;
        let sql = format!(
            "SELECT id FROM part WHERE dept = '{}' AND id >= {cutoff} ORDER BY id DESC LIMIT {limit} OFFSET {offset}",
            names[department as usize]
        );
        let mut expected = String::from("id\n");
        for id in (0..ROWS)
            .rev()
            .filter(|id| id % 3 == department && *id >= cutoff)
            .skip(offset)
            .take(limit)
        {
            expected.push_str(&format!("{id}\n"));
        }
        assert_eq!(run(&sql, &root), expected, "case {case}: {sql}");

        if case % 10 == 0 {
            // Nullable aggregate semantics are checked independently of SQL.
            let sql = format!(
                "SELECT count(amount) FROM part WHERE dept = '{}' AND id >= {cutoff}",
                names[department as usize]
            );
            let expected = (0..ROWS)
                .filter(|id| id % 3 == department && *id >= cutoff && id % 7 != 0)
                .count();
            assert_eq!(
                run(&sql, &root),
                format!("count(amount)\n{expected}\n"),
                "case {case}: {sql}"
            );
        }
    }

    let sql = "SELECT id FROM part WHERE note IS NULL ORDER BY id DESC LIMIT 5";
    let expected = (0..ROWS).rev().filter(|id| id % 13 == 0).take(5).fold(
        String::from("id\n"),
        |mut output, id| {
            output.push_str(&format!("{id}\n"));
            output
        },
    );
    assert_eq!(run(sql, &root), expected);
    fs::remove_dir_all(root).unwrap();
}
