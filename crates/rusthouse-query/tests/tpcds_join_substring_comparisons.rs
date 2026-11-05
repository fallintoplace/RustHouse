use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn joined_zip_prefix_filters_compare_substrings_without_materializing_pairs() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-substring-joins-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let address = root.join("address");
    let store = root.join("store");
    let key = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    write_wide_part(
        &fact,
        &key,
        &[
            vec![Some("1".into())],
            vec![Some("2".into())],
            vec![Some("3".into())],
            vec![Some("4".into())],
        ],
    )
    .unwrap();
    let zip_schema = [
        Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "zip".into(),
            data_type: "Nullable(String)".into(),
        },
    ];
    write_wide_part(
        &address,
        &zip_schema,
        &[
            vec![Some("1".into()), Some("12345AA".into())],
            vec![Some("2".into()), Some("99999XX".into())],
            vec![Some("3".into()), None],
            vec![Some("4".into()), Some("é1234".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &store,
        &zip_schema,
        &[
            vec![Some("1".into()), Some("12345ZZ".into())],
            vec![Some("2".into()), Some("88888YY".into())],
            vec![Some("3".into()), Some("55555".into())],
            vec![Some("4".into()), Some("é9876".into())],
        ],
    )
    .unwrap();
    let paths = [fact, address, store];
    let base =
        "SELECT f.id FROM fact f JOIN address a ON f.id=a.id JOIN store s ON a.id=s.id WHERE ";
    let run = |suffix: &str| {
        let query = parse_join(&format!("{base}{suffix}"))
            .unwrap_or_else(|error| panic!("{suffix}: {error}"))
            .unwrap();
        let mut output = Vec::new();
        execute_multi_join(&query, &paths, &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("id"));
        let mut rows = lines.map(str::to_owned).collect::<Vec<_>>();
        rows.sort_unstable();
        rows
    };
    assert_eq!(
        run("SUBSTR(a.zip, 1, 5) = SUBSTRING(s.zip FROM 1 FOR 5)"),
        vec!["1".to_owned()]
    );
    assert_eq!(
        run("SUBSTRING(a.zip, 1, 5) <> SUBSTR(s.zip, 1, 5)"),
        vec!["2".to_owned(), "4".to_owned()]
    );
    assert_eq!(
        run("SUBSTRING(a.zip, 1, 5) IN ('12345') OR a.zip NOT IN ('88888')"),
        vec!["1".to_owned(), "2".to_owned(), "4".to_owned()]
    );
    assert_eq!(
        run("SUBSTR(a.zip, 1, 5) NOT IN ('12345') OR a.id = 99"),
        vec!["2".to_owned(), "4".to_owned()]
    );
    assert!(run("SUBSTR(a.zip, 1, 5) IN ('no-match', NULL) OR a.id = 99").is_empty());
    assert!(run("SUBSTR(a.zip, 1, 5) NOT IN ('no-match', NULL) OR a.id = 99").is_empty());
    assert_eq!(run("SUBSTR(a.zip, 1, 2) IN ('é')"), vec!["4".to_owned()]);
    assert!(run("SUBSTR(a.zip, 1, 1) IN ('é')").is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn substring_filters_fail_closed_on_unsupported_shapes() {
    for sql in [
        "SELECT f.id FROM fact f JOIN address a ON f.id=a.id WHERE SUBSTR(a.zip,1,5) = SUBSTR(f.id,1,5)",
        "SELECT f.id FROM fact f JOIN address a ON f.id=a.id JOIN store s ON a.id=s.id WHERE SUBSTR(a.zip,0,5) = SUBSTR(s.zip,1,5)",
        "SELECT f.id FROM fact f JOIN address a ON f.id=a.id JOIN store s ON a.id=s.id WHERE SUBSTR(a.zip,1,65) = SUBSTR(s.zip,1,5)",
    ] {
        assert!(parse_join(sql).is_err(), "{sql}");
    }
}

#[test]
fn substring_row_filter_applies_before_grouped_sum() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-substring-group-sum-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let address = root.join("address");
    let store = root.join("store");
    let key = Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    };
    write_wide_part(
        &fact,
        &[
            key.clone(),
            Column {
                name: "amount".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("5".into())],
            vec![Some("2".into()), Some("7".into())],
            vec![Some("3".into()), Some("11".into())],
        ],
    )
    .unwrap();
    let zip_schema = [
        key,
        Column {
            name: "zip".into(),
            data_type: "Nullable(String)".into(),
        },
    ];
    write_wide_part(
        &address,
        &zip_schema,
        &[
            vec![Some("1".into()), Some("12345AA".into())],
            vec![Some("2".into()), Some("99999XX".into())],
            vec![Some("3".into()), None],
        ],
    )
    .unwrap();
    write_wide_part(
        &store,
        &zip_schema,
        &[
            vec![Some("1".into()), Some("12345ZZ".into())],
            vec![Some("2".into()), Some("88888YY".into())],
            vec![Some("3".into()), Some("55555".into())],
        ],
    )
    .unwrap();

    let query = parse_join(
        "SELECT s.id, SUM(f.amount) AS ext_price FROM fact f JOIN address a ON f.id = a.id JOIN store s ON a.id = s.id WHERE SUBSTRING(a.zip, 1, 5) <> SUBSTRING(s.zip, 1, 5) GROUP BY s.id ORDER BY ext_price DESC",
    )
    .unwrap()
    .unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &[fact, address, store], &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "id\text_price\n2\t7\n");

    fs::remove_dir_all(root).unwrap();
}
