use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(query: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn mixed_grouped_count_star_preserves_nulls_order_and_metric_aliases() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-grouped-mixed-count-star-{}",
        std::process::id()
    ));
    for query in [
        "CREATE TABLE rides (zone Nullable(UInt8), amount Nullable(Int64), payment UInt8)",
        "INSERT INTO rides VALUES (1, 10, 1), (1, NULL, 1), (1, 20, 0), (2, NULL, 1), (NULL, 9, 1)",
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
    }

    let cases: &[(&str, &[u8])] = &[
        (
            "SELECT zone, AVG(amount) AS mean, COUNT(*) AS trips FROM rides GROUP BY zone ORDER BY zone ASC NULLS LAST",
            b"zone\tmean\ttrips\n1\t15\t3\n2\tNULL\t1\n\\N\t9\t1\n",
        ),
        (
            "SELECT zone, SUM(amount), COUNT(*) FROM rides GROUP BY zone ORDER BY zone ASC NULLS LAST",
            b"zone\tsum(amount)\tcount(*)\n1\t30\t3\n2\tNULL\t1\n\\N\t9\t1\n",
        ),
        (
            "SELECT zone, COUNT(*) AS trips, AVG(amount) AS mean FROM rides GROUP BY zone ORDER BY trips DESC, zone ASC",
            b"zone\ttrips\tmean\n1\t3\t15\n2\t1\tNULL\n\\N\t1\t9\n",
        ),
        (
            "SELECT zone, SUM(amount), COUNT(amount), COUNT(*) FROM rides GROUP BY zone ORDER BY zone ASC NULLS LAST",
            b"zone\tsum(amount)\tcount(amount)\tcount(*)\n1\t30\t2\t3\n2\tNULL\t0\t1\n\\N\t9\t1\n",
        ),
        (
            "SELECT zone, SUM(amount), COUNT(*) FROM rides GROUP BY zone HAVING COUNT(*) > 1 ORDER BY zone",
            b"zone\tsum(amount)\tcount(*)\n1\t30\t3\n",
        ),
        (
            "SELECT zone, SUM(amount), COUNT(*) FROM rides GROUP BY zone ORDER BY COUNT(*) DESC, zone ASC LIMIT 1",
            b"zone\tsum(amount)\tcount(*)\n1\t30\t3\n",
        ),
        (
            "SELECT zone, payment, SUM(amount), COUNT(*) FROM rides GROUP BY zone, payment ORDER BY zone ASC NULLS LAST, payment",
            b"zone\tpayment\tsum(amount)\tcount(*)\n1\t0\t20\t1\n1\t1\t10\t2\n2\t1\tNULL\t1\n\\N\t1\t9\t1\n",
        ),
        (
            "SELECT number, SUM(number), COUNT(*) FROM numbers(3) GROUP BY number ORDER BY number",
            b"number\tsum(number)\tcount(*)\n0\t0\t1\n1\t1\t1\n2\t2\t1\n",
        ),
    ];
    for &(query, expected) in cases {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
