use rusthouse_query::{ParseError, execute_correlated_min, parse_join_correlated_min_with_schema};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-correlated-min-prefilter-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();

    let part = root.join("part");
    let partsupp = root.join("partsupp");
    let supplier = root.join("supplier");
    let nation = root.join("nation");
    let region = root.join("region");
    write_wide_part(
        &part,
        &[Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        }],
        &[
            vec![Some("1".into())],
            vec![Some("2".into())],
            vec![Some("3".into())],
            vec![Some("4".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &partsupp,
        &[
            Column {
                name: "partkey".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "supplier_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "cost".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("10".into()), Some("5".into())],
            vec![Some("1".into()), Some("11".into()), Some("5".into())],
            vec![Some("1".into()), Some("12".into()), Some("7".into())],
            vec![Some("2".into()), Some("10".into()), Some("8".into())],
            vec![Some("2".into()), Some("11".into()), Some("2".into())],
            vec![Some("3".into()), Some("10".into()), Some("3".into())],
            vec![Some("3".into()), Some("12".into()), Some("3".into())],
            vec![Some("4".into()), Some("11".into()), Some("1".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &supplier,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "nationkey".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "name".into(),
                data_type: "String".into(),
            },
            Column {
                name: "score".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![
                Some("10".into()),
                Some("100".into()),
                Some("Euro A".into()),
                Some("5".into()),
            ],
            vec![
                Some("11".into()),
                Some("200".into()),
                Some("Asia".into()),
                Some("1".into()),
            ],
            vec![
                Some("12".into()),
                Some("101".into()),
                Some("Euro C".into()),
                Some("7".into()),
            ],
        ],
    )
    .unwrap();
    write_wide_part(
        &nation,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "regionkey".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("100".into()), Some("1".into())],
            vec![Some("101".into()), Some("1".into())],
            vec![Some("200".into()), Some("2".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &region,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "name".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("EUROPE".into())],
            vec![Some("2".into()), Some("ASIA".into())],
        ],
    )
    .unwrap();

    (root, vec![part, partsupp, supplier, nation, region])
}

fn schema(table: &str) -> Result<Vec<String>, ParseError> {
    Ok(match table {
        "part" => vec!["id"],
        "partsupp" => vec!["partkey", "supplier_id", "cost"],
        "supplier" => vec!["id", "nationkey", "name", "score"],
        "nation" => vec!["id", "regionkey"],
        "region" => vec!["id", "name"],
        _ => return Err(ParseError(format!("unknown fixture table: {table}"))),
    }
    .into_iter()
    .map(str::to_owned)
    .collect())
}

fn run(
    sql: &str,
    outer_paths: &[std::path::PathBuf],
    inner_paths: &[std::path::PathBuf],
) -> String {
    let plan = parse_join_correlated_min_with_schema(sql, schema)
        .unwrap()
        .unwrap();
    let mut output = Vec::new();
    execute_correlated_min(&plan, outer_paths, inner_paths, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

fn rows(output: &str) -> Vec<&str> {
    output.lines().skip(1).collect()
}

#[test]
fn correlated_min_filters_after_the_key_and_value_relations_join() {
    let (root, paths) = fixture();
    let sql = "SELECT p.id, s.id, ps.cost FROM part p, supplier s, partsupp ps, nation n, region r WHERE p.id = ps.partkey AND s.id = ps.supplier_id AND s.nationkey = n.id AND n.regionkey = r.id AND r.name = 'EUROPE' AND ps.cost = (SELECT MIN(ps.cost) FROM partsupp ps, supplier s, nation n, region r WHERE p.id = ps.partkey AND s.id = ps.supplier_id AND s.nationkey = n.id AND n.regionkey = r.id AND r.name = 'EUROPE') ORDER BY p.id, s.id";
    let output = run(
        sql,
        &[
            paths[0].clone(),
            paths[1].clone(),
            paths[2].clone(),
            paths[3].clone(),
            paths[4].clone(),
        ],
        &[
            paths[1].clone(),
            paths[2].clone(),
            paths[3].clone(),
            paths[4].clone(),
        ],
    );

    assert_eq!(
        rows(&output),
        ["1\t10\t5", "2\t10\t8", "3\t10\t3", "3\t12\t3"]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn correlated_min_waits_until_a_later_value_relation_is_joined() {
    let (root, paths) = fixture();
    let sql = "SELECT p.id, s.id, s.score FROM part p, supplier s, partsupp ps, nation n, region r WHERE p.id = ps.partkey AND s.id = ps.supplier_id AND s.nationkey = n.id AND n.regionkey = r.id AND r.name = 'EUROPE' AND s.score = (SELECT MIN(s.score) FROM partsupp ps, supplier s, nation n, region r WHERE p.id = ps.partkey AND s.id = ps.supplier_id AND s.nationkey = n.id AND n.regionkey = r.id AND r.name = 'EUROPE') ORDER BY p.id, s.id";
    let output = run(
        sql,
        &[
            paths[0].clone(),
            paths[1].clone(),
            paths[2].clone(),
            paths[3].clone(),
            paths[4].clone(),
        ],
        &[
            paths[1].clone(),
            paths[2].clone(),
            paths[3].clone(),
            paths[4].clone(),
        ],
    );

    assert_eq!(rows(&output), ["1\t10\t5", "2\t10\t5", "3\t10\t5"]);
    fs::remove_dir_all(root).unwrap();
}
