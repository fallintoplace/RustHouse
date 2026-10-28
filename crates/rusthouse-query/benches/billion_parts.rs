//! Benchmark verified COUNT latency on an existing immutable billion-row snapshot.
//! Does not synthesize rows or confuse generated numbers() with physical I/O.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{inspect_query_source, query_parts};
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;

fn usage() -> ! {
    eprintln!(
        "Usage: cargo bench -p rusthouse-query --bench billion_parts -- \\
         --part PATH --filter 'id >= 0' --expected-count N \\
         [--min-rows N] [--samples N] [--warmups N]"
    );
    std::process::exit(2);
}

fn main() {
    let mut part: Option<PathBuf> = None;
    let mut filter: Option<String> = None;
    let mut expected_count: Option<u64> = None;
    let mut min_rows = 1_000_000_000u64;
    let mut samples = 7usize;
    let mut warmups = 2usize;
    let mut args = std::env::args().skip(1);
    while let Some(option) = args.next() {
        match option.as_str() {
            "--bench" => {}
            "--part" => part = args.next().map(PathBuf::from),
            "--filter" => filter = args.next(),
            "--expected-count" => {
                expected_count = Some(args.next().unwrap_or_default().parse()
                    .expect("--expected-count must be a UInt64"));
            }
            "--min-rows" => {
                min_rows = args.next().unwrap_or_default().parse()
                    .expect("--min-rows must be a UInt64");
            }
            "--samples" => {
                samples = args.next().unwrap_or_default().parse()
                    .expect("--samples must be a positive odd integer");
            }
            "--warmups" => {
                warmups = args.next().unwrap_or_default().parse()
                    .expect("--warmups must be a nonnegative integer");
            }
            "--help" | "-h" => usage(),
            _ => usage(),
        }
    }
    let (Some(part), Some(filter), Some(expected_count)) = (part, filter, expected_count) else {
        usage();
    };
    assert!(samples > 0 && samples % 2 == 1, "samples must be a positive odd number");
    let parts = query_parts(&part).expect("source must be a frozen wide-part snapshot");
    let metadata = inspect_query_source(&part).expect("invalid part metadata");
    assert!(
        metadata.rows >= min_rows,
        "snapshot has {} physical rows; expected at least {min_rows}",
        metadata.rows
    );

    let sql = format!("SELECT count(*) FROM part WHERE {filter}");
    let query = parse_select(&sql).expect("filtered COUNT SQL must parse");
    let expected = format!("count\\n{expected_count}\\n");
    let mut output = Vec::new();
    for _ in 0..warmups {
        output.clear();
        execute(black_box(&query), Some(&part), &mut output)
            .expect("COUNT warmup failed");
        assert_eq!(output, expected.as_bytes(), "COUNT warmup disagrees with oracle");
    }
    let mut times = Vec::with_capacity(samples);
    for _ in 0..samples {
        output.clear();
        let start = Instant::now();
        execute(black_box(&query), Some(&part), &mut output)
            .expect("COUNT query failed");
        times.push(start.elapsed());
        black_box(&output);
        assert_eq!(output, expected.as_bytes(), "COUNT differs from supplied oracle");
    }
    times.sort_unstable();
    let p50 = times[samples / 2].as_secs_f64();
    let p90 = times[samples.saturating_mul(9).div_ceil(10) - 1].as_secs_f64();
    let logical_rows_per_s = metadata.rows as f64 / p50;

    println!("Physical snapshot rows: {}", metadata.rows);
    println!("Immutable parts: {}", parts.len());
    println!("COUNT query: {sql}");
    println!("Expected matching rows: {expected_count}");
    println!("Warm-cache median: {:.3} ms", p50 * 1_000.0);
    println!("Warm-cache p90: {:.3} ms", p90 * 1_000.0);
    println!("Logical input throughput: {:.2} million rows/s", logical_rows_per_s / 1e6);
    println!("This measures warm-cache single-query latency on real parts.");
    println!("Logical throughput is NOT physical disk throughput or bytes read.");
}
