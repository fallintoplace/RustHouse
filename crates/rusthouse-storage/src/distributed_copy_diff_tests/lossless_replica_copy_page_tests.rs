use super::*;

fn object(name: &str, bytes: u64) -> ObjectMetadata {
    ObjectMetadata {
        key: name.into(),
        bytes,
        crc32c: bytes as u32,
    }
}

#[test]
fn partial_byte_budgets_never_lose_skipped_keys() {
    let sources = [
        object("a/data.bin", 8),
        object("b/data.bin", 6),
        object("c/data.bin", 2),
        object("d/data.bin", 0),
    ];
    let mut cursor: Option<String> = None;
    let mut copied = Vec::new();
    for _ in 0..sources.len() + 1 {
        let page = plan_replica_copy_page(&sources, &[], cursor.as_deref(), 10, 2).unwrap();
        copied.extend(page.copies.iter().map(|item| item.key.clone()));
        cursor = page.next_after;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        copied,
        ["a/data.bin", "b/data.bin", "c/data.bin", "d/data.bin"]
    );
}

#[test]
fn blocked_first_object_fails_instead_of_returning_unsafe_cursor() {
    let source = [object("a/file", 11), object("b/file", 1)];
    assert!(plan_replica_copy_page(&source, &[], None, 10, 8).is_err());
    let page = plan_replica_copy_page(&source, &[], Some("a/file"), 10, 8).unwrap();
    assert_eq!(page.copies, vec![source[1].clone()]);
    assert!(page.next_after.is_none());
}

#[test]
fn pagination_respects_one_item_limits_and_immutable_conflicts() {
    let source = [object("a/file", 0), object("b/file", 0)];
    let first = plan_replica_copy_page(&source, &[], None, 0, 1).unwrap();
    assert_eq!(first.copies, vec![source[0].clone()]);
    assert_eq!(first.next_after.as_deref(), Some("a/file"));
    let second =
        plan_replica_copy_page(&source, &[], first.next_after.as_deref(), 0, 1).unwrap();
    assert_eq!(second.copies, vec![source[1].clone()]);
    assert!(second.next_after.is_none());
    assert!(plan_replica_copy_page(&source, &[], None, 0, 0).is_err());
    let conflict = [object("a/file", 999)];
    assert!(plan_replica_copy_page(&source, &conflict, None, 100, 10).is_err());
}
