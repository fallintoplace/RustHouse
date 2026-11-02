//! Compare exact analytical query latency at different worker limits.
//! Run separately with RUSTHOUSE_QUERY_THREADS=1 and =4.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const PARTS: usize = 40;
const ROWS_PER_PART: usize = 512;
const SAMPLES: usize = 9;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rusthouse-parallel-query-bench-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let columns = [
            Column {
                name: "category".into(),
                data_type: "UInt8".into(),
            },
            Column {
                name: "amount".into(),
                data_type: "Int64".into(),
            },
            Column {
                name: "flag".into(),
                data_type: "UInt8".into(),
            },
        ];
        for part in 0..PARTS {
            let rows = (0..ROWS_PER_PART)
                .map(|index| {
                    vec![
                        Some((index % 4).to_string()),
                        Some(index.to_string()),
                        Some((index % 2).to_string()),
                    ]
                })
                .collect::<Vec<_>>();
            write_wide_part(&root.join(format!("all_{part}_{part}_0")), &columns, &rows)
                .unwrap();
        }
        Self(root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn measure(fixture: &Fixture, name: &str, sql: &str, expected_body: &str) {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    for _ in 0..3 {
        output.clear();
        execute(black_box(&query), Some(&fixture.0), &mut output).unwrap();
    }
    assert_eq!(
        std::str::from_utf8(&output).unwrap().lines().skip(1).collect::<Vec<_>>().join("\\n"),
        expected_body,
        "{name} returned an unexpected result"
    );
    let mut times = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        output.clear();
        let start = Instant::now();
        execute(black_box(&query), Some(&fixture.0), &mut output).unwrap();
        times.push(start.elapsed().as_secs_f64());
        black_box(&output);
    }
    times.sort_by(f64::total_cmp);
    println!("{name:<20} {:>10.3}", times[SAMPLES / 2] * 1000.0);
}

fn main() {
    let fixture = Fixture::new();
    println!("Parallel query benchmark: {PARTS} parts, {ROWS_PER_PART} rows/part");
    println!("RUSTHOUSE_QUERY_THREADS={}", std::env::var("RUSTHOUSE_QUERY_THREADS").unwrap_or_else(|_| "default".into()));
    println!("{:<20} {:>10}", "query", "median ms");
    measure(&fixture, "filtered count", "SELECT count(*) FROM part WHERE flag = 1", "10240");
    measure(&fixture, "filtered sum", "SELECT sum(amount) FROM part WHERE flag = 1", "2611200");
    measure(&fixture, "exact distinct", "SELECT count(DISTINCT category) FROM part", "4");
    measure(&fixture, "grouped count", "SELECT category, count(*) FROM part GROUP BY category ORDER BY category", "0\\t5120\\n1\\t5120\\n2\\t5120\\n3\\t5120");
}
