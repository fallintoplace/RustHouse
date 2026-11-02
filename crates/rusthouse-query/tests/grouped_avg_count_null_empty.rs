//! Regression coverage for mixed grouped AVG, COUNT(*) and COUNT(column).
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct TestPart(PathBuf);

impl Drop for TestPart {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn part(rows: &[(&str, &str, Option<&str>)]) -> TestPart {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-grouped-avg-count-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column {
            name: "region".into(),
            data_type: "String".into(),
        },
        Column {
            name: "passenger_count".into(),
            data_type: "UInt8".into(),
        },
        Column {
            name: "distance".into(),
            data_type: "Nullable(Float64)".into(),
        },
    ];
    let values = rows
        .iter()
        .map(|(region, passengers, distance)| {
            vec![
                Some((*region).to_owned()),
                Some((*passengers).to_owned()),
                distance.map(str::to_owned),
            ]
        })
        .collect::<Vec<_>>();
    write_wide_part(&path, &columns, &values).unwrap();
    TestPart(path)
}

fn query(part: &TestPart, sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(&part.0), &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn grouped_avg_and_counts_keep_row_and_non_null_counts_separate() {
    let fixture = part(&[
        ("east", "1", Some("2")),
        ("east", "1", None),
        ("east", "1", Some("4")),
        ("east", "2", None),
        ("west", "1", Some("8")),
        ("west", "2", None),
        ("west", "2", None),
        ("west", "2", Some("6")),
    ]);

    assert_eq!(
        query(
            &fixture,
            "SELECT region, AVG(distance), COUNT(*), COUNT(distance) FROM part GROUP BY region ORDER BY region"
        ),
        "region\tavg(distance)\tcount(*)\tcount(distance)\neast\t3\t4\t2\nwest\t7\t4\t2\n"
    );
    assert_eq!(
        query(
            &fixture,
            "SELECT region, COUNT(*) AS trips, AVG(distance) AS mean FROM part GROUP BY region ORDER BY region"
        ),
        "region\ttrips\tmean\neast\t4\t3\nwest\t4\t7\n"
    );
    assert_eq!(
        query(
            &fixture,
            "SELECT region, AVG(distance), COUNT() FROM part GROUP BY region ORDER BY region"
        ),
        "region\tavg(distance)\tcount(*)\neast\t3\t4\nwest\t7\t4\n"
    );
    assert_eq!(
        query(
            &fixture,
            "SELECT region, AVG(distance), COUNT(*) FROM part GROUP BY region HAVING COUNT(*) >= 4 ORDER BY COUNT(*) DESC, region"
        ),
        "region\tavg(distance)\tcount(*)\neast\t3\t4\nwest\t7\t4\n"
    );
    // The single COUNT(*) projection still uses its original grouped path.
    assert_eq!(
        query(
            &fixture,
            "SELECT region, COUNT(*) FROM part GROUP BY region ORDER BY region"
        ),
        "region\tcount\neast\t4\nwest\t4\n"
    );
    assert!(parse_select(
        "SELECT region, AVG(distance), COUNT(1) FROM part GROUP BY region"
    )
    .is_err());
}

#[test]
fn composite_grouped_avg_counts_all_null_groups() {
    let fixture = part(&[
        ("east", "1", Some("2")),
        ("east", "1", None),
        ("east", "1", Some("4")),
        ("east", "2", None),
        ("west", "1", Some("8")),
        ("west", "2", None),
        ("west", "2", None),
        ("west", "2", Some("6")),
    ]);
    assert_eq!(
        query(
            &fixture,
            "SELECT region, passenger_count, AVG(distance) AS mean, COUNT(*) AS trips, COUNT(distance) AS measured FROM part GROUP BY region, passenger_count ORDER BY region, passenger_count"
        ),
        "region\tpassenger_count\tmean\ttrips\tmeasured\neast\t1\t3\t3\t2\neast\t2\tNULL\t1\t0\nwest\t1\t8\t1\t1\nwest\t2\t6\t3\t1\n"
    );
    assert_eq!(
        query(
            &fixture,
            "SELECT region, passenger_count, AVG(distance), COUNT(*) FROM part GROUP BY region, passenger_count HAVING COUNT(*) > 1 ORDER BY region, passenger_count"
        ),
        "region\tpassenger_count\tavg(distance)\tcount(*)\neast\t1\t3\t3\nwest\t2\t6\t3\n"
    );
}

#[test]
fn empty_physical_and_filtered_inputs_have_no_group_rows() {
    let empty = part(&[]);
    let nonempty = part(&[("east", "2", None)]);
    let sql = "SELECT region, AVG(distance), COUNT(*), COUNT(distance) FROM part GROUP BY region ORDER BY region";
    let header = "region\tavg(distance)\tcount(*)\tcount(distance)\n";
    assert_eq!(query(&empty, sql), header);
    assert_eq!(
        query(
            &nonempty,
            "SELECT region, AVG(distance), COUNT(*), COUNT(distance) FROM part WHERE region = 'missing' GROUP BY region ORDER BY region"
        ),
        header
    );
    assert_eq!(
        query(&nonempty, sql),
        "region\tavg(distance)\tcount(*)\tcount(distance)\neast\tNULL\t1\t0\n"
    );
}
