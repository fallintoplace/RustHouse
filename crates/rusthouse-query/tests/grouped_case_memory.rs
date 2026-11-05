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

#[test]
fn compact_case_keys_preserve_escaped_text_and_exact_integer_output() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-grouped-case-output-{}",
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
            data_type: "UInt8".into(),
        },
        Column {
            name: "adv".into(),
            data_type: "UInt8".into(),
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
    let escaped_source = "src\tline\nslash\\zero\0control\u{1}";
    let escaped_url = "url\tline\n";
    let rows = [
        vec![
            Some("-2".into()),
            Some("0".into()),
            Some("0".into()),
            Some(escaped_source.into()),
            Some(escaped_url.into()),
        ],
        vec![
            Some("-2".into()),
            Some("0".into()),
            Some("0".into()),
            Some(escaped_source.into()),
            Some(escaped_url.into()),
        ],
        vec![
            Some("2".into()),
            Some("1".into()),
            Some("0".into()),
            Some("unused".into()),
            Some("url-b".into()),
        ],
        vec![
            Some("10".into()),
            Some("0".into()),
            Some("1".into()),
            Some("unused".into()),
            Some("url-c".into()),
        ],
    ];
    write_wide_part(&part, &columns, &rows).unwrap();

    let sql = "SELECT traffic, search, adv, CASE WHEN (search = 0 AND adv = 0) THEN referer ELSE '' END AS Src, url AS Dst, COUNT(*) AS PageViews FROM part GROUP BY traffic, search, adv, Src, Dst ORDER BY PageViews DESC LIMIT 10";
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(&part), &mut output).unwrap();
    assert_eq!(
        output,
        b"traffic\tsearch\tadv\tSrc\tDst\tPageViews\n-2\t0\t0\tsrc\\tline\\nslash\\\\zero\\0control\\x01\turl\\tline\\n\t2\n2\t1\t0\t\turl-b\t1\n10\t0\t1\t\turl-c\t1\n"
    );
    fs::remove_dir_all(part).unwrap();
}
