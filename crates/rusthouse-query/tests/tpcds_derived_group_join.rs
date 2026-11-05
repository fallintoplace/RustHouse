use rusthouse_query::{
    DerivedColumn, execute_derived_grouped_join, parse_derived_grouped_join_with_schema,
};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn joins_customers_to_a_grouped_count_derived_source() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-derived-group-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let sales = root.join("store_sales");
    let dates = root.join("date_dim");
    let customers = root.join("customer");
    write_wide_part(
        &sales,
        &[
            Column {
                name: "ss_ticket_number".into(),
                data_type: "Int64".into(),
            },
            Column {
                name: "ss_customer_sk".into(),
                data_type: "Int64".into(),
            },
            Column {
                name: "ss_sold_date_sk".into(),
                data_type: "Int64".into(),
            },
        ],
        &[
            vec![Some("100".into()), Some("1".into()), Some("1".into())],
            vec![Some("100".into()), Some("1".into()), Some("1".into())],
            vec![Some("101".into()), Some("1".into()), Some("2".into())],
            vec![Some("200".into()), Some("2".into()), Some("1".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &dates,
        &[Column {
            name: "d_date_sk".into(),
            data_type: "Int64".into(),
        }],
        &[vec![Some("1".into())], vec![Some("2".into())]],
    )
    .unwrap();
    write_wide_part(
        &customers,
        &[
            Column {
                name: "c_customer_sk".into(),
                data_type: "Int64".into(),
            },
            Column {
                name: "c_customer_id".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("A".into())],
            vec![Some("2".into()), Some("B".into())],
        ],
    )
    .unwrap();

    let sql = "SELECT c_customer_id, ss_ticket_number, cnt FROM (SELECT ss_ticket_number, ss_customer_sk, COUNT(*) AS cnt FROM store_sales, date_dim WHERE ss_sold_date_sk = d_date_sk AND (ss_sold_date_sk = 1 OR ss_sold_date_sk = 2) AND ss_sold_date_sk IN (1, 1 + 1) AND CASE WHEN store_sales.ss_sold_date_sk > 0 THEN (store_sales.ss_sold_date_sk * 1.000) / store_sales.ss_sold_date_sk ELSE NULL END > 0.5 GROUP BY ss_ticket_number, ss_customer_sk) dn, customer WHERE ss_customer_sk = c_customer_sk AND cnt BETWEEN 1 AND 2 ORDER BY c_customer_id, ss_ticket_number LIMIT 10";
    let schema = |table: &str| -> Result<Vec<DerivedColumn>, rusthouse_query::ParseError> {
        let columns = match table {
            "store_sales" => vec![
                ("ss_ticket_number", "Int64"),
                ("ss_customer_sk", "Int64"),
                ("ss_sold_date_sk", "Int64"),
            ],
            "date_dim" => vec![("d_date_sk", "Int64")],
            "customer" => vec![("c_customer_sk", "Int64"), ("c_customer_id", "String")],
            _ => return Err(rusthouse_query::ParseError("unknown table".into())),
        };
        Ok(columns
            .into_iter()
            .map(|(name, data_type)| DerivedColumn {
                name: name.into(),
                data_type: data_type.into(),
            })
            .collect())
    };
    let plan = parse_derived_grouped_join_with_schema(sql, schema)
        .unwrap()
        .unwrap();
    let inner = plan.inner();
    let inner_paths = [&inner.left, &inner.right]
        .into_iter()
        .chain(inner.steps.iter().map(|step| &step.table))
        .map(|table| match table.as_str() {
            "store_sales" => sales.clone(),
            "date_dim" => dates.clone(),
            _ => panic!("unexpected inner table {table}"),
        })
        .collect::<Vec<PathBuf>>();
    let outer = plan.outer();
    let outer_paths = [&outer.left, &outer.right]
        .into_iter()
        .chain(outer.steps.iter().map(|step| &step.table))
        .map(|table| match table.as_str() {
            "customer" => customers.clone(),
            _ if table == plan.table() => PathBuf::new(),
            _ => panic!("unexpected outer table {table}"),
        })
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    execute_derived_grouped_join(&plan, &inner_paths, &outer_paths, &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "c_customer_id\tss_ticket_number\tcnt\nA\t100\t2\nA\t101\t1\nB\t200\t1\n"
    );

    fs::remove_dir_all(root).unwrap();
}
