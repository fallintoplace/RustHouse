use super::*;
use crate::{ArrayOffsetsEncoding, NumericColumn, read_numeric_array_column};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn read_block(path: &Path) -> Vec<u8> {
    let mut reader = crate::CompressedBlockReader::open(path).unwrap();
    reader.next_decoded().unwrap().unwrap().data
}

#[test]
fn writes_clickhouse_numeric_array_element_and_size_streams() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-native-array-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).unwrap();
    let part = dir.join("all_1_1_0");
    let columns = [Column {
        name: "items".into(),
        data_type: "Array(UInt16)".into(),
    }];
    let rows = [
        vec![Some("[12, 65535]".into())],
        vec![Some("[]".into())],
        vec![Some("[7]".into())],
        vec![Some("[0, 1, 2]".into())],
    ];
    write_wide_part(&part, &columns, &rows).unwrap();
    crate::verify_wide_part(&part).unwrap();
    assert!(part.join("items.size0.bin").exists());
    assert!(part.join("items.size0.mrk2").exists());
    let sizes = read_block(&part.join("items.size0.bin"));
    let data = read_block(&part.join("items.bin"));
    let arrays = read_numeric_array_column(
        &mut sizes.as_slice(),
        &mut data.as_slice(),
        "Array(UInt16)",
        4,
        ArrayOffsetsEncoding::Sizes,
    )
    .unwrap();
    assert_eq!(arrays.offsets(), &[0, 2, 2, 3, 6]);
    assert_eq!(
        arrays.values(),
        &NumericColumn::UInt16(vec![12, 65535, 7, 0, 1, 2])
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn all_empty_array_and_map_granules_have_valid_stream_blocks() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-empty-collection-part-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).unwrap();
    let part = dir.join("all_1_1_0");
    let columns = [
        Column {
            name: "items".into(),
            data_type: "Array(Int8)".into(),
        },
        Column {
            name: "attrs".into(),
            data_type: "Map(String, UInt8)".into(),
        },
    ];
    let rows = (0..1025)
        .map(|_| vec![Some("[]".into()), Some("{}".into())])
        .collect::<Vec<_>>();
    write_wide_part(&part, &columns, &rows).unwrap();
    crate::verify_wide_part(&part).unwrap();

    let mut reader =
        crate::TypedPartReader::open(&part, &["items".into(), "attrs".into()]).unwrap();
    for expected in [1024, 1] {
        let batch = reader.next_batch().unwrap().unwrap();
        assert_eq!(batch[0].len(), expected);
        assert_eq!(batch[1].len(), expected);
        assert!(matches!(
            &batch[0],
            crate::TypedColumn::Text(values) if values.get(0) == Some(&b"[]"[..])
        ));
        assert!(matches!(
            &batch[1],
            crate::TypedColumn::Text(values) if values.get(0) == Some(&b"{}"[..])
        ));
    }
    assert!(reader.next_batch().unwrap().is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_invalid_array_values_before_creating_part() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-invalid-arrays-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).unwrap();
    let columns = [Column {
        name: "xs".into(),
        data_type: "Array(UInt8)".into(),
    }];
    for value in ["1,2", "[1,]", "[,1]", "[256]", "[NULL]", "[1, [2]]"] {
        let part = dir.join("reject");
        assert!(
            write_wide_part(&part, &columns, &[vec![Some(value.into())]]).is_err(),
            "{value}"
        );
        assert!(!part.exists(), "{value}");
    }
    assert!(write_wide_part(&dir.join("reject-null"), &columns, &[vec![None]]).is_err());
    assert!(
        write_wide_part(
            &dir.join("reject-nested"),
            &[Column {
                name: "xs".into(),
                data_type: "Array(Array(UInt8))".into(),
            }],
            &[],
        )
        .is_err()
    );
    std::fs::remove_dir_all(dir).unwrap();
}
