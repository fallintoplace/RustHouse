//! Sort single-metric grouped reports by the selected aggregate expression.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn grouped_single_aggregate_order_expression_with_having_and_pagination() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-grouped-aggregate-order-expression-{}",
        std::process::id()
    ));
    let columns = [
        Column {
            name: "region".into(),
            data_type: "UInt8".into(),
        },
        Column {
            name: "category".into(),
            data_type: "UInt8".into(),
        },
        Column {
            name: "amount".into(),
            data_type: "UInt64".into(),
        },
    ];
    let rows = [
        ("0", "1", "75000"),
        ("2", "1", "50000"),
        ("0", "2", "150000"),
        ("2", "2", "50000"),
        ("0", "3", "100000"),
        ("2", "3", "80000"),
        ("0", "4", "90000"),
        ("2", "4", "110000"),
        ("1", "4", "900000"),
        ("2", "5", "105000"),
        ("0", "6", "99000"),
    ]
    .map(|(region, category, amount)| {
        vec![
            Some(region.into()),
            Some(category.into()),
            Some(amount.into()),
        ]
    });
    write_wide_part(&dir, &columns, &rows).unwrap();

    let run = |sql: &str| {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&dir), &mut output).unwrap();
        String::from_utf8(output).unwrap()
    };
    let sql = "SELECT category, SUM(amount) FROM part WHERE region IN (0, 2) GROUP BY category HAVING SUM(amount) > 100000 ORDER BY SUM(amount) DESC, category ASC LIMIT 3 OFFSET 1";
    let expected = "category\tsum(amount)\n4\t200000\n3\t180000\n1\t125000\n";
    assert_eq!(run(sql), expected);

    // The aggregate expression and ordinal must resolve to the same sort key.
    assert_eq!(
        run("SELECT category, SUM(amount) FROM part WHERE region IN (0, 2) GROUP BY category HAVING SUM(amount) > 100000 ORDER BY 2 DESC, category ASC LIMIT 3 OFFSET 1"),
        expected
    );
    assert_eq!(
        run("SELECT category, SUM(amount) FROM part WHERE region IN (0, 2) AND category IN (1, 3, 5) GROUP BY category ORDER BY sum(amount) DESC"),
        "category\tsum(amount)\n3\t180000\n1\t125000\n5\t105000\n"
    );

    // Multi-metric projections already support aliases; keep that path intact.
    assert_eq!(
        run("SELECT category, SUM(amount) AS revenue FROM part WHERE region IN (0, 2) GROUP BY category HAVING SUM(amount) > 100000 ORDER BY revenue DESC, category ASC LIMIT 3 OFFSET 1"),
        "category\trevenue\n4\t200000\n3\t180000\n1\t125000\n"
    );

    for invalid in [
        "SELECT category, SUM(amount) FROM part GROUP BY category ORDER BY MAX(amount)",
        "SELECT category, SUM(amount) FROM part GROUP BY category ORDER BY SUM(region)",
        "SELECT category, SUM(amount) FROM part GROUP BY category ORDER BY SUM(amount",
    ] {
        assert!(parse_select(invalid).is_err(), "{invalid}");
    }

    fs::remove_dir_all(dir).unwrap();
}
