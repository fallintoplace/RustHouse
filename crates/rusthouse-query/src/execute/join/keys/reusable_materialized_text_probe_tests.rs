use super::*;

#[test]
fn changing_text_probe_key_keeps_backing_allocation() {
    let mut key = Vec::new();
    for input in ["large-tenant-key", "another-key", "large-tenant-key"] {
        let row = [typed::Cell::Text(input.as_bytes().to_vec())];
        let previous = match key.first() {
            Some(typed::Key::Text(value)) => Some(value.as_ptr()),
            _ => None,
        };
        assert!(row_key_into(&row, &[0], &mut key));
        if let Some(ptr) = previous {
            assert_eq!(match &key[0] {
                typed::Key::Text(value) => value.as_ptr(),
                _ => panic!("expected text key"),
            }, ptr);
        }
        assert_eq!(key, row_key(&row, &[0]).unwrap());
    }
    let nullable = [typed::Cell::Null];
    assert!(!row_key_into(&nullable, &[0], &mut key));
    assert!(key.is_empty());
}
