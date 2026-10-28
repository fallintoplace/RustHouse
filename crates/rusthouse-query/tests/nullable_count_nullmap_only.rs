use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn nullable_count_reads_only_null_maps_across_parts_and_granules() {
    let root = std::env::temp_dir().join(format!("rusthouse-null-map-count-{}-{}", std::process::id(), line!()));
    fs::create_dir_all(&root).unwrap();
    let columns = [
        Column { name: "payload".into(), data_type: "Nullable(String)".into() },
        Column { name: "metric".into(), data_type: "Nullable(UInt64)".into() },
    ];
    for part in 0..2 {
        let rows = (0..2051usize).map(|index| {
            vec![
                (index % 3 != 0).then(|| "long-value-".repeat(50)),
                (index % 5 != 0).then(|| index.to_string()),
            ]
        }).collect::<Vec<_>>();
        write_wide_part(&root.join(format!("all_{part}_{part}_0")), &columns, &rows).unwrap();
    }
    for (sql, expected) in [
        ("SELECT count(payload) FROM part", "count(payload)\n2734\n"),
        ("SELECT count(metric) FROM part", "count(metric)\n3280\n"),
        ("SELECT count(payload) FROM part LIMIT 0", "count(payload)\n"),
        ("SELECT count(metric) FROM part LIMIT 1 OFFSET 1", "count(metric)\n"),
        ("SELECT count(payload) FROM part WHERE payload IS NULL", "count(payload)\n0\n"),
    ] {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{sql}");
    }
    fs::remove_dir_all(root).unwrap();
}
