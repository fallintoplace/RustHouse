//! Reproducible on-disk scan benchmark using RustHouse-written MergeTree wide parts.
//! Fixture creation and SQL parsing happen outside the timed region.
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
const WARMUP_RUNS: usize = 3;

#[derive(Clone, Copy)]
enum Layout {
    Sorted,
    Shuffled,
}

impl Layout {
    fn name(self) -> &'static str {
        match self {
            Self::Sorted => "sorted",
            Self::Shuffled => "shuffled",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Output {
    Human,
    Csv,
}

/// SplitMix64 keeps the shuffled workload reproducible across Rust versions
/// without adding benchmark-only dependencies.
fn next_random(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *seed;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn ids_for_layout(rows: usize, layout: Layout) -> Vec<usize> {
    let mut ids = (0..rows).collect::<Vec<_>>();
    if matches!(layout, Layout::Shuffled) {
        let mut seed = 0x52c2_247b_1c53_d25du64;
        for end in (1..ids.len()).rev() {
            let chosen = (next_random(&mut seed) % (end as u64 + 1)) as usize;
            ids.swap(end, chosen);
        }
    }
    ids
}

struct Fixture {
    root: PathBuf,
    ids: Vec<usize>,
}

impl Fixture {
    fn new(rows: usize, layout: Layout) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rusthouse-part-bench-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let ids = ids_for_layout(rows, layout);
        let columns = vec![Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        }];
        for (part, chunk) in ids.chunks(PART_ROWS).enumerate() {
            let values = chunk
                .iter()
                .map(|value| vec![Some(value.to_string())])
                .collect::<Vec<_>>();
            let path = root.join(format!("all_{part:06}_{part:06}_0"));
            write_wide_part(&path, &columns, &values).unwrap();
        }
        Self { root, ids }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn options() -> (usize, usize, Layout, Output) {
    let mut rows = DEFAULT_ROWS;
    let mut samples = DEFAULT_SAMPLES;
    let mut layout = Layout::Sorted;
    let mut output = Output::Human;
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
            "--layout" => {
                layout = match args.next().as_deref() {
                    Some("sorted") => Layout::Sorted,
                    Some("shuffled") => Layout::Shuffled,
                    _ => panic!("--layout must be sorted or shuffled"),
                };
            }
            "--csv" => output = Output::Csv,
            "-h" | "--help" => {
                println!(
                    "Usage: cargo bench -p rusthouse-query --bench parts -- [--rows N] [--samples N] [--layout sorted|shuffled] [--csv]"
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
    (rows, samples, layout, output)
}

fn measure(
    label: &str,
    sql: &str,
    expected: &str,
    fixture: &Fixture,
    rows: usize,
    samples: usize,
    report: Output,
) {
    let query = parse_select(sql).expect("invalid benchmark SQL");
    let mut output = Vec::new();
    for _ in 0..WARMUP_RUNS {
        output.clear();
        execute(black_box(&query), Some(&fixture.root), &mut output).unwrap();
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
        execute(black_box(&query), Some(&fixture.root), &mut output).unwrap();
        timings.push(start.elapsed());
        black_box(&output);
        assert_eq!(output, expected.as_bytes(), "{label} returned bad data");
    }
    timings.sort_unstable();
    let median_seconds = timings[samples / 2].as_secs_f64();
    let p90_seconds = timings[samples.saturating_mul(9).div_ceil(10) - 1].as_secs_f64();
    let rows_per_second = rows as f64 / median_seconds / 1_000_000.0;
    match report {
        Output::Human => println!(
            "{label:<18} {:>10.3} {:>10.3} {:>17.2}",
            median_seconds * 1000.0,
            p90_seconds * 1000.0,
            rows_per_second
        ),
        Output::Csv => println!(
            "{label},{:.6},{:.6},{:.6}",
            median_seconds * 1000.0,
            p90_seconds * 1000.0,
            rows_per_second
        ),
    }
}

fn main() {
    let (rows, samples, layout, report) = options();
    let fixture = Fixture::new(rows, layout);
    let middle = rows / 2;
    let retained = rows - middle;
    let sum = (middle as u128 + rows as u128 - 1) * retained as u128 / 2;

    // OFFSET is in physical scan order, so the oracle must use that order.
    let mut page = String::from("id\n");
    for id in fixture.ids.iter().skip(middle).take(100) {
        writeln!(&mut page, "{id}").unwrap();
    }
    let mut top = String::from("id\n");
    for id in (middle..rows).rev().take(100) {
        writeln!(&mut top, "{id}").unwrap();
    }
    let point = middle;
    let point_expected = format!("id\n{point}\n");

    if report == Output::Human {
        println!(
            "RustHouse frozen-parts benchmark: {rows} rows, {samples} samples, layout={}",
            layout.name()
        );
        println!(
            "{:<18} {:>10} {:>10} {:>17}",
            "workload", "median ms", "p90 ms", "M logical rows/s"
        );
        println!("Logical rows/s measures input rows, not bytes read or cold-cache throughput.");
    } else {
        println!("workload,median_ms,p90_ms,million_logical_rows_per_second");
    }
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
            report,
        );
    }
    measure(
        "filtered sum",
        &format!("SELECT sum(id) FROM part WHERE id >= {middle}"),
        &format!("sum(id)\n{sum}\n"),
        &fixture,
        rows,
        samples,
        report,
    );
    measure(
        "point filter",
        &format!("SELECT id FROM part WHERE id = {point}"),
        &point_expected,
        &fixture,
        rows,
        samples,
        report,
    );
    measure(
        "large OFFSET",
        &format!("SELECT id FROM part LIMIT 100 OFFSET {middle}"),
        &page,
        &fixture,
        rows,
        samples,
        report,
    );
    measure(
        "filtered top-K",
        &format!("SELECT id FROM part WHERE id >= {middle} ORDER BY id DESC LIMIT 100"),
        &top,
        &fixture,
        rows,
        samples,
        report,
    );
}
