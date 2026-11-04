use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn chained_join_tracks_multiple_nullable_avg_denominators() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-multi-avg-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let dimension = root.join("dimension");
    let dates = root.join("dates");
    let key = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    write_wide_part(
        &fact,
        &key,
        &[vec![Some("1".into())], vec![Some("2".into())]],
    )
    .unwrap();
    write_wide_part(
        &dates,
        &key,
        &[vec![Some("1".into())], vec![Some("2".into())]],
    )
    .unwrap();
    write_wide_part(
        &dimension,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "quantity".into(),
                data_type: "Nullable(UInt64)".into(),
            },
            Column {
                name: "price".into(),
                data_type: "Nullable(UInt64)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("1".into()), Some("10".into())],
            vec![Some("1".into()), Some("3".into()), None],
            vec![Some("2".into()), None, Some("6".into())],
        ],
    )
    .unwrap();
    let paths = [fact, dimension, dates];
    let sql = "SELECT f.id, AVG(d.quantity) AS avg_quantity, AVG(d.price) AS avg_price FROM fact f JOIN dimension d ON f.id=d.id JOIN dates t ON d.id=t.id GROUP BY f.id ORDER BY f.id";
    let query = parse_join(sql).unwrap().unwrap();
    assert_eq!(query.grouped_avg_metrics.len(), 2);
    let mut out = Vec::new();
    execute_multi_join(&query, &paths, &mut out).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "f.id\tavg_quantity\tavg_price\n1\t2\t10\n2\t\\N\t6\n"
    );

    let query = parse_join(&format!("{sql} LIMIT 1 OFFSET 1"))
        .unwrap()
        .unwrap();
    let mut out = Vec::new();
    execute_multi_join(&query, &paths, &mut out).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "f.id\tavg_quantity\tavg_price\n2\t\\N\t6\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_grouped_avg_combinations_fail_early() {
    for sql in [
        "SELECT a.id, AVG(b.x) AS x FROM a JOIN b ON a.id=b.id GROUP BY a.id",
        "SELECT a.id, AVG(b.x) AS avg_x, AVG(c.y) AS avg_x FROM a JOIN b ON a.id=b.id JOIN c ON b.id=c.id GROUP BY a.id",
        "SELECT a.id, AVG(b.x) FROM a JOIN b ON a.id=b.id LEFT JOIN c ON b.id=c.id GROUP BY a.id",
    ] {
        assert!(parse_join(sql).is_err(), "{sql}");
    }
}
