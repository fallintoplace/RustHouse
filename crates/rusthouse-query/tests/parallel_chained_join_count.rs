use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::PathBuf;

#[test]
fn parallel_chained_count_merges_skewed_fact_partials_exactly() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-parallel-chained-count-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let fact = root.join("fact");
    fs::create_dir_all(&fact).unwrap();
    let schema = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    for part in 0..20 {
        let rows = (0..(part + 1))
            .map(|row| vec![Some((row % 2 + 1).to_string())])
            .collect::<Vec<_>>();
        write_wide_part(
            &fact.join(format!("all_{part}_{part}_0")),
            &schema,
            &rows,
        )
        .unwrap();
    }
    let mut paths = vec![fact];
    for name in ["dim", "region"] {
        let path = root.join(name);
        write_wide_part(
            &path,
            &schema,
            &[vec![Some("1".into())], vec![Some("2".into())]],
        )
        .unwrap();
        paths.push(path);
    }
    let query = parse_join(
        "SELECT COUNT(*) FROM fact f JOIN dim d ON f.id = d.id JOIN region r ON d.id = r.id",
    )
    .unwrap()
    .unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &paths, &mut output).unwrap();
    let expected: usize = (1..=20).sum();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        format!("count\n{expected}\n")
    );
    fs::remove_dir_all(root).unwrap();
}
