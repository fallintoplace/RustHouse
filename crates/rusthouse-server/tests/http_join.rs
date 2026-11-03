use rusthouse_server::{Config, handle_request};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::Path;

fn table(database: &Path, name: &str, columns: &[Column], rows: &[Vec<Option<String>>]) {
    let directory = database.join(format!("{name}.parts"));
    fs::create_dir_all(&directory).unwrap();
    write_wide_part(&directory.join("all_0_0_0"), columns, rows).unwrap();
    let mut schema = String::from("rusthouse-schema-v1\n");
    for column in columns {
        schema.push_str(&format!("{}\t{}\n", column.name, column.data_type));
    }
    fs::write(database.join(format!("{name}.schema")), schema).unwrap();
}

#[test]
fn sql_http_executes_tpch_q15() {
    let database = std::env::temp_dir().join(format!("rusthouse-http-q15-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    let cols = |values: &[(&str, &str)]| {
        values
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).into(),
                data_type: (*kind).into(),
            })
            .collect::<Vec<_>>()
    };
    let row = |values: &[&str]| {
        values
            .iter()
            .map(|value| Some((*value).into()))
            .collect::<Vec<_>>()
    };
    table(
        &database,
        "supplier",
        &cols(&[
            ("s_suppkey", "UInt64"),
            ("s_name", "String"),
            ("s_address", "String"),
            ("s_phone", "String"),
        ]),
        &[row(&["1", "A", "one", "11"]), row(&["2", "B", "two", "22"])],
    );
    table(
        &database,
        "lineitem",
        &cols(&[
            ("l_suppkey", "UInt64"),
            ("l_extendedprice", "Float64"),
            ("l_discount", "Float64"),
            ("l_shipdate", "Date"),
        ]),
        &[
            row(&["1", "100", "0", "1996-01-01"]),
            row(&["2", "100", "0", "1996-02-01"]),
        ],
    );
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let query = include_str!("../../../bench/tpch/queries/q15.sql");
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"s_suppkey\ts_name\ts_address\ts_phone\ttotal_revenue\n1\tA\tone\t11\t100\n2\tB\ttwo\t22\t100\n");
    let limited = handle_request(
        "POST",
        "/?format=tsv-no-header",
        query.as_bytes(),
        &Config {
            max_rows: 1,
            ..config.clone()
        },
    );
    assert_eq!(limited.status, "200 OK", "{:?}", limited.body);
    assert_eq!(limited.body, b"1\tA\tone\t11\t100\n");
    let json = handle_request("POST", "/?format=json", query.as_bytes(), &config);
    assert_eq!(json.status, "200 OK", "{:?}", json.body);
    let text = String::from_utf8_lossy(&json.body);
    assert!(
        text.contains(
            "\"s_name\":\"A\",\"s_address\":\"one\",\"s_phone\":\"11\",\"total_revenue\":\"100\""
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "\"s_name\":\"B\",\"s_address\":\"two\",\"s_phone\":\"22\",\"total_revenue\":\"100\""
        ),
        "{text}"
    );
    let invalid = query.replace("max(total_revenue)", "min(total_revenue)");
    assert_eq!(
        handle_request("POST", "/", invalid.as_bytes(), &config).status,
        "400 Bad Request"
    );
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn sql_http_executes_tpch_q13() {
    let database = std::env::temp_dir().join(format!("rusthouse-http-q13-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    let cols = |values: &[(&str, &str)]| {
        values
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).into(),
                data_type: (*kind).into(),
            })
            .collect::<Vec<_>>()
    };
    let row = |values: &[&str]| {
        values
            .iter()
            .map(|value| Some((*value).into()))
            .collect::<Vec<_>>()
    };
    table(
        &database,
        "customer",
        &cols(&[("c_custkey", "UInt64")]),
        &[row(&["1"]), row(&["2"]), row(&["3"])],
    );
    table(
        &database,
        "orders",
        &cols(&[
            ("o_orderkey", "UInt64"),
            ("o_custkey", "UInt64"),
            ("o_comment", "String"),
        ]),
        &[
            row(&["11", "1", "regular"]),
            row(&["12", "1", "regular"]),
            row(&["13", "1", "special requests"]),
            row(&["21", "2", "regular"]),
        ],
    );
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let query = include_str!("../../../bench/tpch/queries/q13.sql");
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"c_count\tcustdist\n2\t1\n1\t1\n0\t1\n");
    let limited = handle_request(
        "POST",
        "/?format=tsv-no-header",
        query.as_bytes(),
        &Config {
            max_rows: 2,
            ..config.clone()
        },
    );
    assert_eq!(limited.status, "200 OK", "{:?}", limited.body);
    assert_eq!(limited.body, b"2\t1\n1\t1\n");
    let json = handle_request("POST", "/?format=json", query.as_bytes(), &config);
    assert_eq!(json.status, "200 OK", "{:?}", json.body);
    let json_text = String::from_utf8_lossy(&json.body);
    for value in ["0", "1", "2"] {
        assert!(
            json_text.contains(&format!("\"c_count\":\"{value}\",\"custdist\":\"1\"")),
            "{json_text}"
        );
    }
    let invalid = query.replace("o_comment NOT LIKE", "o_orderkey NOT LIKE");
    assert_eq!(
        handle_request("POST", "/", invalid.as_bytes(), &config).status,
        "400 Bad Request"
    );
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn sql_http_executes_tpch_q17() {
    let database = std::env::temp_dir().join(format!("rusthouse-http-q17-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    let cols = |values: &[(&str, &str)]| {
        values
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).into(),
                data_type: (*kind).into(),
            })
            .collect::<Vec<_>>()
    };
    let row = |values: &[&str]| {
        values
            .iter()
            .map(|value| Some((*value).into()))
            .collect::<Vec<_>>()
    };
    table(
        &database,
        "part",
        &cols(&[
            ("p_partkey", "UInt64"),
            ("p_brand", "String"),
            ("p_container", "String"),
        ]),
        &[row(&["1", "Brand#23", "MED BOX"])],
    );
    table(
        &database,
        "lineitem",
        &cols(&[
            ("l_partkey", "UInt64"),
            ("l_quantity", "Float64"),
            ("l_extendedprice", "Float64"),
        ]),
        &[
            row(&["1", "1", "70"]),
            row(&["1", "20", "14"]),
            row(&["1", "30", "21"]),
        ],
    );
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let query = include_str!("../../../bench/tpch/queries/q17.sql");
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"avg_yearly\n10\n");
    let no_header = handle_request("POST", "/?format=tsv-no-header", query.as_bytes(), &config);
    assert_eq!(no_header.status, "200 OK", "{:?}", no_header.body);
    assert_eq!(no_header.body, b"10\n");
    let invalid = query.replace("l_partkey = p_partkey)", "l_partkey = missing)");
    assert_eq!(
        handle_request("POST", "/", invalid.as_bytes(), &config).status,
        "400 Bad Request"
    );
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn sql_http_executes_tpch_q21() {
    let database = std::env::temp_dir().join(format!("rusthouse-http-q21-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    let cols = |values: &[(&str, &str)]| {
        values
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).into(),
                data_type: (*kind).into(),
            })
            .collect::<Vec<_>>()
    };
    let row = |values: &[&str]| {
        values
            .iter()
            .map(|value| Some((*value).into()))
            .collect::<Vec<_>>()
    };
    table(
        &database,
        "nation",
        &cols(&[("n_nationkey", "UInt64"), ("n_name", "String")]),
        &[row(&["1", "SAUDI ARABIA"])],
    );
    table(
        &database,
        "supplier",
        &cols(&[
            ("s_suppkey", "UInt64"),
            ("s_nationkey", "UInt64"),
            ("s_name", "String"),
        ]),
        &[row(&["1", "1", "A"])],
    );
    table(
        &database,
        "orders",
        &cols(&[("o_orderkey", "UInt64"), ("o_orderstatus", "String")]),
        &[row(&["11", "F"])],
    );
    table(
        &database,
        "lineitem",
        &cols(&[
            ("l_orderkey", "UInt64"),
            ("l_suppkey", "UInt64"),
            ("l_receiptdate", "Date"),
            ("l_commitdate", "Date"),
        ]),
        &[
            row(&["11", "1", "1995-01-03", "1995-01-02"]),
            row(&["11", "2", "1995-01-01", "1995-01-02"]),
        ],
    );
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let query = include_str!("../../../bench/tpch/queries/q21.sql");
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"s_name\tnumwait\nA\t1\n");
    let json = handle_request("POST", "/?format=json", query.as_bytes(), &config);
    assert_eq!(json.status, "200 OK", "{:?}", json.body);
    assert!(String::from_utf8_lossy(&json.body).contains("\"s_name\":\"A\""));
    let no_header = handle_request("POST", "/?format=tsv-no-header", query.as_bytes(), &config);
    assert_eq!(no_header.status, "200 OK", "{:?}", no_header.body);
    assert_eq!(no_header.body, b"A\t1\n");
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn explain_join_does_not_lock_for_exists_in_literal_or_comment() {
    let missing =
        std::env::temp_dir().join(format!("rusthouse-no-database-{}", std::process::id()));
    let config = Config {
        database: Some(missing),
        part: None,
        max_rows: 10,
    };
    let query = "SELECT a.id FROM a JOIN b ON a.id = b.id WHERE a.name = 'EXISTS' -- EXISTS\n";
    let result = handle_request("POST", "/explain", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
}

#[test]
fn sql_http_executes_bounded_three_table_joins_and_explain() {
    let database =
        std::env::temp_dir().join(format!("rusthouse-http-joins-{}", std::process::id()));
    let _ = fs::remove_dir_all(&database);
    fs::create_dir_all(&database).unwrap();
    table(
        &database,
        "orders",
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "customer_id".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("10".into())],
            vec![Some("2".into()), Some("20".into())],
        ],
    );
    table(
        &database,
        "customers",
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "region_id".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("10".into()), Some("100".into())],
            vec![Some("20".into()), Some("200".into())],
        ],
    );
    table(
        &database,
        "regions",
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "label".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("100".into()), Some("east".into())],
            vec![Some("200".into()), Some("west".into())],
        ],
    );
    let sql = "SELECT o.id, r.label FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r ON c.region_id = r.id";
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 1,
    };
    let result = handle_request("POST", "/", sql.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"o.id\tr.label\n1\teast\n");

    let plan = handle_request("POST", "/explain", sql.as_bytes(), &config);
    assert_eq!(plan.status, "200 OK");
    assert!(
        String::from_utf8(plan.body)
            .unwrap()
            .contains("regions AS r")
    );

    let no_header = handle_request("POST", "/?format=tsv-no-header", sql.as_bytes(), &config);
    assert_eq!(no_header.status, "200 OK");
    assert_eq!(no_header.body, b"1\teast\n");

    let unavailable = handle_request(
        "POST",
        "/",
        sql.as_bytes(),
        &Config {
            database: None,
            part: None,
            max_rows: 1,
        },
    );
    assert_eq!(unavailable.status, "400 Bad Request");
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn sql_http_binds_grouped_in_subquery_for_explicit_join() {
    let database =
        std::env::temp_dir().join(format!("rusthouse-http-join-in-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    let columns = |names: &[&str]| {
        names
            .iter()
            .map(|name| Column {
                name: (*name).into(),
                data_type: "UInt64".into(),
            })
            .collect::<Vec<_>>()
    };
    table(
        &database,
        "a",
        &columns(&["ak"]),
        &[vec![Some("1".into())], vec![Some("2".into())]],
    );
    table(
        &database,
        "b",
        &columns(&["bk"]),
        &[vec![Some("1".into())], vec![Some("2".into())]],
    );
    table(
        &database,
        "c",
        &columns(&["ck", "amount"]),
        &[
            vec![Some("1".into()), Some("8".into())],
            vec![Some("2".into()), Some("9".into())],
        ],
    );
    table(
        &database,
        "inner_keys",
        &columns(&["ik", "quantity"]),
        &[
            vec![Some("1".into()), Some("301".into())],
            vec![Some("2".into()), Some("2".into())],
        ],
    );
    let query = "SELECT ak, SUM(amount) AS total FROM a JOIN b ON a.ak = b.bk JOIN c ON b.bk = c.ck WHERE ak IN (SELECT ik FROM inner_keys GROUP BY ik HAVING SUM(quantity) > 300) GROUP BY ak ORDER BY ak";
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"ak\ttotal\n1\t8\n");
    let exists = "SELECT ak, COUNT(*) AS n FROM a JOIN b ON a.ak = b.bk JOIN c ON b.bk = c.ck WHERE EXISTS (SELECT * FROM inner_keys t WHERE t.ik = a.ak AND t.quantity <> c.amount) GROUP BY ak ORDER BY ak";
    let result = handle_request("POST", "/", exists.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"ak\tn\n1\t1\n2\t1\n");
    let not_exists = exists.replace("WHERE EXISTS", "WHERE NOT EXISTS");
    let result = handle_request("POST", "/", not_exists.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"ak\tn\n");
    let limited = handle_request(
        "POST",
        "/",
        exists.as_bytes(),
        &Config {
            max_rows: 1,
            ..config.clone()
        },
    );
    assert_eq!(limited.status, "200 OK", "{:?}", limited.body);
    assert_eq!(limited.body, b"ak\tn\n1\t1\n");
    let invalid = exists.replace("t.ik = a.ak", "t.ik = c.missing");
    let result = handle_request("POST", "/", invalid.as_bytes(), &config);
    assert_eq!(result.status, "400 Bad Request");
    let scalar = "SELECT ak, SUM(amount) AS total FROM a JOIN b ON a.ak = b.bk JOIN c ON b.bk = c.ck GROUP BY ak HAVING total > (SELECT SUM(amount * 0.5) FROM a JOIN b ON a.ak = b.bk JOIN c ON b.bk = c.ck) ORDER BY ak";
    let result = handle_request("POST", "/", scalar.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"ak\ttotal\n2\t9\n");
    let qualified_scalar = scalar
        .replace("total >", "total <")
        .replace("SUM(amount * 0.5)", "SUM(c.amount)");
    let result = handle_request("POST", "/", qualified_scalar.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"ak\ttotal\n1\t8\n2\t9\n");
    let plan = handle_request("POST", "/explain", scalar.as_bytes(), &config);
    assert_eq!(plan.status, "200 OK");
    assert!(String::from_utf8_lossy(&plan.body).contains("scalar subquery"));
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn sql_http_filters_grouped_distinct_with_not_in_subquery() {
    let database =
        std::env::temp_dir().join(format!("rusthouse-http-join-not-in-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    table(
        &database,
        "products",
        &[
            Column {
                name: "p_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "brand".into(),
                data_type: "String".into(),
            },
        ],
        &[vec![Some("1".into()), Some("A".into())]],
    );
    table(
        &database,
        "supplies",
        &[
            Column {
                name: "product".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "supplier".into(),
                data_type: "Nullable(UInt64)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("10".into())],
            vec![Some("1".into()), Some("11".into())],
            vec![Some("1".into()), None],
        ],
    );
    table(
        &database,
        "blocked",
        &[
            Column {
                name: "bad".into(),
                data_type: "Nullable(UInt64)".into(),
            },
            Column {
                name: "note".into(),
                data_type: "String".into(),
            },
        ],
        &[vec![Some("10".into()), Some("x".into())]],
    );
    let query = "SELECT brand, COUNT(DISTINCT supplier) AS n FROM products p JOIN supplies s ON p.p_id = s.product WHERE supplier NOT IN (SELECT bad FROM blocked WHERE note LIKE '%x%') GROUP BY brand";
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"brand\tn\nA\t1\n");
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn sql_http_executes_tpch_q9() {
    let database = std::env::temp_dir().join(format!("rusthouse-http-q9-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    let cols = |values: &[(&str, &str)]| {
        values
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).into(),
                data_type: (*kind).into(),
            })
            .collect::<Vec<_>>()
    };
    let row = |values: &[&str]| values.iter().map(|v| Some((*v).into())).collect::<Vec<_>>();
    table(
        &database,
        "part",
        &cols(&[("p_partkey", "UInt64"), ("p_name", "String")]),
        &[row(&["1", "green part"])],
    );
    table(
        &database,
        "supplier",
        &cols(&[("s_suppkey", "UInt64"), ("s_nationkey", "UInt64")]),
        &[row(&["10", "100"])],
    );
    table(
        &database,
        "nation",
        &cols(&[("n_nationkey", "UInt64"), ("n_name", "String")]),
        &[row(&["100", "USA"])],
    );
    table(
        &database,
        "partsupp",
        &cols(&[
            ("ps_suppkey", "UInt64"),
            ("ps_partkey", "UInt64"),
            ("ps_supplycost", "Float64"),
        ]),
        &[row(&["10", "1", "20"])],
    );
    table(
        &database,
        "orders",
        &cols(&[("o_orderkey", "UInt64"), ("o_orderdate", "Date")]),
        &[row(&["1000", "1995-01-01"]), row(&["1001", "1994-01-01"])],
    );
    table(
        &database,
        "lineitem",
        &cols(&[
            ("l_suppkey", "UInt64"),
            ("l_partkey", "UInt64"),
            ("l_orderkey", "UInt64"),
            ("l_extendedprice", "Float64"),
            ("l_discount", "Float64"),
            ("l_quantity", "Float64"),
        ]),
        &[
            row(&["10", "1", "1000", "100", "0.1", "2"]),
            row(&["10", "1", "1001", "50", "0", "1"]),
        ],
    );
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let query = include_str!("../../../bench/tpch/queries/q09.sql");
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(
        result.body,
        b"nation\to_year\tsum_profit\nUSA\t1995\t50\nUSA\t1994\t30\n"
    );
    let limited = handle_request(
        "POST",
        "/?format=tsv-no-header",
        query.as_bytes(),
        &Config {
            max_rows: 1,
            ..config.clone()
        },
    );
    assert_eq!(limited.status, "200 OK", "{:?}", limited.body);
    assert_eq!(limited.body, b"USA\t1995\t50\n");
    let invalid = query.replace("p_name LIKE", "p_partkey LIKE");
    assert_eq!(
        handle_request("POST", "/", invalid.as_bytes(), &config).status,
        "400 Bad Request"
    );
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn sql_http_executes_tpch_q7() {
    let database = std::env::temp_dir().join(format!("rusthouse-http-q7-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    let cols = |values: &[(&str, &str)]| {
        values
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).into(),
                data_type: (*kind).into(),
            })
            .collect::<Vec<_>>()
    };
    let row = |values: &[&str]| values.iter().map(|v| Some((*v).into())).collect::<Vec<_>>();
    table(
        &database,
        "nation",
        &cols(&[("n_nationkey", "UInt64"), ("n_name", "String")]),
        &[row(&["1", "FRANCE"]), row(&["2", "GERMANY"])],
    );
    table(
        &database,
        "supplier",
        &cols(&[("s_suppkey", "UInt64"), ("s_nationkey", "UInt64")]),
        &[row(&["11", "1"]), row(&["22", "2"])],
    );
    table(
        &database,
        "customer",
        &cols(&[("c_custkey", "UInt64"), ("c_nationkey", "UInt64")]),
        &[row(&["101", "1"]), row(&["102", "2"])],
    );
    table(
        &database,
        "orders",
        &cols(&[("o_orderkey", "UInt64"), ("o_custkey", "UInt64")]),
        &[row(&["1", "101"]), row(&["2", "102"])],
    );
    table(
        &database,
        "lineitem",
        &cols(&[
            ("l_suppkey", "UInt64"),
            ("l_orderkey", "UInt64"),
            ("l_shipdate", "Date"),
            ("l_extendedprice", "Float64"),
            ("l_discount", "Float64"),
        ]),
        &[
            row(&["11", "2", "1995-01-01", "100", "0.1"]),
            row(&["22", "1", "1996-12-31", "80", "0.25"]),
        ],
    );
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let query = include_str!("../../../bench/tpch/queries/q07.sql");
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"supp_nation\tcust_nation\tl_year\trevenue\nFRANCE\tGERMANY\t1995\t90\nGERMANY\tFRANCE\t1996\t60\n");
    let limited = handle_request(
        "POST",
        "/?format=tsv-no-header",
        query.as_bytes(),
        &Config {
            max_rows: 1,
            ..config.clone()
        },
    );
    assert_eq!(limited.status, "200 OK", "{:?}", limited.body);
    assert_eq!(limited.body, b"FRANCE\tGERMANY\t1995\t90\n");
    let invalid = query.replace("l_shipdate BETWEEN", "s_suppkey BETWEEN");
    assert_eq!(
        handle_request("POST", "/", invalid.as_bytes(), &config).status,
        "400 Bad Request"
    );
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn sql_http_executes_tpch_q8() {
    let database = std::env::temp_dir().join(format!("rusthouse-http-q8-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    let cols = |values: &[(&str, &str)]| {
        values
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).into(),
                data_type: (*kind).into(),
            })
            .collect::<Vec<_>>()
    };
    let row = |values: &[&str]| values.iter().map(|v| Some((*v).into())).collect::<Vec<_>>();
    table(
        &database,
        "part",
        &cols(&[("p_partkey", "UInt64"), ("p_type", "String")]),
        &[row(&["1", "ECONOMY ANODIZED STEEL"])],
    );
    table(
        &database,
        "region",
        &cols(&[("r_regionkey", "UInt64"), ("r_name", "String")]),
        &[row(&["1", "AMERICA"])],
    );
    table(
        &database,
        "nation",
        &cols(&[
            ("n_nationkey", "UInt64"),
            ("n_regionkey", "UInt64"),
            ("n_name", "String"),
        ]),
        &[row(&["1", "1", "BRAZIL"]), row(&["2", "1", "USA"])],
    );
    table(
        &database,
        "supplier",
        &cols(&[("s_suppkey", "UInt64"), ("s_nationkey", "UInt64")]),
        &[row(&["1", "1"]), row(&["2", "2"])],
    );
    table(
        &database,
        "customer",
        &cols(&[("c_custkey", "UInt64"), ("c_nationkey", "UInt64")]),
        &[row(&["11", "2"])],
    );
    table(
        &database,
        "orders",
        &cols(&[
            ("o_orderkey", "UInt64"),
            ("o_custkey", "UInt64"),
            ("o_orderdate", "Date"),
        ]),
        &[
            row(&["1", "11", "1995-01-01"]),
            row(&["2", "11", "1996-12-31"]),
        ],
    );
    table(
        &database,
        "lineitem",
        &cols(&[
            ("l_partkey", "UInt64"),
            ("l_suppkey", "UInt64"),
            ("l_orderkey", "UInt64"),
            ("l_extendedprice", "Float64"),
            ("l_discount", "Float64"),
        ]),
        &[
            row(&["1", "1", "1", "75", "0"]),
            row(&["1", "2", "1", "25", "0"]),
            row(&["1", "2", "2", "20", "0"]),
        ],
    );
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let query = include_str!("../../../bench/tpch/queries/q08.sql");
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"o_year\tmkt_share\n1995\t0.75\n1996\t0\n");
    let limited = handle_request(
        "POST",
        "/?format=tsv-no-header",
        query.as_bytes(),
        &Config {
            max_rows: 1,
            ..config.clone()
        },
    );
    assert_eq!(limited.status, "200 OK", "{:?}", limited.body);
    assert_eq!(limited.body, b"1995\t0.75\n");
    let invalid = query.replace("p_type =", "p_partkey =");
    assert_eq!(
        handle_request("POST", "/", invalid.as_bytes(), &config).status,
        "400 Bad Request"
    );
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn sql_http_executes_tpch_q2() {
    let database = std::env::temp_dir().join(format!("rusthouse-http-q2-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    let cols = |values: &[(&str, &str)]| {
        values
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).into(),
                data_type: (*kind).into(),
            })
            .collect::<Vec<_>>()
    };
    let row = |values: &[&str]| values.iter().map(|v| Some((*v).into())).collect::<Vec<_>>();
    table(
        &database,
        "part",
        &cols(&[
            ("p_partkey", "UInt64"),
            ("p_size", "UInt64"),
            ("p_type", "String"),
            ("p_mfgr", "String"),
        ]),
        &[row(&["1", "15", "SMALL BRASS", "M1"])],
    );
    table(
        &database,
        "region",
        &cols(&[("r_regionkey", "UInt64"), ("r_name", "String")]),
        &[row(&["1", "EUROPE"])],
    );
    table(
        &database,
        "nation",
        &cols(&[
            ("n_nationkey", "UInt64"),
            ("n_regionkey", "UInt64"),
            ("n_name", "String"),
        ]),
        &[row(&["1", "1", "FRANCE"]), row(&["2", "1", "GERMANY"])],
    );
    table(
        &database,
        "supplier",
        &cols(&[
            ("s_suppkey", "UInt64"),
            ("s_nationkey", "UInt64"),
            ("s_acctbal", "Float64"),
            ("s_name", "String"),
            ("s_address", "String"),
            ("s_phone", "String"),
            ("s_comment", "String"),
        ]),
        &[
            row(&["11", "1", "50", "A", "a", "1", "aa"]),
            row(&["22", "2", "80", "B", "b", "2", "bb"]),
        ],
    );
    table(
        &database,
        "partsupp",
        &cols(&[
            ("ps_partkey", "UInt64"),
            ("ps_suppkey", "UInt64"),
            ("ps_supplycost", "Float64"),
        ]),
        &[row(&["1", "11", "10"]), row(&["1", "22", "10"])],
    );
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let query = include_str!("../../../bench/tpch/queries/q02.sql");
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"s_acctbal\ts_name\tn_name\tp_partkey\tp_mfgr\ts_address\ts_phone\ts_comment\n80\tB\tGERMANY\t1\tM1\tb\t2\tbb\n50\tA\tFRANCE\t1\tM1\ta\t1\taa\n");
    let limited = handle_request(
        "POST",
        "/?format=tsv-no-header",
        query.as_bytes(),
        &Config {
            max_rows: 1,
            ..config.clone()
        },
    );
    assert_eq!(limited.status, "200 OK", "{:?}", limited.body);
    assert_eq!(limited.body, b"80\tB\tGERMANY\t1\tM1\tb\t2\tbb\n");
    let invalid = query.replace("min(ps_supplycost)", "min(missing)");
    assert_eq!(
        handle_request("POST", "/", invalid.as_bytes(), &config).status,
        "400 Bad Request"
    );
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn sql_http_executes_correlated_pair_sum() {
    let database =
        std::env::temp_dir().join(format!("rusthouse-http-pair-sum-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    let cols = |values: &[(&str, &str)]| {
        values
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).into(),
                data_type: (*kind).into(),
            })
            .collect::<Vec<_>>()
    };
    let row = |values: &[&str]| values.iter().map(|v| Some((*v).into())).collect::<Vec<_>>();
    table(
        &database,
        "stock",
        &cols(&[
            ("part", "UInt64"),
            ("supplier", "UInt64"),
            ("available", "Int64"),
        ]),
        &[row(&["1", "1", "11"]), row(&["1", "2", "3"])],
    );
    table(
        &database,
        "parts",
        &cols(&[("id", "UInt64")]),
        &[row(&["1"])],
    );
    table(
        &database,
        "sales",
        &cols(&[
            ("part_key", "UInt64"),
            ("supplier_key", "UInt64"),
            ("quantity", "Float64"),
            ("day", "Date"),
        ]),
        &[
            row(&["1", "1", "20", "1994-01-01"]),
            row(&["1", "2", "4", "1994-01-01"]),
        ],
    );
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let query = "SELECT stock.part, stock.supplier, stock.available FROM stock JOIN parts ON stock.part = parts.id WHERE stock.available > (SELECT 0.5 * SUM(s.quantity) FROM sales s WHERE s.part_key = stock.part AND s.supplier_key = stock.supplier AND s.day >= CAST('1994-01-01' AS date) AND s.day < CAST('1995-01-01' AS date)) ORDER BY stock.part, stock.supplier";
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(
        result.body,
        b"stock.part\tstock.supplier\tstock.available\n1\t1\t11\n1\t2\t3\n"
    );
    let limited = handle_request(
        "POST",
        "/?format=tsv-no-header",
        query.as_bytes(),
        &Config {
            max_rows: 1,
            ..config.clone()
        },
    );
    assert_eq!(limited.status, "200 OK", "{:?}", limited.body);
    assert_eq!(limited.body, b"1\t1\t11\n");
    let invalid = query.replace("SUM(s.quantity)", "SUM(s.missing)");
    assert_eq!(
        handle_request("POST", "/", invalid.as_bytes(), &config).status,
        "400 Bad Request"
    );
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn sql_http_executes_tpch_q20() {
    let database = std::env::temp_dir().join(format!("rusthouse-http-q20-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    let cols = |values: &[(&str, &str)]| {
        values
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).into(),
                data_type: (*kind).into(),
            })
            .collect::<Vec<_>>()
    };
    let row = |values: &[&str]| values.iter().map(|v| Some((*v).into())).collect::<Vec<_>>();
    table(
        &database,
        "supplier",
        &cols(&[
            ("s_suppkey", "UInt64"),
            ("s_nationkey", "UInt64"),
            ("s_name", "String"),
            ("s_address", "String"),
        ]),
        &[row(&["1", "1", "A", "a"]), row(&["2", "1", "B", "b"])],
    );
    table(
        &database,
        "nation",
        &cols(&[("n_nationkey", "UInt64"), ("n_name", "String")]),
        &[row(&["1", "CANADA"])],
    );
    table(
        &database,
        "part",
        &cols(&[("p_partkey", "UInt64"), ("p_name", "String")]),
        &[row(&["1", "forest green"])],
    );
    table(
        &database,
        "partsupp",
        &cols(&[
            ("ps_suppkey", "UInt64"),
            ("ps_partkey", "UInt64"),
            ("ps_availqty", "Int64"),
        ]),
        &[row(&["1", "1", "11"]), row(&["2", "1", "11"])],
    );
    table(
        &database,
        "lineitem",
        &cols(&[
            ("l_partkey", "UInt64"),
            ("l_suppkey", "UInt64"),
            ("l_quantity", "Float64"),
            ("l_shipdate", "Date"),
        ]),
        &[
            row(&["1", "1", "20", "1994-01-01"]),
            row(&["1", "2", "20", "1994-12-31"]),
        ],
    );
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let query = include_str!("../../../bench/tpch/queries/q20.sql");
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"s_name\ts_address\nA\ta\nB\tb\n");
    let limited = handle_request(
        "POST",
        "/?format=tsv-no-header",
        query.as_bytes(),
        &Config {
            max_rows: 1,
            ..config.clone()
        },
    );
    assert_eq!(limited.status, "200 OK", "{:?}", limited.body);
    assert_eq!(limited.body, b"A\ta\n");
    let invalid = query.replace("sum(l_quantity)", "sum(missing)");
    assert_eq!(
        handle_request("POST", "/", invalid.as_bytes(), &config).status,
        "400 Bad Request"
    );
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn sql_http_executes_tpch_q22() {
    let database = std::env::temp_dir().join(format!("rusthouse-http-q22-{}", std::process::id()));
    fs::create_dir_all(&database).unwrap();
    let cols = |values: &[(&str, &str)]| {
        values
            .iter()
            .map(|(name, kind)| Column {
                name: (*name).into(),
                data_type: (*kind).into(),
            })
            .collect::<Vec<_>>()
    };
    let row = |values: &[&str]| {
        values
            .iter()
            .map(|value| Some((*value).into()))
            .collect::<Vec<_>>()
    };
    table(
        &database,
        "customer",
        &cols(&[
            ("c_custkey", "UInt64"),
            ("c_phone", "String"),
            ("c_acctbal", "Float64"),
        ]),
        &[
            row(&["1", "13-a", "100"]),
            row(&["2", "13-b", "300"]),
            row(&["3", "31-a", "500"]),
        ],
    );
    table(
        &database,
        "orders",
        &cols(&[("o_custkey", "UInt64")]),
        &[row(&["2"])],
    );
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };
    let query = include_str!("../../../bench/tpch/queries/q22.sql");
    let result = handle_request("POST", "/", query.as_bytes(), &config);
    assert_eq!(result.status, "200 OK", "{:?}", result.body);
    assert_eq!(result.body, b"cntrycode\tnumcust\ttotacctbal\n31\t1\t500\n");
    let limited = handle_request(
        "POST",
        "/?format=tsv-no-header",
        query.as_bytes(),
        &Config {
            max_rows: 0,
            ..config.clone()
        },
    );
    assert_eq!(limited.status, "200 OK");
    assert!(limited.body.is_empty());
    assert_eq!(
        handle_request("POST", "/explain", query.as_bytes(), &config).status,
        "400 Bad Request"
    );
    fs::remove_dir_all(database).unwrap();
}
