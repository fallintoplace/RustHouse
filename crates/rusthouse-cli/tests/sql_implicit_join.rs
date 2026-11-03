use std::fs;
use std::process::Command;

#[test]
fn implicit_inner_join_matches_explicit_join() {
    let db = std::env::temp_dir().join(format!("rusthouse-implicit-join-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE lefts (id UInt64, k Nullable(UInt64))",
        "CREATE TABLE rights (id UInt64, k Nullable(UInt64), name String)",
        "CREATE TABLE thirds (id UInt64, label String)",
        "INSERT INTO lefts VALUES (1, 10), (2, NULL), (3, 10)",
        "INSERT INTO rights VALUES (1, 10, 'one'), (3, 10, 'three'), (3, 10, 'again'), (2, NULL, 'null')",
        "INSERT INTO thirds VALUES (1, 'a'), (3, 'c')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (implicit, explicit) in [
        (
            "SELECT l.id, r.name FROM lefts l, rights r WHERE l.id = r.id AND l.k = r.k ORDER BY l.id",
            "SELECT l.id, r.name FROM lefts l JOIN rights r ON l.id = r.id AND l.k = r.k ORDER BY l.id",
        ),
        (
            "SELECT l.id, r.name, t.label FROM lefts l, rights r, thirds t WHERE t.id = r.id AND l.id = r.id ORDER BY l.id",
            "SELECT l.id, r.name, t.label FROM lefts l JOIN rights r ON l.id = r.id JOIN thirds t ON t.id = r.id ORDER BY l.id",
        ),
        (
            "SELECT l.id, r.name, t.label FROM lefts l, rights r, thirds t WHERE l.id = t.id AND r.id = t.id ORDER BY l.id",
            "SELECT l.id, r.name, t.label FROM lefts l JOIN thirds t ON l.id = t.id JOIN rights r ON r.id = t.id ORDER BY l.id",
        ),
        (
            "SELECT l.*, r.*, t.* FROM lefts l, rights r, thirds t WHERE l.id = t.id AND r.id = t.id ORDER BY l.id",
            "SELECT l.*, r.*, t.* FROM lefts l JOIN thirds t ON l.id = t.id JOIN rights r ON r.id = t.id ORDER BY l.id",
        ),
        (
            "SELECT count(*) FROM lefts l, rights r, thirds t WHERE l.id = t.id AND r.id = t.id",
            "SELECT count(*) FROM lefts l JOIN thirds t ON l.id = t.id JOIN rights r ON r.id = t.id",
        ),
        (
            "SELECT count(*) FROM lefts l, rights r WHERE l.id = r.id AND l.id > 1",
            "SELECT count(*) FROM lefts l JOIN rights r ON l.id = r.id WHERE l.id > 1",
        ),
    ] {
        let actual = sql(implicit);
        let expected = sql(explicit);
        assert!(actual.status.success(), "{implicit}: {actual:?}");
        assert!(expected.status.success(), "{explicit}: {expected:?}");
        assert_eq!(actual.stdout, expected.stdout, "{implicit}");
    }
    for statement in [
        "CREATE TABLE orders (o_orderkey UInt64)",
        "CREATE TABLE lineitem (l_orderkey UInt64)",
        "INSERT INTO orders VALUES (1), (2)",
        "INSERT INTO lineitem VALUES (1), (1), (3)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let unqualified = sql("SELECT count(*) FROM orders, lineitem WHERE o_orderkey = l_orderkey");
    assert!(unqualified.status.success(), "{unqualified:?}");
    assert_eq!(unqualified.stdout, b"count\n2\n");
    for statement in [
        "CREATE TABLE part (p_key UInt64)",
        "CREATE TABLE supplier (s_key UInt64)",
        "CREATE TABLE partsupp (ps_part UInt64, ps_supplier UInt64)",
        "INSERT INTO part VALUES (1), (2)",
        "INSERT INTO supplier VALUES (3), (4)",
        "INSERT INTO partsupp VALUES (1, 3), (1, 4), (2, 3)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let connected = sql(
        "SELECT count(*) FROM part, supplier, partsupp WHERE p_key = ps_part AND s_key = ps_supplier",
    );
    assert!(connected.status.success(), "{connected:?}");
    assert_eq!(connected.stdout, b"count\n3\n");
    let same_table = sql(
        "SELECT count(*) FROM part p, partsupp ps WHERE p_key = ps.ps_part AND p_key = p.p_key",
    );
    assert!(same_table.status.success(), "{same_table:?}");
    assert_eq!(same_table.stdout, b"count\n3\n");
    for query in [
        "SELECT count(*) FROM part p, supplier s, partsupp ps WHERE p_key = ps.ps_part AND s.s_key = ps_supplier",
        "SELECT count(*) FROM part p, supplier s, partsupp ps WHERE ps.ps_part = p_key AND ps_supplier = s.s_key",
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, b"count\n3\n");
    }
    let mixed_ambiguous = sql("SELECT count(*) FROM lefts l, rights r WHERE id = r.id");
    assert!(!mixed_ambiguous.status.success(), "{mixed_ambiguous:?}");
    assert!(String::from_utf8_lossy(&mixed_ambiguous.stderr).contains("ambiguous JOIN column"));
    assert!(!sql("SELECT count(*) FROM part p, partsupp ps WHERE p_key = ps.ps_part AND p_key = unknown.ps_part").status.success());
    assert!(
        !sql("SELECT count(*) FROM part p, partsupp ps WHERE missing = ps.ps_part")
            .status
            .success()
    );
    let ambiguous = sql("SELECT count(*) FROM lefts, rights WHERE id = id");
    assert!(!ambiguous.status.success(), "{ambiguous:?}");
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("ambiguous JOIN column"));
    // Reordering must not change the column order of an unqualified star.
    assert!(
        !sql("SELECT * FROM lefts l, rights r, thirds t WHERE l.id = t.id AND r.id = t.id")
            .status
            .success()
    );
    let filtered =
        sql("SELECT l.id FROM lefts l, rights r WHERE l.id = r.id AND r.id = 1 AND r.k = 10");
    assert!(filtered.status.success(), "{filtered:?}");
    assert_eq!(filtered.stdout, b"l.id\n1\n");
    for query in [
        "SELECT l.id FROM lefts l, rights r WHERE l.id = r.id OR l.id = 1",
        "SELECT l.id FROM lefts l, rights r WHERE l.id = 1",
    ] {
        assert!(!sql(query).status.success(), "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
