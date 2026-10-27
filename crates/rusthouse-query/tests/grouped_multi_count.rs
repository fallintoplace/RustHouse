use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn groups_counts_by_two_keys_with_filter_having_and_sort() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-multi-key-group-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let cols = [
        Column {
            name: "region".into(),
            data_type: "UInt16".into(),
        },
        Column {
            name: "event".into(),
            data_type: "String".into(),
        },
        Column {
            name: "active".into(),
            data_type: "UInt8".into(),
        },
    ];
    let rows = [
        vec![Some("1".into()), Some("view".into()), Some("1".into())],
        vec![Some("2".into()), Some("click".into()), Some("1".into())],
        vec![Some("1".into()), Some("view".into()), Some("1".into())],
        vec![Some("1".into()), Some("click".into()), Some("0".into())],
    ];
    write_wide_part(&part, &cols, &rows).unwrap();
    let run = |sql: &str| {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&part), &mut output).unwrap();
        String::from_utf8(output).unwrap()
    };
    assert_eq!(
        run(
            "SELECT region, event, count(*) FROM part WHERE active = 1 GROUP BY region, event ORDER BY count(*) DESC, region LIMIT 2"
        ),
        "region\tevent\tcount\n1\tview\t2\n2\tclick\t1\n"
    );
    assert_eq!(
        run("SELECT region, event, count(*) FROM part GROUP BY region, event HAVING COUNT(*) > 1"),
        "region\tevent\tcount\n1\tview\t2\n"
    );
    assert_eq!(
        run(
            "SELECT region, event, count(*) FROM part GROUP BY region, event ORDER BY 3 DESC LIMIT 1"
        ),
        "region\tevent\tcount\n1\tview\t2\n"
    );
    assert_eq!(
        run(
            "SELECT region, event, count(*) FROM part GROUP BY region, event ORDER BY count(*) DESC, region ASC LIMIT 1 OFFSET 1"
        ),
        "region\tevent\tcount\n1\tclick\t1\n"
    );

    assert_eq!(
        run(
            "SELECT region, event, count(*) AS c FROM part GROUP BY region, event HAVING c > 1 ORDER BY c DESC"
        ),
        "region\tevent\tc\n1\tview\t2\n"
    );
    assert!(
        parse_select("SELECT region, event, count(*) FROM part GROUP BY event, region").is_err()
    );
    fs::remove_dir_all(part).unwrap();
}
