#[test]
fn bounded_waves_keep_stable_peer_order_without_skips() {
    let peers: Vec<usize> = (0..32).collect();
    let selected: Vec<usize> = peers
        .chunks(8)
        .flat_map(|wave| wave.iter().copied())
        .collect();
    assert_eq!(selected, peers);
    assert_eq!(peers.chunks(8).count(), 4);
}
