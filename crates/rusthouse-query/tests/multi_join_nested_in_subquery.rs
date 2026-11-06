use rusthouse_query::{bind_join_in_subquery, execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn nested_date32_in_filters_a_grouped_join() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-nested-in-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let items = root.join("items");
    let dates = root.join("date_dim");

    write_wide_part(
        &fact,
        &[
            Column {
                name: "item_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "sold_date".into(),
                data_type: "Date32".into(),
            },
            Column {
                name: "amount".into(),
                data_type: "Int64".into(),
            },
        ],
        &[
            vec![
                Some("1".into()),
                Some("2020-01-01".into()),
                Some("10".into()),
            ],
            vec![
                Some("1".into()),
                Some("2020-01-03".into()),
                Some("100".into()),
            ],
            vec![
                Some("2".into()),
                Some("2020-01-02".into()),
                Some("7".into()),
            ],
            vec![
                Some("2".into()),
                Some("2020-01-04".into()),
                Some("50".into()),
            ],
        ],
    )
    .unwrap();
    write_wide_part(
        &items,
        &[Column {
            name: "item_id".into(),
            data_type: "UInt64".into(),
        }],
        &[vec![Some("1".into())], vec![Some("2".into())]],
    )
    .unwrap();
    write_wide_part(
        &dates,
        &[
            Column {
                name: "date_key".into(),
                data_type: "Date32".into(),
            },
            Column {
                name: "week_id".into(),
                data_type: "UInt32".into(),
            },
            Column {
                name: "day_code".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![
                Some("2020-01-01".into()),
                Some("100".into()),
                Some("target".into()),
            ],
            vec![
                Some("2020-01-02".into()),
                Some("100".into()),
                Some("next".into()),
            ],
            vec![
                Some("2020-01-03".into()),
                Some("101".into()),
                Some("other".into()),
            ],
            vec![
                Some("2020-01-04".into()),
                Some("102".into()),
                Some("other".into()),
            ],
        ],
    )
    .unwrap();

    let mut query = parse_join(
        "SELECT i.item_id, SUM(f.amount) AS total FROM fact f JOIN items i ON f.item_id = i.item_id JOIN date_dim d ON f.sold_date = d.date_key WHERE sold_date IN (SELECT date_key FROM date_dim WHERE week_id IN (SELECT week_id FROM date_dim WHERE day_code IN ('target'))) GROUP BY i.item_id ORDER BY i.item_id",
    )
    .unwrap()
    .unwrap();
    assert!(query.in_subquery.is_some());
    bind_join_in_subquery(
        &mut query,
        &dates,
        &[fact.clone(), items.clone(), dates.clone()],
    )
    .unwrap();

    let mut output = Vec::new();
    execute_multi_join(
        &query,
        &[fact.clone(), items.clone(), dates.clone()],
        &mut output,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "item_id\ttotal\n1\t10\n2\t7\n"
    );

    let mut scalar_query = parse_join(
        "SELECT i.item_id, SUM(f.amount) AS total FROM fact f JOIN items i ON f.item_id = i.item_id JOIN date_dim d ON f.sold_date = d.date_key WHERE sold_date IN (SELECT date_key FROM date_dim WHERE week_id = (SELECT week_id FROM date_dim WHERE day_code = 'target')) GROUP BY i.item_id ORDER BY i.item_id",
    )
    .unwrap()
    .unwrap();
    assert!(
        scalar_query
            .in_subquery
            .as_ref()
            .unwrap()
            .inner
            .in_subquery
            .as_ref()
            .unwrap()
            .scalar
    );
    bind_join_in_subquery(
        &mut scalar_query,
        &dates,
        &[fact.clone(), items.clone(), dates.clone()],
    )
    .unwrap();
    let mut scalar_output = Vec::new();
    execute_multi_join(
        &scalar_query,
        &[fact.clone(), items.clone(), dates.clone()],
        &mut scalar_output,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(scalar_output).unwrap(),
        "item_id\ttotal\n1\t10\n2\t7\n"
    );

    let mut multirow_scalar = parse_join(
        "SELECT i.item_id, SUM(f.amount) AS total FROM fact f JOIN items i ON f.item_id = i.item_id JOIN date_dim d ON f.sold_date = d.date_key WHERE sold_date IN (SELECT date_key FROM date_dim WHERE week_id = (SELECT week_id FROM date_dim WHERE day_code IN ('target', 'next'))) GROUP BY i.item_id ORDER BY i.item_id",
    )
    .unwrap()
    .unwrap();
    let error = bind_join_in_subquery(
        &mut multirow_scalar,
        &dates,
        &[fact.clone(), items.clone(), dates.clone()],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("scalar JOIN IN subquery returned more than one row"),
        "{error}"
    );

    let mut empty_scalar = parse_join(
        "SELECT i.item_id, SUM(f.amount) AS total FROM fact f JOIN items i ON f.item_id = i.item_id JOIN date_dim d ON f.sold_date = d.date_key WHERE sold_date IN (SELECT date_key FROM date_dim WHERE week_id = (SELECT week_id FROM date_dim WHERE day_code = 'missing')) GROUP BY i.item_id ORDER BY i.item_id",
    )
    .unwrap()
    .unwrap();
    bind_join_in_subquery(
        &mut empty_scalar,
        &dates,
        &[fact.clone(), items.clone(), dates.clone()],
    )
    .unwrap();
    let mut empty_output = Vec::new();
    execute_multi_join(
        &empty_scalar,
        &[fact.clone(), items.clone(), dates.clone()],
        &mut empty_output,
    )
    .unwrap();
    assert_eq!(String::from_utf8(empty_output).unwrap(), "item_id\ttotal\n");

    let mut mismatched_types = parse_join(
        "SELECT i.item_id, SUM(f.amount) AS total FROM fact f JOIN items i ON f.item_id = i.item_id JOIN date_dim d ON f.sold_date = d.date_key WHERE sold_date IN (SELECT date_key FROM date_dim WHERE week_id = (SELECT day_code FROM date_dim WHERE day_code = 'target')) GROUP BY i.item_id ORDER BY i.item_id",
    )
    .unwrap()
    .unwrap();
    let error = bind_join_in_subquery(
        &mut mismatched_types,
        &dates,
        &[fact.clone(), items.clone(), dates.clone()],
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("nested JOIN IN requires matching scalar key types"),
        "{error}"
    );

    fs::remove_dir_all(root).unwrap();
}
