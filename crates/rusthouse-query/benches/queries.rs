use rusthouse_query::{execute, parse_select};
use std::fmt::Write as _;
use std::hint::black_box;
use std::time::Instant;

const DEFAULT_ROWS: u64 = 100_000;
const DEFAULT_SAMPLES: usize = 11;
const WARMUP_RUNS: usize = 3;

fn options() -> (u64, usize) {
    let mut rows = DEFAULT_ROWS;
    let mut samples = DEFAULT_SAMPLES;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            // Cargo passes this to custom bench targets with harness = false.
            "--bench" => {}
            "--rows" => {
                rows = args
                    .next()
                    .expect("--rows needs a value")
                    .parse()
                    .expect("--rows must be an integer");
            }
            "--samples" => {
                samples = args
                    .next()
                    .expect("--samples needs a value")
                    .parse()
                    .expect("--samples must be an integer");
            }
            "-h" | "--help" => {
                println!(
                    "Usage: cargo bench -p rusthouse-query --bench queries -- [--rows N] [--samples N]"
                );
                std::process::exit(0);
            }
            _ => panic!("unknown benchmark argument: {arg}"),
        }
    }
    assert!(
        (1..=1_000_000).contains(&rows),
        "--rows must be between 1 and 1000000"
    );
    assert!(
        samples > 0 && samples & 1 == 1,
        "--samples must be a positive odd number"
    );
    (rows, samples)
}

fn measure(name: &str, sql: &str, expected: &str, rows: u64, samples: usize) {
    // Parse once so the measurement isolates query execution.
    let query = parse_select(sql).expect("benchmark SQL must parse");
    let mut output = Vec::new();

    for _ in 0..WARMUP_RUNS {
        output.clear();
        execute(black_box(&query), None, &mut output).expect("warmup must execute");
    }
    assert_eq!(
        output,
        expected.as_bytes(),
        "{name} returned the wrong result"
    );

    let mut timings = Vec::with_capacity(samples);
    for _ in 0..samples {
        // Reuse the output allocation but do not time its clearing or assertions.
        output.clear();
        let start = Instant::now();
        execute(black_box(&query), None, &mut output).expect("benchmark must execute");
        timings.push(start.elapsed());
        black_box(&output);
        assert_eq!(
            output,
            expected.as_bytes(),
            "{name} returned the wrong result"
        );
    }
    timings.sort_unstable();
    let median_seconds = timings[samples / 2].as_secs_f64();
    println!(
        "{name:<18} {:>10.6} {:>17.4}",
        median_seconds * 1000.0,
        rows as f64 / median_seconds / 1_000_000.0
    );
}

fn main() {
    let (rows, samples) = options();
    let middle = rows / 2;
    let matched = rows - middle;
    let sum = (u128::from(middle) + u128::from(rows - 1)) * u128::from(matched) / 2;

    let mut top_k = String::from("number\n");
    for number in (0..rows).rev().filter(|number| number % 7 == 0).take(100) {
        writeln!(&mut top_k, "{number}").unwrap();
    }

    println!(
        "RustHouse numbers() benchmarks: {rows} input rows, {samples} samples, {WARMUP_RUNS} warmups"
    );
    println!(
        "{:<18} {:>10} {:>17}",
        "workload", "median ms", "M input rows/s"
    );

    measure(
        "filtered count",
        &format!("SELECT count(*) FROM numbers({rows}) WHERE number % 7 = 0"),
        &format!("count\n{}\n", rows.div_ceil(7)),
        rows,
        samples,
    );
    measure(
        "filtered sum",
        &format!("SELECT sum(number) FROM numbers({rows}) WHERE number >= {middle}"),
        &format!("sum(number)\n{sum}\n"),
        rows,
        samples,
    );
    measure(
        "filtered top-K",
        &format!(
            "SELECT number FROM numbers({rows}) WHERE number % 7 = 0 ORDER BY number DESC LIMIT 100"
        ),
        &top_k,
        rows,
        samples,
    );
}
