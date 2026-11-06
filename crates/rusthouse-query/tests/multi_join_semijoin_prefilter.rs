use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn large_build_table_uses_filtered_join_keys_above_small_keyset_limit() {
    const DIMENSION_ROWS: usize = 50_000;
    const MATCHING_ROWS: usize = 40_000;
    const BRIDGE_ROWS_PER_KEY: usize = 6;

    let root = std::env::temp_dir().join(format!(
        "rusthouse-multi-join-semijoin-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let dimension = root.join("dimension");
    let bridge = root.join("bridge");

    let fact_rows = (0..DIMENSION_ROWS)
        .map(|id| vec![Some(id.to_string()), Some((id + 1).to_string())])
        .collect::<Vec<_>>();
    write_wide_part(
        &fact,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "value".into(),
                data_type: "UInt64".into(),
            },
        ],
        &fact_rows,
    )
    .unwrap();

    let dimension_rows = (0..DIMENSION_ROWS)
        .map(|id| {
            vec![
                Some(id.to_string()),
                Some(if id < MATCHING_ROWS { "keep" } else { "skip" }.into()),
            ]
        })
        .collect::<Vec<_>>();
    write_wide_part(
        &dimension,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "flag".into(),
                data_type: "String".into(),
            },
        ],
        &dimension_rows,
    )
    .unwrap();

    let bridge_rows = (0..DIMENSION_ROWS)
        .flat_map(|id| {
            std::iter::repeat_with(move || vec![Some(id.to_string())]).take(BRIDGE_ROWS_PER_KEY)
        })
        .collect::<Vec<_>>();
    fs::create_dir_all(&bridge).unwrap();
    let bridge_column = [Column {
        name: "dimension_id".into(),
        data_type: "UInt64".into(),
    }];
    let split = 262_144;
    write_wide_part(
        &bridge.join("all_0_0_0"),
        &bridge_column,
        &bridge_rows[..split],
    )
    .unwrap();
    write_wide_part(
        &bridge.join("all_1_1_0"),
        &bridge_column,
        &bridge_rows[split..],
    )
    .unwrap();

    let query = parse_join(
        "SELECT MIN(f.value) AS min_value FROM fact f JOIN dimension d ON f.id = d.id JOIN bridge b ON d.id = b.dimension_id WHERE d.flag = 'keep'",
    )
    .unwrap()
    .unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &[fact, dimension, bridge], &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "min_value\n1\n");

    fs::remove_dir_all(root).unwrap();
}
