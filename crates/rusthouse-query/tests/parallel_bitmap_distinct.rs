use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn exact_distinct_across_20_parts_deduplicates_overlapping_int32_keys() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-parallel-distinct-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let columns = [
        Column { name: "id".into(), data_type: "Nullable(Int32)".into() },
    ];
    // Twenty active parts trigger the parallel path on multi-core CI.
    // Each part repeats values from a shared domain; NULL is not a key.
    for part in 0..20 {
        let rows = (0..2000).map(|index| {
            if index == 0 {
                vec![None]
            } else {
                vec![Some(((index + part * 200) as i32 - 5_000).to_string())]
            }
        }).collect::<Vec<_>>();
        write_wide_part(&root.join(format!("p_{}_{}_0", part+1, part+1)), &columns, &rows).unwrap();
    }
    let query = parse_select("SELECT count(DISTINCT id) FROM part").unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&root), &mut output).unwrap();
    // Ranges [1,1999] + 200 * part form one continuous 5800-key domain.
    assert_eq!(String::from_utf8(output).unwrap(), "countDistinct(id)\n5799\n");
    fs::remove_dir_all(root).unwrap();
}
