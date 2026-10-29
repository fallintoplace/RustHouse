use super::*;

#[test]
fn external_merge_can_stop_before_reading_the_remaining_run() {
    let scratch = SortScratch::new().unwrap();
    let path = scratch.0.join("run.bin");
    let directions: Arc<[bool]> = Arc::from(Vec::<bool>::new());
    let first = SortedRow {
        key: Number::Unsigned(1), extra_keys: Vec::new(),
        values: vec![Number::Unsigned(1)], sequence: 0,
        descending: false, extra_descending: directions.clone(),
    };
    let second = SortedRow {
        key: Number::Unsigned(2), extra_keys: Vec::new(),
        values: vec![Number::Unsigned(2)], sequence: 1,
        descending: false, extra_descending: directions.clone(),
    };
    let mut writer = BufWriter::new(File::create(&path).unwrap());
    write_sorted_row(&mut writer, &first).unwrap();
    write_sorted_row(&mut writer, &second).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let mut visited = 0;
    merge_runs(&[path], false, &directions, |_| {
        visited += 1;
        Ok(false)
    }).unwrap();
    assert_eq!(visited, 1);
}

#[test]
fn rejects_noncanonical_float_spill_bytes() {
    let mut encoded = Vec::new();
    write_number(&mut encoded, Number::Float(-0.0)).unwrap();
    match read_number(&mut encoded.as_slice()).unwrap() {
        Number::Float(value) => assert_eq!(value.to_bits(), (-0.0f64).to_bits()),
        _ => panic!("expected Float64"),
    }
    encoded[9] = 1;
    assert!(read_number(&mut encoded.as_slice()).is_err());
}

#[cfg(unix)]
#[test]
fn scratch_directory_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = SortScratch::new().unwrap();
    assert_eq!(fs::metadata(&scratch.0).unwrap().permissions().mode() & 0o077, 0);
}

#[test]
fn external_sort_merges_many_runs_and_preserves_stable_ties() {
    let mut sort = SortedRows::new(None, false);
    for index in (0..10_000u128).rev() {
        sort.push(Number::Unsigned(index % 20), vec![Number::Unsigned(index)])
            .unwrap();
    }
    assert!(sort.run_paths.len() > 32);
    let mut bytes = Vec::new();
    sort.write(&mut bytes).unwrap();
    let parsed = std::str::from_utf8(&bytes).unwrap().lines()
        .map(|s| s.parse::<u128>().unwrap()).collect::<Vec<_>>();
    assert_eq!(parsed.len(), 10_000);
    // Rows with the same ORDER BY key retain input arrival order.
    assert_eq!(&parsed[..4], &[9980, 9960, 9940, 9920]);
    assert_eq!(&parsed[parsed.len()-4..], &[79, 59, 39, 19]);
}

#[test]
fn external_sort_handles_descending_multikey_pages_beyond_memory_cap() {
    let mut sort = SortedRows::with_offset(Some(MAX_SORTED_ROWS + 1), 5, true).unwrap();
    sort.set_extra_directions([false]);
    for index in 0..500u128 {
        sort.push_with_extra(
            Number::Unsigned(index % 10),
            vec![Number::Unsigned(index)],
            vec![Number::Unsigned(index)],
        ).unwrap();
    }
    let mut bytes = Vec::new();
    sort.write(&mut bytes).unwrap();
    let rows = std::str::from_utf8(&bytes).unwrap().lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 495);
    assert_eq!(rows[0], "59");
}

#[test]
fn reserves_bounded_topk_and_offset_without_eager_unbounded_allocation() {
    let limited = SortedRows::new(Some(16), false);
    assert!(limited.rows.capacity() >= 16);
    let paged = SortedRows::with_offset(Some(5), 90, true).unwrap();
    assert!(paged.rows.capacity() >= 95);
    let unbounded = SortedRows::new(None, false);
    assert_eq!(unbounded.rows.capacity(), 0);
}

#[test]
fn direction_comparison_reverses_order_without_changing_ties() {
    let one = Number::Unsigned(1);
    let two = Number::Unsigned(2);
    assert_eq!(compare_with_direction(one, two, false), Ordering::Less);
    assert_eq!(compare_with_direction(one, two, true), Ordering::Greater);
    assert_eq!(compare_with_direction(one, one, true), Ordering::Equal);
}

#[test]
fn heap_retains_more_than_one_hundred_thousand_rows() {
    let mut sorted = SortedRows::new(Some(100_001), false);
    for value in 0..100_001u128 {
        sorted
            .push(Number::Unsigned(value), vec![Number::Unsigned(value)])
            .unwrap();
    }
    assert_eq!(sorted.rows.len(), 100_001);
    assert_eq!(sorted.seen, 100_001);
}

#[test]
fn bounded_heap_keeps_the_best_rows_with_offset_and_ties() {
    let mut sorted = SortedRows::with_offset(Some(2), 1, false).unwrap();
    for value in [10u128, 8, 1, 4, 1, 3, 2] {
        sorted
            .push(Number::Unsigned(value), vec![Number::Unsigned(value)])
            .unwrap();
    }
    let mut output = Vec::new();
    sorted.write(&mut output).unwrap();
    assert_eq!(output, b"1\n2\n");

    let mut descending = SortedRows::new(Some(2), true);
    for value in [5u128, 1, 9, 3, 9] {
        descending
            .push(Number::Unsigned(value), vec![Number::Unsigned(value)])
            .unwrap();
    }
    let mut output = Vec::new();
    descending.write(&mut output).unwrap();
    assert_eq!(output, b"9\n9\n");
}

#[test]
fn bounded_topk_can_discard_losing_keys_and_later_ties() {
    let mut sorted = SortedRows::new(Some(2), false);
    sorted.set_extra_directions([true]);
    sorted
        .push_with_extra(
            Number::Unsigned(1),
            vec![Number::Unsigned(5)],
            vec![Number::Unsigned(15)],
        )
        .unwrap();
    sorted
        .push_with_extra(
            Number::Unsigned(1),
            vec![Number::Unsigned(3)],
            vec![Number::Unsigned(13)],
        )
        .unwrap();

    assert!(sorted.would_discard(Number::Unsigned(2), [Number::Unsigned(8)]));
    assert!(sorted.would_discard(Number::Unsigned(1), [Number::Unsigned(3)]));
    assert!(!sorted.would_discard(Number::Unsigned(1), [Number::Unsigned(7)]));
    sorted
        .push_with_extra(
            Number::Unsigned(1),
            vec![Number::Unsigned(7)],
            vec![Number::Unsigned(17)],
        )
        .unwrap();
    let mut output = Vec::new();
    sorted.write(&mut output).unwrap();
    assert_eq!(output, b"17\n15\n");

    let unlimited = SortedRows::new(None, false);
    assert!(!unlimited.would_discard(Number::Unsigned(1), []));
}

#[test]
fn losing_primary_keys_do_not_evaluate_secondary_keys() {
    let mut sorted = SortedRows::new(Some(1), false);
    sorted.set_extra_directions([false]);
    sorted
        .push_with_extra(
            Number::Unsigned(5),
            vec![Number::Unsigned(10)],
            vec![Number::Unsigned(5)],
        )
        .unwrap();
    let mut visited = 0;
    assert!(sorted.would_discard(
        Number::Unsigned(6),
        std::iter::once_with(|| {
            visited += 1;
            Number::Unsigned(1)
        })
    ));
    assert_eq!(visited, 0);
    assert!(!sorted.would_discard(
        Number::Unsigned(5),
        std::iter::once_with(|| {
            visited += 1;
            Number::Unsigned(9)
        })
    ));
    assert_eq!(visited, 1);
}

#[test]
fn rejected_topk_candidates_still_advance_stable_sequence() {
    let mut sorted = SortedRows::new(Some(1), false);
    sorted
        .push(Number::Unsigned(2), vec![Number::Unsigned(2)])
        .unwrap();
    sorted
        .push(Number::Unsigned(3), vec![Number::Unsigned(3)])
        .unwrap();
    sorted
        .push(Number::Unsigned(2), vec![Number::Unsigned(99)])
        .unwrap();
    assert_eq!(sorted.seen, 3);
    assert_eq!(sorted.rows.len(), 1);
    let mut output = Vec::new();
    sorted.write(&mut output).unwrap();
    assert_eq!(output, b"2\n");
}

#[test]
fn shares_sort_directions_and_keeps_secondary_order() {
    let mut sorted = SortedRows::new(Some(3), false);
    sorted.set_extra_directions([true]);
    for number in [10u128, 30, 20] {
        sorted
            .push_with_extra(
                Number::Unsigned(1),
                vec![Number::Unsigned(number)],
                vec![Number::Unsigned(number)],
            )
            .unwrap();
    }
    assert!(
        sorted
            .rows
            .iter()
            .all(|row| Arc::ptr_eq(&row.extra_descending, &sorted.extra_descending))
    );

    let mut output = Vec::new();
    sorted.write(&mut output).unwrap();
    assert_eq!(output, b"30\n20\n10\n");
}
