use super::*;
use crate::{
    NumericColumn, TypedColumn, TypedPartReader, inspect_part, read_checksums, read_wide_marks,
    verify_part_file_sizes, verify_wide_part,
};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn root() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-wide-writer-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}

#[test]
fn completed_writer_syncs_real_part_directory_and_rejects_files() {
    let root = root();
    let part = root.join("complete");
    let columns = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    write_wide_part(&part, &columns, &[vec![Some("42".into())]]).unwrap();
    sync_part_directory(&part).unwrap();
    let not_dir = root.join("not-a-directory");
    fs::write(&not_dir, b"hello").unwrap();
    assert!(sync_part_directory(&not_dir).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_later_column_encoding_removes_partial_part() {
    let root = root();
    let part = root.join("late-error");
    let columns = (0..5)
        .map(|index| Column {
            name: format!("col{index}"),
            data_type: "UInt8".to_owned(),
        })
        .collect::<Vec<_>>();
    let row = (0..5)
        .map(|index| Some(if index == 4 { "invalid" } else { "1" }.to_owned()))
        .collect::<Vec<_>>();
    assert!(write_wide_part(&part, &columns, &[row]).is_err());
    assert!(
        !part.exists(),
        "failed writes must not publish partial parts"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reserves_exact_numeric_granule_widths() {
    for (kind, width) in [
        ("UInt8", 1),
        ("Nullable(Int16)", 2),
        ("Date", 2),
        ("DateTime", 4),
        ("Float64", 8),
        ("UInt128", 16),
        ("Nullable(String)", 0),
    ] {
        assert_eq!(fixed_width_bytes_per_row(kind), width, "{kind}");
    }
}

#[test]
fn writes_105_columns_and_rejects_129() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-wide-columns-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let cols = (0..105)
        .map(|i| Column {
            name: format!("field{i}"),
            data_type: "UInt64".to_owned(),
        })
        .collect::<Vec<_>>();
    let values = vec![Some("3".to_owned()); 105];
    write_wide_part(&root.join("wide"), &cols, &[values]).unwrap();
    assert_eq!(
        crate::inspect_part(root.join("wide"))
            .unwrap()
            .columns
            .len(),
        105
    );
    let too_wide = (0..129)
        .map(|i| Column {
            name: format!("field{i}"),
            data_type: "UInt64".to_owned(),
        })
        .collect::<Vec<_>>();
    assert!(write_wide_part(&root.join("too-wide"), &too_wide, &[]).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn compresses_repeating_streams_but_leaves_tiny_blocks_raw() {
    use crate::CompressedBlockReader;
    use std::io::Cursor;

    for (data, codec) in [(vec![7u8; 4096], 0x82u8), (vec![1, 2, 3], 0x02)] {
        let encoded = block(&data, PartCompression::Lz4).unwrap();
        let mut reader =
            CompressedBlockReader::new(Cursor::new(encoded), "column.bin").unwrap();
        let decoded = reader.next_decoded().unwrap().unwrap();
        assert_eq!(decoded.block.method, codec);
        assert_eq!(decoded.data, data);
        assert!(reader.next_decoded().is_none());
    }
}

#[test]
fn block_checksum_covers_the_exact_encoded_payload() {
    for data in [
        vec![1u8, 2, 3, 4],
        vec![0x55u8; 4096],
        (0..4096).map(|index| (index * 73 % 251) as u8).collect(),
    ] {
        let block = block(&data, PartCompression::Lz4).unwrap();
        let checksum = u128::from(cityhash128(&block[16..])).to_le_bytes();
        assert_eq!(&block[..16], &checksum);
        let stored_size = u32::from_le_bytes(block[17..21].try_into().unwrap());
        assert_eq!(stored_size as usize, block.len() - 16);
        let uncompressed_size = u32::from_le_bytes(block[21..25].try_into().unwrap());
        assert_eq!(uncompressed_size as usize, data.len());
    }
}

#[test]
fn round_trip_numeric_text_and_nullable_values() {
    let root = root();
    let part = root.join("all_1_1_0");
    let columns = vec![
        Column {
            name: "id".into(),
            data_type: "Int64".into(),
        },
        Column {
            name: "message".into(),
            data_type: "String".into(),
        },
        Column {
            name: "score".into(),
            data_type: "Nullable(UInt64)".into(),
        },
    ];
    write_wide_part(
        &part,
        &columns,
        &[
            vec![
                Some("-3".into()),
                Some("hello\tworld".into()),
                Some("12".into()),
            ],
            vec![Some("7".into()), Some("مرحبا".into()), None],
        ],
    )
    .unwrap();
    assert_eq!(inspect_part(&part).unwrap().rows, 2);
    let mut reader =
        TypedPartReader::open(&part, &["id".into(), "message".into(), "score".into()]).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    assert!(
        matches!(&batch[0], TypedColumn::Numeric(NumericColumn::Int64(values)) if values == &vec![-3, 7])
    );
    assert!(
        matches!(&batch[1], TypedColumn::Text(values) if values.get(0) == Some(b"hello\tworld".as_slice()) && values.get(1) == Some("مرحبا".as_bytes()))
    );
    assert!(
        matches!(&batch[2], TypedColumn::NullableNumeric { nulls, .. } if nulls == &vec![0,1])
    );
    assert!(reader.next_batch().unwrap().is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn zstd_blocks_round_trip_and_tiny_blocks_stay_uncompressed() {
    use crate::CompressedBlockReader;
    use std::io::Cursor;

    let data = b"long repeated message for ZSTD part streams ".repeat(128);
    let encoded = block(&data, PartCompression::Zstd).unwrap();
    let mut reader = CompressedBlockReader::new(Cursor::new(encoded), "zstd.bin").unwrap();
    let decoded = reader.next_decoded().unwrap().unwrap();
    assert_eq!(decoded.block.method, 0x90);
    assert_eq!(decoded.data, data);
    assert!(reader.next_decoded().is_none());

    let tiny = block(b"tiny", PartCompression::Zstd).unwrap();
    let mut reader = CompressedBlockReader::new(Cursor::new(tiny), "tiny.bin").unwrap();
    let decoded = reader.next_decoded().unwrap().unwrap();
    assert_eq!(decoded.block.method, 0x02);
    assert_eq!(decoded.data, b"tiny");
}

#[test]
fn writes_zstd_wide_parts_with_readable_marks_and_checksums() {
    use crate::CompressedBlockReader;

    let root = root();
    let part = root.join("all_3_3_0");
    let columns = vec![
        Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "message".into(),
            data_type: "String".into(),
        },
    ];
    let message = "ClickHouse-compatible compression keeps the same column values. ".repeat(4);
    let rows = (0..2050u64)
        .map(|id| vec![Some(id.to_string()), Some(message.clone())])
        .collect::<Vec<_>>();

    write_wide_part_with_compression(&part, &columns, &rows, PartCompression::Zstd).unwrap();
    let mut blocks = CompressedBlockReader::open(part.join("message.bin")).unwrap();
    for _ in 0..3 {
        let decoded = blocks.next_decoded().unwrap().unwrap();
        assert_eq!(decoded.block.method, 0x90);
    }
    assert!(blocks.next_decoded().is_none());

    let marks = read_wide_marks(part.join("message.mrk2")).unwrap();
    assert_eq!(
        marks.marks.iter().map(|mark| mark.rows).collect::<Vec<_>>(),
        vec![Some(1024), Some(1024), Some(2)]
    );
    assert!(marks.marks[0].compressed_offset < marks.marks[1].compressed_offset);
    verify_wide_part(&part).unwrap();

    let mut scan = TypedPartReader::open(&part, &["id".into(), "message".into()]).unwrap();
    for (first, count) in [(0u64, 1024usize), (1024, 1024), (2048, 2)] {
        let batch = scan.next_batch().unwrap().unwrap();
        assert_eq!(batch[0].len(), count);
        assert_eq!(batch[1].len(), count);
        assert!(
            matches!(&batch[0], TypedColumn::Numeric(NumericColumn::UInt64(values))
            if values.first() == Some(&first))
        );
        assert!(matches!(&batch[1], TypedColumn::Text(values)
            if (0..count).all(|index| values.get(index) == Some(message.as_bytes()))));
    }
    assert!(scan.next_batch().unwrap().is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn computes_clickhouse_blockwise_seeded_manifest_hashes() {
    assert_eq!(checksum_hash(b""), 0);
    // CityHash v1.0.2 with seed (0, 0), not CityHash128.
    assert_eq!(
        checksum_hash(b"abc"),
        0xa78da83cc4e0210684541c08b5db83dcu128
    );
    assert_ne!(checksum_hash(b"abc"), u128::from(cityhash128(b"abc")));

    let one_block = vec![42u8; 2048];
    assert_eq!(
        checksum_hash(&one_block),
        u128::from(cityhash128_with_seed(&one_block, U128::new(0, 0)))
    );

    let input = vec![42u8; 4097];
    let first = cityhash128_with_seed(&input[..2048], U128::new(0, 0));
    let second = cityhash128_with_seed(&input[2048..4096], first);
    let tail = cityhash128_with_seed(&input[4096..], second);
    assert_eq!(checksum_hash(&input), u128::from(tail));
    assert_ne!(checksum_hash(&input), u128::from(cityhash128(&input)));
}

#[test]
fn writes_valid_checksums_for_metadata_and_compressed_streams() {
    let root = root();
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
    write_wide_part(
        &part,
        &columns,
        &[
            vec![Some("10".into()), Some("hi".into())],
            vec![Some("20".into()), None],
        ],
    )
    .unwrap();

    let manifest = read_checksums(part.join("checksums.txt")).unwrap();
    assert_eq!(manifest.version, 3);
    assert_eq!(manifest.files.len(), 8);
    for entry in &manifest.files {
        let bytes = fs::read(part.join(&entry.name)).unwrap();
        assert_eq!(entry.size, bytes.len() as u64);
        assert_eq!(entry.hash, checksum_hash(&bytes));
        assert_eq!(entry.uncompressed.is_some(), entry.name.ends_with(".bin"));
    }

    let id = manifest
        .files
        .iter()
        .find(|file| file.name == "id.bin")
        .unwrap();
    let raw_ids = [10u64, 20]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect::<Vec<_>>();
    assert_eq!(
        id.uncompressed,
        Some(UncompressedChecksum {
            size: raw_ids.len() as u64,
            hash: checksum_hash(&raw_ids),
        })
    );
    let nulls = manifest
        .files
        .iter()
        .find(|file| file.name == "note.null.bin")
        .unwrap();
    assert_eq!(
        nulls.uncompressed,
        Some(UncompressedChecksum {
            size: 2,
            hash: checksum_hash(&[0, 1]),
        })
    );
    assert_eq!(verify_part_file_sizes(&part).unwrap().files, 8);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn writes_multiple_aligned_granules_and_reads_each_batch() {
    let root = root();
    let part = root.join("all_2_2_0");
    let cols = vec![
        Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "note".into(),
            data_type: "Nullable(String)".into(),
        },
    ];
    let rows: Vec<Vec<Option<String>>> = (0..2100)
        .map(|i| vec![Some(i.to_string()), (i % 3 != 0).then(|| format!("row{i}"))])
        .collect();
    write_wide_part(&part, &cols, &rows).unwrap();
    let marks = read_wide_marks(part.join("id.mrk2")).unwrap();
    assert_eq!(
        marks.marks.iter().map(|mark| mark.rows).collect::<Vec<_>>(),
        vec![Some(1024), Some(1024), Some(52)]
    );
    assert_eq!(
        marks.marks[0].compressed_offset,
        marks.marks[1].compressed_offset
    );
    assert!(marks.marks[1].decompressed_offset > marks.marks[0].decompressed_offset);
    let note_marks = read_wide_marks(part.join("note.null.mrk2")).unwrap();
    assert_eq!(
        note_marks
            .marks
            .iter()
            .map(|mark| mark.rows)
            .collect::<Vec<_>>(),
        vec![Some(1024), Some(1024), Some(52)]
    );
    verify_wide_part(&part).unwrap();
    let mut scan = TypedPartReader::open(&part, &["id".into(), "note".into()]).unwrap();
    for (first, count) in [(0u64, 1024usize), (1024, 1024), (2048, 52)] {
        let batch = scan.next_batch().unwrap().unwrap();
        assert_eq!(batch[0].len(), count);
        assert_eq!(batch[1].len(), count);
        assert!(
            matches!(&batch[0], TypedColumn::Numeric(NumericColumn::UInt64(values))
                if values.first() == Some(&first)
                    && values.last() == Some(&(first + count as u64 - 1)))
        );
        assert!(matches!(&batch[1], TypedColumn::NullableText { nulls, .. }
            if nulls[0] == u8::from(first % 3 == 0)));
    }
    assert!(scan.next_batch().unwrap().is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn accepts_empty_parts_and_rejects_invalid_rows() {
    let root = root();
    let cols = vec![Column {
        name: "id".into(),
        data_type: "UInt8".into(),
    }];
    let empty = root.join("all_0_0_0");
    write_wide_part(&empty, &cols, &[]).unwrap();
    assert_eq!(inspect_part(&empty).unwrap().rows, 0);
    let manifest = read_checksums(empty.join("checksums.txt")).unwrap();
    assert_eq!(
        manifest
            .files
            .iter()
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>(),
        vec!["columns.txt", "count.txt"]
    );
    assert_eq!(verify_part_file_sizes(&empty).unwrap().files, 2);
    assert!(write_wide_part(&root.join("bad"), &cols, &[vec![Some("256".into())]]).is_err());
    assert!(!root.join("bad").exists());
    assert!(write_wide_part(&root.join("null"), &cols, &[vec![None]]).is_err());
    assert!(!root.join("null").exists());
    assert!(write_wide_part(&empty, &cols, &[]).is_err());
    fs::remove_dir_all(root).unwrap();
}
