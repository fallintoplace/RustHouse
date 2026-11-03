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
