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
    let paths = [fact, address, store];
    let base = "SELECT f.id FROM fact f JOIN address a ON f.id=a.id JOIN store s ON a.id=s.id WHERE ";
    let run = |suffix: &str| {
        let query = parse_join(&format!("{base}{suffix}")).unwrap().unwrap();
        let mut output = Vec::new();
        execute_multi_join(&query, &paths, &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("f.id"));
        let mut rows = lines.collect::<Vec<_>>();
        rows.sort_unstable();
        rows
    };
    assert_eq!(
        run("SUBSTR(a.zip, 1, 5) = SUBSTRING(s.zip FROM 1 FOR 5)"),
        vec!["1"]
    );
    assert_eq!(
        run("SUBSTRING(a.zip, 1, 5) <> SUBSTR(s.zip, 1, 5)"),
        vec!["2"]
    );
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
