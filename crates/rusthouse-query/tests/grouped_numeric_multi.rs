use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn run(sql: &str, path: &std::path::Path) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, Some(path), &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn groups_several_numeric_aggregates_in_one_scan() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-grouped-numeric-multi-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let columns = vec![
        Column {
            name: "station".into(),
            data_type: "String".into(),
        },
        Column {
            name: "temperature".into(),
            data_type: "Float64".into(),
        },
    ];
    let left = [("Berlin", "10"), ("Bremen", "-4"), ("Berlin", "20")];
    let right = [("Bremen", "2"), ("Berlin", "30"), ("Cologne", "1")];
    for (index, rows) in [left.as_slice(), right.as_slice()].iter().enumerate() {
        let records = rows
            .iter()
            .map(|(station, temp)| vec![Some((*station).into()), Some((*temp).into())])
            .collect::<Vec<_>>();
        let name = index + 1;
        write_wide_part(
            &root.join(format!("all_{name}_{name}_0")),
            &columns,
            &records,
        )
        .unwrap();
    }
    let query = "SELECT station, min(temperature), avg(temperature), max(temperature) FROM part GROUP BY station ORDER BY station";
    assert_eq!(
        run(query, &root),
        "station\tmin(temperature)\tavg(temperature)\tmax(temperature)\nBerlin\t10\t20\t30\nBremen\t-4\t-1\t2\nCologne\t1\t1\t1\n"
    );
    assert_eq!(
        run(
            "SELECT station, min(temperature), max(temperature) FROM part GROUP BY station ORDER BY station DESC LIMIT 1",
            &root
        ),
        "station\tmin(temperature)\tmax(temperature)\nCologne\t1\t1\n"
    );
    assert_eq!(
        run(
            "SELECT station, min(temperature), avg(temperature) FROM part GROUP BY station LIMIT 0",
            &root
        ),
        "station\tmin(temperature)\tavg(temperature)\n"
    );
    assert!(
        parse_select(
            "SELECT station, min(temperature), max(temperature) FROM part GROUP BY temperature"
        )
        .is_err()
    );
    assert!(parse_select("SELECT station, min(temperature), max(temperature) FROM part").is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn grouped_mixed_count_and_sum_keep_nullable_numeric_semantics() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-grouped-nullable-multi-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    let columns = [
        Column { name: "station".into(), data_type: "String".into() },
        Column { name: "temperature".into(), data_type: "Nullable(UInt64)".into() },
    ];
    let rows = [
        vec![Some("west".into()), Some("3".into())],
        vec![Some("east".into()), None],
        vec![Some("west".into()), Some("5".into())],
        vec![Some("east".into()), Some("7".into())],
    ];
    write_wide_part(&root, &columns, &rows).unwrap();
    assert_eq!(
        run("SELECT station, sum(temperature), count(temperature) FROM part GROUP BY station ORDER BY station", &root),
        "station\tsum(temperature)\tcount(temperature)\neast\t7\t1\nwest\t8\t2\n"
    );
    fs::remove_dir_all(root).unwrap();
}
