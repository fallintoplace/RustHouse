use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn computes_multiple_aggregates_in_one_filtered_scan() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-multi-aggregate-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column {
            name: "amount".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "width".into(),
            data_type: "UInt16".into(),
        },
        Column {
            name: "tag".into(),
            data_type: "String".into(),
        },
    ];
    let rows = [
        vec![Some("2".into()), Some("10".into()), Some("ok".into())],
        vec![Some("3".into()), Some("20".into()), Some("db".into())],
        vec![Some("4".into()), Some("30".into()), Some("ok".into())],
    ];
    write_wide_part(&part, &columns, &rows).unwrap();
    let run = |sql: &str| {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&part), &mut output).unwrap();
        String::from_utf8(output).unwrap()
    };
    assert_eq!(
        run("SELECT sum(amount), count(*), avg(width) FROM part"),
        "sum(amount)\tcount(*)\tavg(width)\n9\t3\t20\n"
    );
    assert_eq!(
        run("SELECT sum(width + 1), sum(width + 2) FROM part"),
        "sum(width + 1)\tsum(width + 2)\n63\t66\n"
    );
    assert_eq!(
        run("SELECT min(amount), max(amount), count(tag) FROM part WHERE tag = 'ok'"),
        "min(amount)\tmax(amount)\tcount(tag)\n2\t4\t2\n"
    );
    assert_eq!(
        run("SELECT count(DISTINCT tag), count(*), sum(width) FROM part"),
        "count(DISTINCT tag)\tcount(*)\tsum(width)\n2\t3\t60\n"
    );
    assert_eq!(
        run("SELECT count(DISTINCT amount + 1), count(*) FROM part WHERE tag = 'ok'"),
        "count(DISTINCT amount + 1)\tcount(*)\n2\t2\n"
    );
    assert_eq!(
        run("SELECT sum(length(tag)), avg(length(tag)) FROM part WHERE amount > 2"),
        "sum(length(tag))\tavg(length(tag))\n4\t2\n"
    );
    fs::remove_dir_all(part).unwrap();
}
