use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn collapsed_case_keys_do_not_retain_unselected_referers() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-grouped-case-memory-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&part);
    let columns = [
        Column {
            name: "traffic".into(),
            data_type: "Int16".into(),
        },
        Column {
            name: "search".into(),
            data_type: "UInt16".into(),
        },
        Column {
            name: "adv".into(),
            data_type: "UInt16".into(),
        },
        Column {
            name: "referer".into(),
            data_type: "String".into(),
        },
        Column {
            name: "url".into(),
            data_type: "String".into(),
        },
    ];
    let rows = (0..130_000)
        .map(|row| {
            vec![
                Some("1".into()),
                Some("1".into()),
                Some("1".into()),
                Some(format!("referer-{row}")),
                Some("same-url".into()),
            ]
        })
        .collect::<Vec<_>>();
    write_wide_part(&part, &columns, &rows).unwrap();

    let sql = "SELECT traffic, search, adv, CASE WHEN (search = 0 AND adv = 0) THEN referer ELSE '' END AS Src, url AS Dst, COUNT(*) AS PageViews FROM part GROUP BY traffic, search, adv, Src, Dst ORDER BY PageViews DESC LIMIT 10";
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(&part), &mut output).unwrap();
    assert_eq!(
        output,
        b"traffic\tsearch\tadv\tSrc\tDst\tPageViews\n1\t1\t1\t\tsame-url\t130000\n"
    );
    fs::remove_dir_all(part).unwrap();
}
