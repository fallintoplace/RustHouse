//! Reproducible on-disk scan benchmark using RustHouse-written MergeTree wide parts.
//! Part creation and SQL parsing are intentionally outside the measurement.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const PART_ROWS: usize = 4096;
const DEFAULT_ROWS: usize = 65_536;
const DEFAULT_SAMPLES: usize = 11;

struct Fixture(PathBuf);

impl Fixture {
    fn new(rows: usize) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rusthouse-part-bench-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        let columns = vec![Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        }];
        for (part, first) in (0..rows).step_by(PART_ROWS).enumerate() {
            let end = (first + PART_ROWS).min(rows);
            let values = (first..end)
                .map(|value| vec![Some(value.to_string())])
                .collect::<Vec<_>>();
            let path = fixture.0.join(format!("all_{part}_{part}_0"));
            write_wide_part(&path, &columns, &values).unwrap();
        }
        fixture
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn options() -> (usize, usize) {
    let mut rows = DEFAULT_ROWS;
    let mut samples = DEFAULT_SAMPLES;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bench" => {}
            "--rows" => {
                rows = args
                    .next()
                    .expect("--rows requires a value")
                    .parse()
                    .expect("--rows must be an integer");
            }
            "--samples" => {
                samples = args
                    .next()
                    .expect("--samples requires a value")
                    .parse()
                    .expect("--samples must be an integer");
            }
            "-h" | "--help" => {
                println!(
                    "Usage: cargo bench -p rusthouse-query --bench parts -- [--rows N] [--samples N]"
                );
                std::process::exit(0);
            }
            other => panic!("unknown option: {other}"),
        }
    }
    assert!(
        (1..=1_048_576).contains(&rows),
        "rows must be between 1 and 1048576"
    );
    assert!(
        samples > 0 && samples % 2 == 1,
        "samples must be positive and odd"
    );
    (rows, samples)
}

fn measure(label: &str, sql: &str, expected: &str, fixture: &Fixture, rows: usize, samples: usize) {
    let query = parse_select(sql).expect("invalid benchmark SQL");
    let mut output = Vec::new();
    for _ in 0..3 {
        output.clear();
        execute(black_box(&query), Some(&fixture.0), &mut output).unwrap();
    }
    assert_eq!(
        output,
        expected.as_bytes(),
        "{label} warmup returned bad data"
    );
    let mut timings = Vec::with_capacity(samples);
    for _ in 0..samples {
        output.clear();
        let start = Instant::now();
        execute(black_box(&query), Some(&fixture.0), &mut output).unwrap();
        timings.push(start.elapsed());
        black_box(&output);
        assert_eq!(output, expected.as_bytes(), "{label} returned bad data");
    }
    timings.sort_unstable();
    let seconds = timings[samples / 2].as_secs_f64();
    let p90_seconds = timings[samples.saturating_mul(9).div_ceil(10) - 1].as_secs_f64();
    println!(
        "{label:<18} {:>10.3} {:>10.3} {:>17.2}",
        seconds * 1000.0,
        p90_seconds * 1000.0,
        rows as f64 / seconds / 1_000_000.0
    );
}

fn main() {
    let (rows, samples) = options();
    let fixture = Fixture::new(rows);
    let middle = rows / 2;
    let retained = rows - middle;
    let sum = (middle as u128 + rows as u128 - 1) * retained as u128 / 2;

    let mut page = String::from("id\n");
    for id in (middle..rows).take(100) {
        writeln!(&mut page, "{id}").unwrap();
    }
    let mut top = String::from("id\n");
    for id in (middle..rows).rev().take(100) {
        writeln!(&mut top, "{id}").unwrap();
    }
    println!("RustHouse frozen-parts benchmark: {rows} rows, {samples} samples");
    println!(
        "{:<18} {:>10} {:>10} {:>17}",
        "workload", "median ms", "p90 ms", "M logical rows/s"
    );
    println!(
        "Logical rows/s uses input rows, not verified bytes read or cold-cache throughput."
    );
    for (label, fraction) in [
        ("count 100%", 100usize),
        ("count 50%", 50),
        ("count 10%", 10),
        ("count 1%", 1),
    ] {
        let selected = rows.saturating_mul(fraction).div_ceil(100).min(rows);
        let cutoff = rows - selected;
        measure(
            label,
            &format!("SELECT count(*) FROM part WHERE id >= {cutoff}"),
            &format!("count\n{selected}\n"),
            &fixture,
            rows,
            samples,
        );
    }
    measure(
        "filtered sum",
        &format!("SELECT sum(id) FROM part WHERE id >= {middle}"),
        &format!("sum(id)\n{sum}\n"),
        &fixture,
        rows,
        samples,
    );
    measure(
        "large OFFSET",
        &format!("SELECT id FROM part LIMIT 100 OFFSET {middle}"),
        &page,
        &fixture,
        rows,
        samples,
    );
    measure(
        "filtered top-K",
        &format!("SELECT id FROM part WHERE id >= {middle} ORDER BY id DESC LIMIT 100"),
        &top,
        &fixture,
        rows,
        samples,
    );
}
