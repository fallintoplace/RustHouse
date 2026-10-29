use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn large_unsigned_64bit_dimension_keys_merge_without_signed_conversion() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-parallel-u64-group-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let cols = [Column { name:"id".into(), data_type:"UInt64".into() }];
    for part in 0..20 {
        let rows = [
            vec![Some("18446744073709551615".into())],
            vec![Some("9223372036854775808".into())],
            vec![Some("18446744073709551615".into())],
        ];
        write_wide_part(
            &root.join(format!("p_{}_{}_0", part+1, part+1)), &cols, &rows
        ).unwrap();
    }
    let mut output = Vec::new();
    execute(&parse_select(
        "SELECT id, count(*) FROM part GROUP BY id ORDER BY id ASC"
    ).unwrap(), Some(&root), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(),
        "id\tcount\n9223372036854775808\t20\n18446744073709551615\t40\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn signed_64bit_dimension_keys_are_not_collapsed_into_unsigned_domain() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-parallel-i64-group-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let cols = [Column { name:"id".into(), data_type:"Int64".into() }];
    for part in 0..20 {
        let rows = [-1i64, i64::MIN, 0, i64::MAX].iter()
            .map(|value| vec![Some(value.to_string())]).collect::<Vec<_>>();
        write_wide_part(
            &root.join(format!("p_{}_{}_0", part+1, part+1)), &cols, &rows
        ).unwrap();
    }
    let mut output = Vec::new();
    execute(&parse_select(
        "SELECT id, count(*) FROM part GROUP BY id ORDER BY id ASC"
    ).unwrap(), Some(&root), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(),
        "id\tcount\n-9223372036854775808\t20\n-1\t20\n0\t20\n9223372036854775807\t20\n");
    fs::remove_dir_all(root).unwrap();
}
