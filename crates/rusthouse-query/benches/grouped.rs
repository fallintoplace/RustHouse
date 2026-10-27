//! Deterministic String-key and Float64 grouped scan benchmark.
//! Fixture generation and SQL parsing are not included in query timings.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const PART_ROWS: usize = 32_768;
const DEFAULT_ROWS: usize = 65_536;
const DEFAULT_STATIONS: usize = 413;
const DEFAULT_SAMPLES: usize = 11;
const WARMUPS: usize = 3;

#[derive(Clone, Copy)]
struct Stats {
    min: i64,
    max: i64,
    sum: i128,
    count: u64,
}

impl Stats {
    fn new() -> Self {
        Self { min: i64::MAX, max: i64::MIN, sum: 0, count: 0 }
    }

    fn add(&mut self, tenths: i64) {
        self.min = self.min.min(tenths);
        self.max = self.max.max(tenths);
        self.sum += i128::from(tenths);
        self.count += 1;
    }

    fn expected(&self, function: &str) -> f64 {
        match function {
            "min" => self.min as f64 / 10.0,
            "max" => self.max as f64 / 10.0,
            "avg" => self.sum as f64 / self.count as f64 / 10.0,
            _ => unreachable!(),
        }
    }
}

struct Fixture {
    root: PathBuf,
    stats: Vec<Stats>,
}

impl Fixture {
    fn new(rows: usize, stations: usize) -> Self {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rusthouse-grouped-bench-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let columns = [
            Column { name: "station".into(), data_type: "String".into() },
            Column { name: "temperature".into(), data_type: "Float64".into() },
        ];
        let mut stats = vec![Stats::new(); stations];
        for (index, first) in (0..rows).step_by(PART_ROWS).enumerate() {
            let mut values = Vec::with_capacity((rows - first).min(PART_ROWS));
            for row in first..(first + PART_ROWS).min(rows) {
                let station = row % stations;
                let tenths =
                    (row.wrapping_mul(2_654_435_761).wrapping_add(17) % 801) as i64 - 400;
                stats[station].add(tenths);
                values.push(vec![
                    Some(format!("station-{station:05}")),
                    Some((tenths as f64 / 10.0).to_string()),
                ]);
            }
            let part = index + 1;
            write_wide_part(
                &root.join(format!("all_{part}_{part}_0")),
                &columns,
                &values,
            ).unwrap();
        }
        Self { root, stats }
    }

    fn check(&self, output: &[u8], function: &str) {
        let rendered = std::str::from_utf8(output).unwrap();
        let mut lines = rendered.lines();
        assert_eq!(
            lines.next(),
            Some(format!("station\t{function}(temperature)").as_str())
        );
        for (station, expected) in self.stats.iter().enumerate() {
            let line = lines.next().expect("missing group result");
            let (key, value) = line.split_once('\t').expect("missing aggregate column");
            assert_eq!(key, format!("station-{station:05}"));
            let actual = value.parse::<f64>().expect("invalid numeric aggregate");
            let wanted = expected.expected(function);
            assert!(
                (actual - wanted).abs() < 1e-7,
                "{function} for {key}: expected {wanted}, got {actual}"
            );
        }
        assert!(lines.next().is_none(), "unexpected extra group");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn options() -> (usize, usize, usize) {
    let mut rows = DEFAULT_ROWS;
    let mut stations = DEFAULT_STATIONS;
    let mut samples = DEFAULT_SAMPLES;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bench" => {}
            "--rows" => rows = args.next().expect("--rows needs a value").parse().unwrap(),
            "--stations" => stations = args.next().expect("--stations needs a value").parse().unwrap(),
            "--samples" => samples = args.next().expect("--samples needs a value").parse().unwrap(),
            "-h" | "--help" => {
                println!("Usage: grouped [--rows N] [--stations N] [--samples odd-N]");
                std::process::exit(0);
            }
            _ => panic!("unexpected benchmark argument: {arg}"),
        }
    }
    assert!((1..=1_048_576).contains(&rows));
    assert!((1..=100_000).contains(&stations) && stations <= rows);
    assert!(samples > 0 && samples % 2 == 1);
    (rows, stations, samples)
}

fn measure(fixture: &Fixture, rows: usize, samples: usize, function: &str) {
    let sql = format!(
        "SELECT station, {function}(temperature) FROM part GROUP BY station ORDER BY station"
    );
    let query = parse_select(&sql).unwrap();
    let mut output = Vec::new();
    for _ in 0..WARMUPS {
        output.clear();
        execute(black_box(&query), Some(&fixture.root), &mut output).unwrap();
        fixture.check(&output, function);
    }
    let mut timings = Vec::with_capacity(samples);
    for _ in 0..samples {
        output.clear();
        let start = Instant::now();
        execute(black_box(&query), Some(&fixture.root), &mut output).unwrap();
        timings.push(start.elapsed().as_secs_f64());
        black_box(&output);
        fixture.check(&output, function);
    }
    timings.sort_by(f64::total_cmp);
    let median = timings[samples / 2];
    let p90 = timings[(samples * 9).div_ceil(10) - 1];
    println!(
        "{function:<8} {:>12.3} {:>12.3} {:>16.2}",
        median * 1000.0,
        p90 * 1000.0,
        rows as f64 / median / 1_000_000.0
    );
}

fn main() {
    let (rows, stations, samples) = options();
    let fixture = Fixture::new(rows, stations);
    println!("RustHouse grouped scans: {rows} rows, {stations} keys, {samples} samples");
    println!("{:<8} {:>12} {:>12} {:>16}", "aggregate", "median ms", "p90 ms", "M rows/s");
    for function in ["min", "avg", "max"] {
        measure(&fixture, rows, samples, function);
    }
}
