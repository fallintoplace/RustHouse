use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn batched_count_column_preserves_boolean_and_modulo_filter_semantics() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-count-column-batch-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let columns = vec![Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    let rows = (0..4097)
        .map(|value| vec![Some(value.to_string())])
        .collect::<Vec<_>>();
    write_wide_part(&root.join("all_0_0_0"), &columns, &rows).unwrap();
    for where_clause in [
        "id > 1024",
        "id = 4096",
        "id < 5 OR id >= 4090",
        "id % 7 = 3",
        "id BETWEEN 10 AND 30",
        "id IN (1, 7, 100, 4096)",
    ] {
        let sql = format!("SELECT count(id) FROM part WHERE {where_clause}");
        let oracle = format!(
            "SELECT count(number) FROM numbers(4097) WHERE {}",
            where_clause.replace("id", "number")
        );
        let run = |sql: &str, path| {
            let mut result = Vec::new();
            execute(&parse_select(sql).unwrap(), path, &mut result).unwrap();
            String::from_utf8(result).unwrap()
        };
        assert_eq!(
            run(&sql, Some(root.as_path())).replace("count(id)", "count(number)"),
            run(&oracle, None),
            "{sql}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}
