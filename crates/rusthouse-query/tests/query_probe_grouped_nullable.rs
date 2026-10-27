use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn grouped_nullable_aggregates_match_independent_multi_part_totals() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-query-group-probe-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let columns = [
        Column {
            name: "grp".into(),
            data_type: "String".into(),
        },
        Column {
            name: "value".into(),
            data_type: "Nullable(Int64)".into(),
        },
    ];
    let batches: [[(&str, Option<i64>); 4]; 2] = [
        [
            ("alpha", Some(10)),
            ("beta", Some(0)),
            ("alpha", None),
            ("gamma", Some(-2)),
        ],
        [
            ("beta", Some(30)),
            ("alpha", Some(20)),
            ("gamma", None),
            ("beta", None),
        ],
    ];
    for (index, batch) in batches.iter().enumerate() {
        let rows = batch
            .iter()
            .map(|(grp, value)| {
                vec![
                    Some((*grp).to_owned()),
                    value.map(|number| number.to_string()),
                ]
            })
            .collect::<Vec<_>>();
        let part = index + 1;
        write_wide_part(
            &root.join(format!("all_{part}_{part}_0")),
            &columns,
            &rows,
        )
        .unwrap();
    }
    for (sql, expected) in [
        (
            "SELECT grp, count(*) FROM part GROUP BY grp ORDER BY grp",
            "grp\tcount\nalpha\t3\nbeta\t3\ngamma\t2\n",
        ),
        (
            "SELECT grp, sum(value), min(value) FROM part GROUP BY grp ORDER BY grp DESC",
            "grp\tsum(value)\tmin(value)\ngamma\t-2\t-2\nbeta\t30\t0\nalpha\t30\t10\n",
        ),
        (
            "SELECT grp, count(*) FROM part WHERE value IS NOT NULL GROUP BY grp HAVING count(*) >= 2 ORDER BY grp DESC",
            "grp\tcount\nbeta\t2\nalpha\t2\n",
        ),
        (
            "SELECT grp, avg(value) FROM part GROUP BY grp ORDER BY grp LIMIT 2 OFFSET 1",
            "grp\tavg(value)\nbeta\t15\ngamma\t-2\n",
        ),
    ] {
        let parsed = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
        let mut output = Vec::new();
        execute(&parsed, Some(&root), &mut output)
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{sql}");
    }
    fs::remove_dir_all(root).unwrap();
}
