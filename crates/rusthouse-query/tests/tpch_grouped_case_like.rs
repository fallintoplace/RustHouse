use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};

#[test]
fn grouped_case_like_matches_promotional_prefix_and_retains_zero_groups() {
    let path = std::env::temp_dir().join(format!("rusthouse-tpch-promo-case-{}", std::process::id()));
    let columns = [
        Column { name: "region".into(), data_type: "String".into() },
        Column { name: "p_type".into(), data_type: "String".into() },
        Column { name: "amount".into(), data_type: "UInt64".into() },
    ];
    let rows = [
        vec![Some("east".into()), Some("PROMO X".into()), Some("10".into())],
        vec![Some("east".into()), Some("OTHER".into()), Some("20".into())],
        vec![Some("west".into()), Some("OTHER".into()), Some("30".into())],
    ];
    write_wide_part(&path, &columns, &rows).unwrap();
    let sql = "SELECT region, sum(CASE WHEN p_type LIKE 'PROMO%' THEN amount ELSE 0 END) AS promo FROM part GROUP BY region ORDER BY region";
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(&path), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "region\tpromo\neast\t10\nwest\t0\n");
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn grouped_case_like_requires_a_string_pattern() {
    assert!(parse_select("SELECT region, sum(CASE WHEN p_type LIKE 5 THEN amount ELSE 0 END) FROM part GROUP BY region").is_err());
}
