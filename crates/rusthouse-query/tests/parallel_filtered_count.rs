use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_ID: AtomicUsize = AtomicUsize::new(0);

struct Snapshot(PathBuf);

impl Snapshot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-parallel-count-{}-{}",
            std::process::id(),
            TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let schema = [
            Column { name: "id".into(), data_type: "UInt64".into() },
            Column { name: "kind".into(), data_type: "String".into() },
        ];
        for part_number in 0..40usize {
            let rows = (0..64usize)
                .map(|row| {
                    let id = part_number * 64 + row;
                    vec![Some(id.to_string()), Some(if id % 4 == 0 { "hot" } else { "cold" }.into())]
                })
                .collect::<Vec<_>>();
            write_wide_part(
                &path.join(format!("all_{part_number}_{part_number}_0")),
                &schema,
                &rows,
            )
            .unwrap();
        }
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn filtered_count_is_correct_across_many_immutable_parts() {
    let source = Snapshot::new();
    for (sql, expected) in [
        ("SELECT count(*) FROM part WHERE id >= 1000", "count\n1560\n"),
        ("SELECT count(*) FROM part WHERE kind = 'hot'", "count\n640\n"),
        ("SELECT count(*) FROM part WHERE id < 100 OR kind = 'hot'", "count\n715\n"),
        ("SELECT count(*) AS hits FROM part WHERE id >= 2500", "hits\n60\n"),
        ("SELECT count(*) FROM part WHERE id < 0", "count\n0\n"),
    ] {
        let query = parse_select(sql).unwrap();
        let mut result = Vec::new();
        execute(&query, Some(source.path()), &mut result).unwrap();
        assert_eq!(String::from_utf8(result).unwrap(), expected, "{sql}");
    }
}

#[test]
fn limits_and_metadata_only_count_retain_existing_semantics() {
    let source = Snapshot::new();
    for (sql, expected) in [
        ("SELECT count(*) FROM part", "count\n2560\n"),
        ("SELECT count(*) FROM part WHERE id >= 0 LIMIT 0", "count\n"),
        ("SELECT count(*) FROM part WHERE id >= 0 OFFSET 1", "count\n"),
    ] {
        let mut result = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(source.path()), &mut result).unwrap();
        assert_eq!(String::from_utf8(result).unwrap(), expected, "{sql}");
    }
}
