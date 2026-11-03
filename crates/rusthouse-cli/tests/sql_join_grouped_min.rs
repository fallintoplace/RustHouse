use std::fs;
use std::process::Command;

#[test]
fn grouped_min_across_inner_joins() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-min-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE f (id UInt64, fk UInt64)",
        "CREATE TABLE d (dk UInt64, next UInt64, grp String)",
        "CREATE TABLE e (ek UInt64, amount Nullable(Int64))",
        "CREATE TABLE costs (ck UInt64, cost Nullable(Float64))",
        "INSERT INTO f VALUES (1, 10), (2, 10), (3, 20), (4, 30)",
        "INSERT INTO d VALUES (10, 100, 'x'), (10, 101, 'x'), (20, 102, 'y'), (30, 103, 'z')",
        "INSERT INTO e VALUES (100, 5), (101, -2), (102, NULL), (103, 7)",
        "INSERT INTO costs VALUES (100, 1.25), (101, -2.5), (102, NULL), (103, 3.75)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT d.grp, MIN(e.amount) AS lowest FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek GROUP BY d.grp ORDER BY d.grp",
            "d.grp\tlowest\nx\t-2\ny\t\\N\nz\t7\n",
        ),
        (
            "SELECT d.grp, MIN(e.amount) AS lowest FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek GROUP BY d.grp ORDER BY lowest DESC NULLS LAST LIMIT 2",
            "d.grp\tlowest\nz\t7\nx\t-2\n",
        ),
        (
            "SELECT d.grp, MIN(e.amount) AS lowest FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE grp LIKE 'x%' GROUP BY d.grp",
            "d.grp\tlowest\nx\t-2\n",
        ),
        (
            "SELECT d.grp, MIN(d.next) AS lowest FROM f JOIN d ON f.fk = d.dk GROUP BY d.grp ORDER BY d.grp",
            "d.grp\tlowest\nx\t100\ny\t102\nz\t103\n",
        ),
        (
            "SELECT d.grp, MIN(e.amount) AS lowest FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek WHERE grp = 'none' GROUP BY d.grp",
            "d.grp\tlowest\n",
        ),
        (
            "SELECT d.grp, MIN(costs.cost) AS lowest FROM f JOIN d ON f.fk = d.dk JOIN costs ON d.next = costs.ck GROUP BY d.grp ORDER BY lowest ASC NULLS LAST",
            "d.grp\tlowest\nx\t-2.5\nz\t3.75\ny\t\\N\n",
        ),
        (
            "SELECT d.grp, MIN(e.amount) AS lowest, f.id FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek GROUP BY d.grp, f.id ORDER BY d.grp, f.id LIMIT 2 OFFSET 1",
            "d.grp\tlowest\tf.id\nx\t-2\t2\ny\t\\N\t3\n",
        ),
    ] {
        let result = sql(query);
        assert!(
            result.status.success(),
            "{query}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT d.grp, MIN(e.missing) FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek GROUP BY d.grp LIMIT 0",
        "SELECT d.grp, MIN(d.grp) FROM f JOIN d ON f.fk = d.dk GROUP BY d.grp LIMIT 0",
        "SELECT d.grp, MIN(e.amount) FROM f LEFT JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek GROUP BY d.grp",
        "SELECT d.grp, MIN(e.amount), SUM(e.amount) FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek GROUP BY d.grp",
        "SELECT d.grp, MIN(e.amount) AS lowest FROM f JOIN d ON f.fk = d.dk JOIN e ON d.next = e.ek GROUP BY d.grp HAVING lowest > 1",
    ] {
        assert!(!sql(query).status.success(), "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
