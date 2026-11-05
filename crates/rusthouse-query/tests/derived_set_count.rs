use rusthouse_query::{
    DerivedColumn, execute_derived_set_count, parse_derived_set_count_with_schema,
};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn write_table(root: &Path, name: &str, rows: &[(&str, &str)]) -> PathBuf {
    let path = root.join(name);
    let schema = [
        Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "value".into(),
            data_type: "String".into(),
        },
    ];
    let rows = rows
        .iter()
        .map(|(id, value)| vec![Some((*id).into()), Some((*value).into())])
        .collect::<Vec<_>>();
    write_wide_part(&path, &schema, &rows).unwrap();
    path
}

fn schema(_table: &str) -> Result<Vec<DerivedColumn>, rusthouse_query::ParseError> {
    Ok(vec![
        DerivedColumn {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        DerivedColumn {
            name: "value".into(),
            data_type: "String".into(),
        },
    ])
}

fn execute_count(root: &Path, sql: &str) -> String {
    let plan = parse_derived_set_count_with_schema(sql, schema)
        .unwrap()
        .unwrap();
    let paths = plan
        .branches()
        .iter()
        .map(|branch| {
            [&branch.left, &branch.right]
                .into_iter()
                .chain(branch.steps.iter().map(|step| &step.table))
                .map(|table| root.join(table))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    execute_derived_set_count(&plan, &paths, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn derived_set_count_handles_distinct_intersect_and_left_associative_except() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-derived-set-count-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    write_table(
        &root,
        "left_a",
        &[("1", "alpha"), ("1", "alpha"), ("2", "beta")],
    );
    write_table(&root, "left_b", &[("1", "x"), ("2", "x")]);
    write_table(&root, "middle_a", &[("2", "beta")]);
    write_table(&root, "middle_b", &[("2", "x")]);
    write_table(
        &root,
        "right_a",
        &[("1", "alpha"), ("3", "gamma"), ("2", "beta")],
    );
    write_table(&root, "right_b", &[("1", "x"), ("2", "x"), ("3", "x")]);
    write_table(&root, "last_a", &[("1", "alpha")]);
    write_table(&root, "last_b", &[("1", "x")]);

    let intersect = "SELECT COUNT(*) FROM ((SELECT l.id, l.value FROM left_a l JOIN left_b lb ON l.id = lb.id) INTERSECT (SELECT r.id, r.value FROM right_a r JOIN right_b rb ON r.id = rb.id) INTERSECT (SELECT m.id, m.value FROM middle_a m JOIN middle_b mb ON m.id = mb.id)) sets LIMIT 100";
    assert_eq!(execute_count(&root, intersect), "count_star()\n1\n");

    // SQL evaluates chained EXCEPT from left to right: (A EXCEPT B) EXCEPT C.
    let except = "SELECT COUNT(*) AS remaining FROM ((SELECT l.id, l.value FROM left_a l JOIN left_b lb ON l.id = lb.id) EXCEPT (SELECT m.id, m.value FROM middle_a m JOIN middle_b mb ON m.id = mb.id) EXCEPT (SELECT z.id, z.value FROM last_a z JOIN last_b zb ON z.id = zb.id)) sets";
    assert_eq!(execute_count(&root, except), "remaining\n0\n");

    let limit_zero = "SELECT COUNT(*) AS total FROM ((SELECT l.id, l.value FROM left_a l JOIN left_b lb ON l.id = lb.id) INTERSECT (SELECT r.id, r.value FROM right_a r JOIN right_b rb ON r.id = rb.id)) sets LIMIT 0";
    assert_eq!(execute_count(&root, limit_zero), "total\n");

    fs::remove_dir_all(root).unwrap();
}
