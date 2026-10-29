use super::*;

#[test]
fn chained_join_hash_keys_include_allocated_text_and_duplicates() {
    let key = vec![typed::Key::Text(vec![b'x'; 2048])];
    assert!(hash_key_bytes(&key).unwrap() >= 2048 + std::mem::size_of::<typed::Key>());
    let mut used = 0usize;
    charge_hash_bytes(&mut used, hash_key_bytes(&key).unwrap()).unwrap();
    let charged = used;
    charge_hash_bytes(&mut used, 2 * std::mem::size_of::<usize>()).unwrap();
    assert!(used > charged);
    used = MAX_CHAIN_HASH_BYTES - 1;
    assert!(charge_hash_bytes(&mut used, hash_key_bytes(&key).unwrap()).is_err());
    assert_eq!(used, MAX_CHAIN_HASH_BYTES - 1);
}

#[test]
fn singleton_bucket_promotes_to_duplicate_matches_in_input_order() {
    let mut bucket = Bucket::One(3);
    assert_eq!(bucket.iter().copied().collect::<Vec<_>>(), vec![3]);
    bucket.push(7);
    bucket.push(9);
    assert_eq!(bucket.iter().copied().collect::<Vec<_>>(), vec![3, 7, 9]);
}
