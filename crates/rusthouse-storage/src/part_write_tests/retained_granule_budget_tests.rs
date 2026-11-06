use super::*;

#[test]
fn rejects_a_stream_at_the_cap_without_retaining_the_overflow_granule() {
    let mut granules = Vec::new();
    let mut used = MAX_STREAM_BYTES - 1;
    retain_bounded_granule(&mut granules, &mut used, vec![42]).unwrap();
    assert_eq!(used, MAX_STREAM_BYTES);
    assert!(retain_bounded_granule(&mut granules, &mut used, vec![43]).is_err());
    assert_eq!(used, MAX_STREAM_BYTES);
    assert_eq!(granules, vec![vec![42]]);
}

#[test]
fn empty_granules_consume_their_on_disk_padding_byte() {
    let mut granules = Vec::new();
    let mut used = 0;
    retain_bounded_granule(&mut granules, &mut used, Vec::new()).unwrap();
    assert_eq!(used, 1);
    assert_eq!(granules.len(), 1);
}
