use super::*;

#[test]
fn tiny_typed_snapshot_opens_parts_without_a_prefetch_thread() {
    use crate::{Column, write_wide_part};
    use std::fs;
    let root =
        std::env::temp_dir().join(format!("rusthouse-tiny-prefetch-{}", std::process::id(),));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let columns = [Column {
        name: "value".into(),
        data_type: "UInt64".into(),
    }];
    for index in 0..2u64 {
        write_wide_part(
            &root.join(format!("all_{index}_{index}_0")),
            &columns,
            &[vec![Some(index.to_string())]],
        )
        .unwrap();
    }
    let mut scan = TypedQueryReader::open(&root, &["value".into()]).unwrap();
    let mut seen = Vec::new();
    while let Some(batch) = scan.next_batch().unwrap() {
        let TypedColumn::Numeric(crate::NumericColumn::UInt64(values)) = &batch[0] else {
            panic!("expected UInt64 data");
        };
        seen.extend_from_slice(values);
    }
    assert_eq!(seen, [0, 1]);
    assert_eq!(scan.prefetch_worker_starts, 0);
    drop(scan);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn typed_part_prefetch_keeps_physical_row_order_and_offset() {
    use crate::{Column, write_wide_part};
    use std::fs;

    let root = std::env::temp_dir().join(format!(
        "rusthouse-typed-part-prefetch-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let cols = [Column {
        name: "category".into(),
        data_type: "String".into(),
    }];
    for index in 0..24 {
        write_wide_part(
            &root.join(format!("all_{index}_{index}_0")),
            &cols,
            &[vec![Some(format!("category-{index}"))]],
        )
        .unwrap();
    }

    let mut scan = TypedQueryReader::open(&root, &["category".into()]).unwrap();
    let mut seen = Vec::new();
    while let Some(batch) = scan.next_batch().unwrap() {
        let TypedColumn::Text(values) = &batch[0] else {
            panic!("expected String batch");
        };
        for index in 0..values.len() {
            seen.push(values.get(index).unwrap().to_vec());
        }
    }
    assert_eq!(
        seen,
        (0..24)
            .map(|index| format!("category-{index}").into_bytes())
            .collect::<Vec<_>>()
    );
    assert_eq!(scan.prefetch_worker_starts, 1);
    drop(scan);

    let mut paged = TypedQueryReader::open(&root, &["category".into()]).unwrap();
    assert_eq!(paged.skip_prefix_rows(18).unwrap(), 18);
    let mut after_offset = Vec::new();
    while let Some(batch) = paged.next_batch().unwrap() {
        let TypedColumn::Text(values) = &batch[0] else {
            panic!("expected String batch");
        };
        for index in 0..values.len() {
            after_offset.push(values.get(index).unwrap().to_vec());
        }
    }
    assert_eq!(
        after_offset,
        (18..24)
            .map(|index| format!("category-{index}").into_bytes())
            .collect::<Vec<_>>()
    );
    drop(paged);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn prefetched_nullable_parts_preserve_nullmap_only_scan() {
    use crate::{Column, write_wide_part};
    use std::fs;

    let root =
        std::env::temp_dir().join(format!("rusthouse-nullmap-prefetch-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let cols = [Column {
        name: "value".into(),
        data_type: "Nullable(UInt64)".into(),
    }];
    for index in 0..16 {
        write_wide_part(
            &root.join(format!("all_{index}_{index}_0")),
            &cols,
            &[vec![if index % 3 == 0 {
                None
            } else {
                Some(index.to_string())
            }]],
        )
        .unwrap();
    }
    let mut scan = TypedQueryReader::open(&root, &["value".into()]).unwrap();
    let mut nullmaps = Vec::new();
    while let Some(map) = scan.next_null_batch().unwrap() {
        nullmaps.extend_from_slice(&map);
    }
    assert_eq!(
        nullmaps,
        (0..16)
            .map(|index| if index % 3 == 0 { 1u8 } else { 0u8 })
            .collect::<Vec<_>>()
    );
    drop(scan);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn typed_kinds_own_their_null_map_requirements() {
    for kind in [
        "Nullable(String)",
        "Nullable(UInt64)",
        "Nullable(DateTime64(3))",
    ] {
        assert!(StorageKind::from_data_type(kind).unwrap().needs_null_map());
    }
    for kind in ["String", "UUID", "UInt64", "DateTime"] {
        assert!(!StorageKind::from_data_type(kind).unwrap().needs_null_map());
    }
    assert!(StorageKind::from_data_type("Nullable(UUID)").is_err());
}

#[test]
fn adjacent_string_marks_reuse_one_decoded_block() {
    use ch_cityhash102::cityhash128;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "rusthouse-typed-string-cache-{}-{}.bin",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let data = b"\x03abc\x03def";
    let mut encoded = vec![0x02];
    encoded.extend_from_slice(&(9u32 + data.len() as u32).to_le_bytes());
    encoded.extend_from_slice(&(data.len() as u32).to_le_bytes());
    encoded.extend_from_slice(data);
    let mut checked = u128::from(cityhash128(&encoded)).to_le_bytes().to_vec();
    checked.extend_from_slice(&encoded);
    std::fs::write(&path, checked).unwrap();

    let mut stream = Stream {
        marks: vec![
            WideMark {
                compressed_offset: 0,
                decompressed_offset: 0,
                rows: Some(1),
            },
            WideMark {
                compressed_offset: 0,
                decompressed_offset: 4,
                rows: Some(1),
            },
        ],
        reader: CompressedBlockReader::open(&path).unwrap(),
        cached: None,
    };
    {
        let mut first = MarkStream::open(&mut stream, 0).unwrap();
        let values = read_legacy_strings_buffered(&mut first, 1).unwrap();
        assert_eq!(values.get(0), Some(b"abc".as_slice()));
    }
    assert!(
        stream
            .cached
            .as_ref()
            .is_some_and(|block| block.block.offset == 0)
    );
    {
        let mut second = MarkStream::open(&mut stream, 1).unwrap();
        let values = read_legacy_strings_buffered(&mut second, 1).unwrap();
        assert_eq!(values.get(0), Some(b"def".as_slice()));
    }
    assert!(
        stream
            .cached
            .as_ref()
            .is_some_and(|block| block.block.offset == 0)
    );
    drop(stream);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn typed_reader_seeks_to_validated_granule_boundaries() {
    use crate::{Column, write_wide_part};
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "rusthouse-typed-mark-seek-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let part = root.join("all_1_1_0");
    let columns = vec![
        Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "note".into(),
            data_type: "Nullable(String)".into(),
        },
    ];
    let rows = (0..2050)
        .map(|id| {
            vec![
                Some(id.to_string()),
                if id % 2 == 0 {
                    None
                } else {
                    Some(format!("row-{id}"))
                },
            ]
        })
        .collect::<Vec<_>>();
    write_wide_part(&part, &columns, &rows).unwrap();
    let metadata = crate::inspect_query_source(&part).unwrap();
    let mut snapshot =
        TypedQueryReader::open_with_metadata(&part, &["id".into(), "note".into()], &metadata)
            .unwrap();
    let mut scanned = 0;
    while let Some(batch) = snapshot.next_batch().unwrap() {
        scanned += batch[0].len();
    }
    assert_eq!(scanned, 2050);
    assert!(matches!(
        TypedQueryReader::open_with_metadata(&root, &["id".into()], &metadata,),
        Err(PartError::InvalidScan(_))
    ));

    let mut reader = TypedPartReader::open(&part, &["id".into(), "note".into()]).unwrap();
    assert_eq!(reader.mark_count(), 3);
    let mut skip = TypedPartReader::open(&part, &["id".into(), "note".into()]).unwrap();
    assert_eq!(skip.skip_complete_marks(2049), 2048);
    assert_eq!(skip.next_batch().unwrap().unwrap()[0].len(), 2);

    reader.seek_to_mark(2).unwrap();
    let tail = reader.next_batch().unwrap().unwrap();
    match &tail[0] {
        TypedColumn::Numeric(NumericColumn::UInt64(values)) => {
            assert_eq!(values, &[2048, 2049]);
        }
        _ => panic!("expected UInt64"),
    }
    assert_eq!(tail[1].len(), 2);
    assert!(reader.next_batch().unwrap().is_none());

    reader.seek_to_mark(0).unwrap();
    assert_eq!(reader.next_batch().unwrap().unwrap()[0].len(), 1024);
    reader.seek_to_mark(reader.mark_count()).unwrap();
    assert!(reader.next_batch().unwrap().is_none());
    assert!(matches!(
        reader.seek_to_mark(4),
        Err(PartError::InvalidGranuleIndex { index: 4, marks: 3 })
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn typed_offset_skips_complete_snapshot_parts_before_decoding() {
    use crate::{Column, write_wide_part};
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "rusthouse-typed-page-part-skip-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let columns = vec![Column {
        name: "text".into(),
        data_type: "String".into(),
    }];
    for i in 0..4 {
        write_wide_part(
            &root.join(format!("all_{i}_{i}_0")),
            &columns,
            &[vec![Some(format!("row-{i}"))]],
        )
        .unwrap();
    }
    let metadata = crate::inspect_query_source(&root).unwrap();
    let mut scan =
        TypedQueryReader::open_with_metadata(&root, &["text".into()], &metadata).unwrap();
    assert_eq!(scan.skip_prefix_rows(3).unwrap(), 3);
    let tail = scan.next_batch().unwrap().unwrap();
    assert!(
        matches!(&tail[0], TypedColumn::Text(values) if values.get(0) == Some(b"row-3".as_slice()))
    );
    assert!(scan.next_batch().unwrap().is_none());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn map_text_plain_utf8_fast_path_keeps_exact_json_escaping() {
    let mut output = String::new();
    quoted_map_text(&mut output, "café 東京".as_bytes()).unwrap();
    assert_eq!(output, "\"café 東京\"");
    output.clear();
    quoted_map_text(&mut output, b"quote\\\" and \\t tab").unwrap();
    assert_eq!(output, "\"quote\\\\\\\" and \\\\t tab\"");
    output.clear();
    quoted_map_text(&mut output, b"").unwrap();
    assert_eq!(output, "\"\"");
}
