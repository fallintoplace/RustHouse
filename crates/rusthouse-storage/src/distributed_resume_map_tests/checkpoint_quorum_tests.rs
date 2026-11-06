use super::*;

#[test]
fn bit_sliced_votes_match_independent_per_chunk_counts() {
    for chunks in [0, 1, 7, 8, 9, 63, 64, 65, 511, 512, 513, 65_536] {
        for members in [1usize, 2, 3, 7, 8, 17, 64] {
            let maps = (0..members)
                .map(|member| {
                    let mut map = ResumeMap::new(chunks).unwrap();
                    for chunk in 0..chunks {
                        if (chunk.wrapping_mul(13) ^ member.wrapping_mul(17)) % 11 < 6 {
                            map.mark_complete(chunk).unwrap();
                        }
                    }
                    map
                })
                .collect::<Vec<_>>();
            let refs = maps.iter().collect::<Vec<_>>();
            for required in [1, members / 2 + 1, members] {
                let quorum = ResumeMap::quorum_verified(&refs, required).unwrap();
                assert_eq!(quorum.len(), chunks);
                assert_eq!(
                    quorum.encode_binary_checked().len(),
                    12 + chunks.div_ceil(8)
                );
                for chunk in 0..chunks {
                    let votes = maps
                        .iter()
                        .filter(|map| map.completed[chunk / 8] & (1 << (chunk % 8)) != 0)
                        .count();
                    let is_verified = quorum.completed[chunk / 8] & (1 << (chunk % 8)) != 0;
                    assert_eq!(
                        is_verified,
                        votes >= required,
                        "chunks={chunks}, members={members}, required={required}, chunk={chunk}"
                    );
                }
                assert_eq!(
                    ResumeMap::decode_binary_checked(&quorum.encode_binary_checked()).unwrap(),
                    quorum,
                );
            }
        }
    }
}

#[test]
fn rejects_invalid_quorums_and_mismatched_layouts() {
    let a = ResumeMap::new(8).unwrap();
    let b = ResumeMap::new(9).unwrap();
    assert!(ResumeMap::quorum_verified(&[], 1).is_err());
    assert!(ResumeMap::quorum_verified(&[&a], 0).is_err());
    assert!(ResumeMap::quorum_verified(&[&a], 2).is_err());
    assert!(ResumeMap::quorum_verified(&[&a, &b], 1).is_err());
    assert!(ResumeMap::quorum_verified(&vec![&a; 65], 32).is_err());
    assert_eq!(ResumeMap::quorum_verified(&[&a], 1).unwrap(), a);
}
