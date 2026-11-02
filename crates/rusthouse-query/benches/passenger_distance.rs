//! Deterministic passenger-distance grouped AVG and COUNT benchmark.
//! Fixture generation, SQL parsing and result validation are outside timings.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const PART_ROWS: usize = 32_768;
const DEFAULT_ROWS: usize = 262_144;
const DEFAULT_GROUPS: usize = 4;
const DEFAULT_SAMPLES: usize = 11;
const WARMUPS: usize = 3;

const AVG_SQL: &str = "SELECT passenger_count, AVG(trip_distance) FROM part GROUP BY passenger_count ORDER BY passenger_count";
const COUNT_SQL: &str = "SELECT passenger_count, COUNT(*) FROM part GROUP BY passenger_count ORDER BY passenger_count";
const MIXED_SQL: &str = "SELECT passenger_count, AVG(trip_distance) AS average_distance, COUNT(*) AS trips, COUNT(trip_distance) AS measured FROM part GROUP BY passenger_count ORDER BY passenger_count";

#[derive(Clone, Copy, Default)]
struct Stats {
    distance_tenths: i128,
    rows: u64,
    measured: u64,
}

impl Stats {
    fn mean(&self) -> f64 {
        self.distance_tenths as f64 / (10.0 * self.measured as f64)
    }
}

struct Fixture {
    root: PathBuf,
    groups: Vec<Stats>,
}

impl Fixture {
    fn new(rows: usize, passengers: usize) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rusthouse-passenger-distance-bench-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();

        let columns = [
            Column {
                name: "passenger_count".into(),
                data_type: "UInt8".into(),
            },
            Column {
                name: "trip_distance".into(),
                data_type: "Nullable(Float64)".into(),
            },
        ];
        let mut groups = vec![Stats::default(); passengers];
        for (part, first) in (0..rows).step_by(PART_ROWS).enumerate() {
            let mut values = Vec::with_capacity((rows - first).min(PART_ROWS));
            for row in first..(first + PART_ROWS).min(rows) {
                let group = row % passengers;
                let stats = &mut groups[group];
                stats.rows += 1;
                // Integer tenths give the expected mean an independent oracle.
                let tenths = ((row as u64 * 48_271 + 17) % 2_000 + 1) as i64;
                let distance = if row % 11 == 0 {
                    None
                } else {
                    stats.measured += 1;
                    stats.distance_tenths += i128::from(tenths);
                    Some((tenths as f64 / 10.0).to_string())
                };
                values.push(vec![Some((group + 1).to_string()), distance]);
            }
            let part = part + 1;
            write_wide_part(
                &root.join(format!("all_{part}_{part}_0")),
                &columns,
                &values,
            )
            .unwrap();
        }
        Self { root, groups }
    }

    fn check(&self, output: &[u8], kind: &str) {
        let result = std::str::from_utf8(output).unwrap();
        let mut lines = result.lines();
        let header = match kind {
            "avg" => "passenger_count\tavg(trip_distance)",
            "count" => "passenger_count\tcount",
            "mixed" => "passenger_count\taverage_distance\ttrips\tmeasured",
            _ => unreachable!(),
        };
        assert_eq!(lines.next(), Some(header));
        for (group, stats) in self.groups.iter().enumerate() {
            let line = lines.next().expect("missing passenger group");
            let fields = line.split('\t').collect::<Vec<_>>();
            assert_eq!(fields[0], (group + 1).to_string());
            let check_mean = |value: &str| {
                let actual = value.parse::<f64>().expect("invalid AVG result");
                assert!(
                    (actual - stats.mean()).abs() < 1e-6,
                    "passengers {}: expected {}, got {actual}",
                    group + 1,
                    stats.mean()
                );
            };
            match kind {
                "avg" => {
                    assert_eq!(fields.len(), 2);
                    check_mean(fields[1]);
                }
                "count" => {
                    assert_eq!(fields.len(), 2);
                    assert_eq!(fields[1].parse::<u64>().unwrap(), stats.rows);
                }
                "mixed" => {
                    assert_eq!(fields.len(), 4);
                    check_mean(fields[1]);
                    assert_eq!(fields[2].parse::<u64>().unwrap(), stats.rows);
                    assert_eq!(fields[3].parse::<u64>().unwrap(), stats.measured);
                }
                _ => unreachable!(),
            }
        }
        assert!(lines.next().is_none(), "unexpected passenger group");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn options() -> (usize, usize, usize) {
    let mut rows = DEFAULT_ROWS;
    let mut groups = DEFAULT_GROUPS;
    let mut samples = DEFAULT_SAMPLES;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bench" => {}
            "--rows" => rows = args.next().expect("--rows needs a value").parse().unwrap(),
            "--passenger-groups" => {
                groups = args
                    .next()
                    .expect("--passenger-groups needs a value")
                    .parse()
                    .unwrap()
            }
            "--samples" => {
                samples = args.next().expect("--samples needs a value").parse().unwrap()
            }
            "-h" | "--help" => {
                println!("Usage: passenger_distance [--rows N] [--passenger-groups 1..8] [--samples odd-N]");
                std::process::exit(0);
            }
            _ => panic!("unexpected benchmark argument: {arg}"),
        }
    }
    assert!((1..=1_048_576).contains(&rows));
    assert!((1..=8).contains(&groups) && rows >= groups * 2);
    assert!(samples > 0 && samples <= 101 && samples % 2 == 1);
    (rows, groups, samples)
}

fn measure(fixture: &Fixture, rows: usize, samples: usize, kind: &str, sql: &str) {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    for _ in 0..WARMUPS {
        output.clear();
        execute(black_box(&query), Some(&fixture.root), &mut output).unwrap();
        fixture.check(&output, kind);
    }
    let mut timings = Vec::with_capacity(samples);
    for _ in 0..samples {
        output.clear();
        let start = Instant::now();
        execute(black_box(&query), Some(&fixture.root), &mut output).unwrap();
        timings.push(start.elapsed().as_secs_f64());
        black_box(&output);
        fixture.check(&output, kind);
    }
    timings.sort_by(f64::total_cmp);
    let median = timings[samples / 2];
    let p90 = timings[(samples * 9).div_ceil(10) - 1];
    println!(
        "{kind:<8} {:>12.3} {:>12.3} {:>16.2}",
        median * 1000.0,
        p90 * 1000.0,
        rows as f64 / median / 1_000_000.0
    );
}

fn main() {
    let (rows, groups, samples) = options();
    let fixture = Fixture::new(rows, groups);
    println!("RustHouse passenger distances: {rows} rows, {groups} passenger groups, {samples} samples");
    println!(
        "{:<8} {:>12} {:>12} {:>16}",
        "query", "median ms", "p90 ms", "M rows/s"
    );
    for (kind, sql) in [
        ("avg", AVG_SQL),
        ("count", COUNT_SQL),
        ("mixed", MIXED_SQL),
    ] {
        measure(&fixture, rows, samples, kind, sql);
    }
}
