use rusthouse_query::{Projection, execute, explain_select, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn groups_distinct_values_and_ignores_nulls() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-grouped-distinct-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column {
            name: "RegionID".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "UserID".into(),
            data_type: "Nullable(UInt64)".into(),
        },
        Column {
            name: "SearchPhrase".into(),
            data_type: "String".into(),
        },
    ];
    let values = [
        (1, Some(7), "alpha"),
        (1, Some(7), "alpha"),
        (1, Some(8), "beta"),
        (2, Some(9), "alpha"),
        (2, None, "alpha"),
        (3, None, ""),
        (1, Some(9), ""),
    ];
    let rows = values
        .iter()
        .map(|(region, user, phrase)| {
            vec![
                Some(region.to_string()),
                user.map(|value| value.to_string()),
                Some((*phrase).to_owned()),
            ]
        })
        .collect::<Vec<_>>();
    write_wide_part(&root, &columns, &rows).unwrap();
    let run = |statement: &str| {
        let parsed = parse_select(statement).unwrap();
        let mut output = Vec::new();
        execute(&parsed, Some(&root), &mut output).unwrap();
        String::from_utf8(output).unwrap()
    };

    let q9 = "SELECT RegionID, COUNT(DISTINCT UserID) AS u FROM part \
              GROUP BY RegionID ORDER BY u DESC LIMIT 10";
    assert_eq!(run(q9), "RegionID\tu\n1\t3\n2\t1\n3\t0\n");
    let parsed = parse_select(q9).unwrap();
    assert!(matches!(
        &parsed.projection,
        Projection::GroupedDistinctCount { .. }
    ));
    assert!(explain_select(&parsed).contains("count(DISTINCT UserID)"));

    let q14 = "SELECT SearchPhrase, COUNT(DISTINCT UserID) AS u FROM part \
               WHERE SearchPhrase <> '' GROUP BY SearchPhrase \
               ORDER BY u DESC, SearchPhrase ASC LIMIT 10";
    assert_eq!(run(q14), "SearchPhrase\tu\nalpha\t2\nbeta\t1\n");
    assert_eq!(
        run("SELECT RegionID, COUNT(DISTINCT UserID) FROM part \
             GROUP BY 1 ORDER BY 2 DESC, 1 ASC LIMIT 2 OFFSET 1"),
        "RegionID\tcountDistinct(UserID)\n2\t1\n3\t0\n"
    );
    assert_eq!(
        run("SELECT RegionID, COUNT(DISTINCT UserID) FROM part \
             GROUP BY RegionID LIMIT 0"),
        "RegionID\tcountDistinct(UserID)\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_invalid_grouped_distinct_syntax() {
    for sql in [
        "SELECT RegionID, COUNT(DISTINCT UserID) FROM part",
        "SELECT RegionID, COUNT(DISTINCT) FROM part GROUP BY RegionID",
        "SELECT RegionID, COUNT(DISTINCT UserID) FROM part GROUP BY UserID",
        "SELECT RegionID, SearchPhrase, COUNT(DISTINCT UserID) FROM part GROUP BY RegionID, SearchPhrase",
        "SELECT RegionID, COUNT(DISTINCT UserID) FROM part GROUP BY RegionID ORDER BY missing",
        "SELECT RegionID, COUNT(DISTINCT UserID) FROM part GROUP BY RegionID HAVING u > 1",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn groups_repeated_string_values_without_counting_nulls() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-string-grouped-distinct-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column {
            name: "segment".into(),
            data_type: "String".into(),
        },
        Column {
            name: "tag".into(),
            data_type: "Nullable(String)".into(),
        },
    ];
    let rows = [
        vec![Some("east".into()), Some("a".into())],
        vec![Some("east".into()), Some("a".into())],
        vec![Some("east".into()), Some("b".into())],
        vec![Some("east".into()), None],
        vec![Some("west".into()), Some("x".into())],
        vec![Some("west".into()), Some("x".into())],
        vec![Some("west".into()), None],
    ];
    write_wide_part(&root, &columns, &rows).unwrap();
    let mut output = Vec::new();
    execute(
        &parse_select(
            "SELECT segment, COUNT(DISTINCT tag) FROM part GROUP BY segment ORDER BY segment",
        )
        .unwrap(),
        Some(&root),
        &mut output,
    )
    .unwrap();
    assert_eq!(output, b"segment\tcountDistinct(tag)\neast\t2\nwest\t1\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dense_byte_grouped_distinct_keeps_first_seen_signed_keys() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-byte-grouped-distinct-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    let columns = [
        Column { name: "segment".into(), data_type: "Int8".into() },
        Column { name: "tag".into(), data_type: "Nullable(UInt64)".into() },
    ];
    let rows = [
        vec![Some("-1".into()), Some("7".into())],
        vec![Some("3".into()), Some("10".into())],
        vec![Some("-1".into()), Some("7".into())],
        vec![Some("-1".into()), Some("8".into())],
        vec![Some("3".into()), None],
    ];
    write_wide_part(&root, &columns, &rows).unwrap();
    let mut output = Vec::new();
    execute(
        &parse_select("SELECT segment, COUNT(DISTINCT tag) FROM part GROUP BY segment").unwrap(),
        Some(&root),
        &mut output,
    ).unwrap();
    assert_eq!(output, b"segment\tcountDistinct(tag)\n-1\t2\n3\t1\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn grouped_distinct_byte_values_use_exact_bitmap_and_skip_nulls() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-byte-values-distinct-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    let columns = [
        Column { name: "region".into(), data_type: "UInt16".into() },
        Column { name: "event_code".into(), data_type: "Nullable(Int8)".into() },
    ];
    let rows = [
        vec![Some("2".into()), Some("-1".into())],
        vec![Some("1".into()), Some("7".into())],
        vec![Some("1".into()), Some("7".into())],
        vec![Some("1".into()), Some("-128".into())],
        vec![Some("2".into()), None],
        vec![Some("2".into()), Some("-1".into())],
    ];
    write_wide_part(&root, &columns, &rows).unwrap();
    let query = parse_select(
        "SELECT region, COUNT(DISTINCT event_code) FROM part GROUP BY region ORDER BY region"
    ).unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&root), &mut output).unwrap();
    assert_eq!(output, b"region\tcountDistinct(event_code)\n1\t2\n2\t1\n");
    fs::remove_dir_all(root).unwrap();
}
