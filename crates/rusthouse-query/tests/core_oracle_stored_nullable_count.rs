use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::Path;

fn run(root: &Path, sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(root), &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn nullable_count_differs_from_physical_rows_across_parts() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-core-nullable-count-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let columns = [
        Column {
            name: "grp".into(),
            data_type: "String".into(),
        },
        Column {
            name: "value".into(),
            data_type: "Nullable(Int64)".into(),
        },
    ];
    let parts = [
        vec![
            vec![Some("alpha".into()), Some("4".into())],
            vec![Some("beta".into()), None],
            vec![Some("alpha".into()), None],
        ],
        vec![
            vec![Some("beta".into()), Some("-2".into())],
            vec![Some("alpha".into()), Some("6".into())],
            vec![Some("beta".into()), None],
        ],
    ];
    for (index, rows) in parts.iter().enumerate() {
        write_wide_part(&root.join(format!("all_{index}_{index}_0")), &columns, rows)
            .unwrap();
    }
    assert_eq!(
        run(&root, "SELECT grp, count(*) FROM part GROUP BY grp ORDER BY grp"),
        "grp\tcount\nalpha\t3\nbeta\t3\n"
    );
    assert_eq!(
        run(&root, "SELECT grp, count(value) FROM part GROUP BY grp ORDER BY grp"),
        "grp\tcount(value)\nalpha\t2\nbeta\t1\n"
    );
    fs::remove_dir_all(root).unwrap();
}
