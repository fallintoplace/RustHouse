use rusthouse_storage::{
    Column, NumericColumn, TypedColumn, TypedPartReader, datetime64_scale,
    format_datetime64, parse_datetime64, verify_wide_part, write_wide_part,
};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn datetime64_round_trips_fractional_utc_and_pre_epoch_ticks() {
    assert_eq!(datetime64_scale("DateTime64(3, 'UTC')"), Some(3));
    assert_eq!(datetime64_scale("DateTime64(9)"), Some(9));
    assert_eq!(datetime64_scale("DateTime64(10)"), None);
    assert_eq!(datetime64_scale("DateTime64(3, 'Europe/Berlin')"), None);
    assert_eq!(parse_datetime64("1969-12-31 23:59:59.250", 3), Some(-750));
    assert_eq!(format_datetime64(-750, 3).as_deref(),
        Some("1969-12-31 23:59:59.250"));
    assert_eq!(parse_datetime64("1970-01-01 00:00:00.001", 3), Some(1));
    assert_eq!(parse_datetime64("2024-02-29T12:34:56.987+01:00", 3),
        parse_datetime64("2024-02-29 11:34:56.987", 3));
    assert!(parse_datetime64("2024-02-29 12:00:00.1234", 3).is_none());
    assert!(parse_datetime64("2024-02-30 12:00:00.123", 3).is_none());
}

#[test]
fn datetime64_wide_part_writer_supports_nullable_fractional_values() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-datetime64-write-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column {name:"ts".into(), data_type:"DateTime64(3, 'UTC')".into()},
        Column {name:"event_time".into(), data_type:"Nullable(DateTime64(3))".into()},
    ];
    let rows = [
        vec![Some("1969-12-31 23:59:59.250".into()), None],
        vec![Some("1970-01-01 00:00:00.001".into()),
             Some("2024-02-29T12:34:56.987+01:00".into())],
    ];
    write_wide_part(&part, &columns, &rows).unwrap();
    verify_wide_part(&part).unwrap();
    let mut scan = TypedPartReader::open(
        &part, &["ts".to_owned(), "event_time".to_owned()],
    ).unwrap();
    let batch = scan.next_batch().unwrap().unwrap();
    assert!(matches!(&batch[0], TypedColumn::Numeric(NumericColumn::Int64(values))
        if values == &vec![-750, 1]));
    assert!(matches!(&batch[1], TypedColumn::NullableNumeric {
        values: NumericColumn::Int64(values), nulls
    } if nulls == &vec![1,0] &&
        values[1] == parse_datetime64("2024-02-29 11:34:56.987", 3).unwrap()));
    fs::remove_dir_all(part).unwrap();
}
