use rusthouse_storage::{
    Column, NumericColumn, TypedColumn, TypedPartReader,
    format_date32, parse_date32, verify_wide_part, write_wide_part,
};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn date32_round_trips_pre_epoch_leap_and_extended_future() {
    for (input, expected) in [
        ("1969-12-31", -1),
        ("1970-01-01", 0),
        ("2000-02-29", 11016),
    ] {
        assert_eq!(parse_date32(input), Some(expected));
        assert_eq!(format_date32(expected), input);
    }
    assert!(parse_date32("1900-02-29").is_none());
    assert!(parse_date32("2024-02-30").is_none());
}

#[test]
fn wide_parts_store_nullable_date32_as_signed_four_byte_days() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-date32-{}-{}", std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column {name:"shipdate".into(), data_type:"Date32".into()},
        Column {name:"arrival".into(), data_type:"Nullable(Date32)".into()},
    ];
    let rows = [
        vec![Some("1969-12-31".into()), None],
        vec![Some("1970-01-01".into()), Some("2000-02-29".into())],
        vec![Some("2299-12-31".into()), Some("1969-12-31".into())],
    ];
    write_wide_part(&path, &columns, &rows).unwrap();
    verify_wide_part(&path).unwrap();
    let mut reader = TypedPartReader::open(&path, &["shipdate".into(), "arrival".into()]).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    assert!(matches!(&batch[0], TypedColumn::Numeric(NumericColumn::Int32(values))
        if values == &vec![-1, 0, parse_date32("2299-12-31").unwrap()]));
    assert!(matches!(&batch[1], TypedColumn::NullableNumeric {
        values: NumericColumn::Int32(values), nulls
    } if nulls == &vec![1,0,0] && values[1] == parse_date32("2000-02-29").unwrap()));
    fs::remove_dir_all(path).unwrap();
}
