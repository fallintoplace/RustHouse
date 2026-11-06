use super::*;

#[test]
fn direct_map_rows_preserve_empty_and_escaped_values() {
    let mut text = String::from("{");
    quoted_map_text(&mut text, b"k\"x").unwrap();
    text.push(':');
    quoted_map_text(&mut text, "caf\u{e9}\n".as_bytes()).unwrap();
    text.push('}');
    let rows = StringColumn::from_rows(vec![text.as_bytes().to_vec(), b"{}".to_vec()]).unwrap();
    assert_eq!(rows.get(0), Some(text.as_bytes()));
    assert_eq!(rows.get(1), Some(&b"{}"[..]));
}

#[test]
fn map_row_scratch_buffer_retains_capacity_when_reused() {
    let mut row = String::new();
    row.push_str(&"value".repeat(1024));
    let reserved = row.capacity();
    row.clear();
    row.push_str("{}");
    assert_eq!(row, "{}");
    assert!(row.capacity() >= reserved);
}
