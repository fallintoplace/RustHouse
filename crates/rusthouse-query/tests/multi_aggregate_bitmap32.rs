use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn exact_uint32_distinct_in_multiple_aggregates_exceeds_100k_distinct_values() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-multi-u32-distinct-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let cols = [
        Column { name: "id".into(), data_type: "UInt32".into() },
        Column { name: "revenue".into(), data_type: "UInt8".into() },
    ];
    let rows = (0..120_100u32).map(|id| {
        vec![Some(id.to_string()), Some("1".into())]
    }).collect::<Vec<_>>();
    write_wide_part(&dir, &cols, &rows).unwrap();
    let query = parse_select("SELECT count(DISTINCT id), sum(revenue) FROM part").unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&dir), &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "count(DISTINCT id)\tsum(revenue)\n120100\t120100\n"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn mixed_nullable_signed_int32_distinct_ignores_null_and_preserves_bit_patterns() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-multi-i32-distinct-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let cols = [
        Column { name: "id".into(), data_type: "Nullable(Int32)".into() },
        Column { name: "revenue".into(), data_type: "UInt8".into() },
    ];
    let rows = [
        vec![Some((-1i32).to_string()), Some("1".into())],
        vec![Some(i32::MIN.to_string()), Some("1".into())],
        vec![Some((-1i32).to_string()), Some("1".into())],
        vec![None, Some("1".into())],
    ];
    write_wide_part(&dir, &cols, &rows).unwrap();
    let query = parse_select("SELECT count(DISTINCT id), sum(revenue) FROM part").unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&dir), &mut output).unwrap();
    assert!(String::from_utf8(output).unwrap().ends_with("2\t4\n"));
    fs::remove_dir_all(dir).unwrap();
}
