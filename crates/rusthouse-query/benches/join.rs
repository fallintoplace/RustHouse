//! Verified physical hash-join benchmark over immutable wide parts.
//! SQL parsing and fixture creation occur outside the measured region.
use rusthouse_query::{execute_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const PART_ROWS: usize = 4096;
const WARMUPS: usize = 3;

struct Fixture {
    root: PathBuf,
    left: PathBuf,
    right: PathBuf,
    expected: Vec<u8>,
}

impl Fixture {
    fn new(rows: usize, matched: usize) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rusthouse-join-bench-{}-{unique}",
            std::process::id(),
        ));
        let left = root.join("left");
        let right = root.join("right");
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(&right).unwrap();
        let columns = [Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        }];
        for (table, count) in [(&left, rows), (&right, matched)] {
            for (part, start) in (0..count).step_by(PART_ROWS).enumerate() {
                let stop = (start + PART_ROWS).min(count);
                let values = (start..stop)
                    .map(|id| vec![Some(id.to_string())])
                    .collect::<Vec<_>>();
                write_wide_part(
                    &table.join(format!("all_{part}_{part}_0")),
                    &columns,
                    &values,
                )
                .unwrap();
            }
        }
        let mut expected = String::from("l.id\n");
        for id in 0..matched {
            expected.push_str(&format!("{id}\n"));
        }
        Self {
            root,
            left,
            right,
            expected: expected.into_bytes(),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn options() -> (usize, usize, usize) {
    let mut rows = 8192usize;
    let mut match_percent = 50usize;
    let mut samples = 11usize;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bench" => {}
            "--rows" => rows = args.next().expect("--rows needs a value").parse().unwrap(),
            "--match-percent" => {
                match_percent = args
                    .next()
                    .expect("--match-percent needs a value")
                    .parse()
                    .unwrap()
            }
            "--samples" => {
                samples = args.next().expect("--samples needs a value").parse().unwrap()
            }
            "-h" | "--help" => {
                println!("Usage: join [--rows 8192] [--match-percent 50] [--samples 11]");
                std::process::exit(0);
            }
            other => panic!("unknown benchmark argument: {other}"),
        }
    }
    assert!((100..=65_536).contains(&rows), "rows must be 100..65536");
    assert!(
        (1..=100).contains(&match_percent),
        "match-percent must be 1..100"
    );
    assert!(samples > 0 && samples % 2 == 1, "samples must be positive and odd");
    (rows, match_percent, samples)
}

fn main() {
    let (rows, match_percent, samples) = options();
    let matched = rows * match_percent / 100;
    let fixture = Fixture::new(rows, matched);
    let query = parse_join("SELECT l.id FROM left l JOIN right r ON l.id = r.id")
        .unwrap()
        .expect("JOIN statement expected");
    let mut output = Vec::new();
    for _ in 0..WARMUPS {
        output.clear();
        execute_join(black_box(&query), &fixture.left, &fixture.right, &mut output)
            .expect("JOIN warmup failed");
        assert_eq!(output, fixture.expected, "JOIN warmup output mismatch");
    }
    let mut timings = Vec::with_capacity(samples);
    for _ in 0..samples {
        output.clear();
        let start = Instant::now();
        execute_join(black_box(&query), &fixture.left, &fixture.right, &mut output)
            .expect("JOIN execution failed");
        timings.push(start.elapsed());
        black_box(&output);
        assert_eq!(output, fixture.expected, "JOIN output mismatch");
    }
    timings.sort_unstable();
    let median_ms = timings[samples / 2].as_secs_f64() * 1000.0;
    let p90_ms = timings[(samples * 9).div_ceil(10) - 1].as_secs_f64() * 1000.0;
    println!(
        "RustHouse hash JOIN benchmark: {rows} probe rows, {matched} build rows, {samples} samples, {WARMUPS} warmups"
    );
    println!("workload median_ms p90_ms million_probe_rows_per_second");
    println!(
        "inner equi-join {median_ms:.6} {p90_ms:.6} {:.4}",
        rows as f64 / (median_ms / 1000.0) / 1_000_000.0,
    );
    println!("Immutable wide-part scan and JOIN execution included; fixture and parsing excluded.");
}
