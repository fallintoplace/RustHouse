use super::*;
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn filters_and_nullable_counts_match_sequential_across_skewed_parts() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-parallel-filtered-count-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let columns = [
        Column {
            name: "category".into(),
            data_type: "String".into(),
        },
        Column {
            name: "value".into(),
            data_type: "Nullable(UInt64)".into(),
        },
    ];
    for index in 0..20 {
        let rows = (0..(index + 1))
            .map(|row| {
                let is_keep = row % 2 == 0;
                vec![
                    Some(if is_keep { "keep" } else { "other" }.into()),
                    if is_keep && index % 3 == 0 {
                        None
                    } else {
                        Some(row.to_string())
                    },
                ]
            })
            .collect::<Vec<_>>();
        write_wide_part(
            &root.join(format!("all_{index}_{index}_0")),
            &columns,
            &rows,
        )
        .unwrap();
    }
    let names = vec!["value".to_owned(), "category".to_owned()];
    let query =
        crate::parse_select("SELECT count(*) FROM part WHERE category = 'keep'").unwrap();
    let indices = names
        .iter()
        .enumerate()
        .map(|(i, name)| (name.clone(), i))
        .collect::<HashMap<_, _>>();
    let types = columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect::<HashMap<_, _>>();
    let conditions = vec![compile(&query.predicates, &indices, &types).unwrap()];
    let parts = rusthouse_storage::query_parts(&root).unwrap();
    let all = parallel_filtered_count_parts(&parts, 3, &names, &conditions, None).unwrap();
    let nonnull =
        parallel_filtered_count_parts(&parts, 3, &names, &conditions, Some(0)).unwrap();
    let expected_all: u64 = (0..20).map(|i| (i + 2) / 2).sum();
    let expected_nonnull: u64 = (0..20).filter(|i| i % 3 != 0).map(|i| (i + 2) / 2).sum();
    assert_eq!(all, expected_all);
    assert_eq!(nonnull, expected_nonnull);
    assert!(nonnull < all);
    fs::remove_dir_all(root).unwrap();
}
