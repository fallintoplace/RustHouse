//! Maximum feasible flow through a bounded directed replica network.
//! This is a capacity planner; it does not perform transport reservations.
use std::collections::VecDeque;

const MAX_VERTICES: usize = 64;
const MAX_LINKS: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityLink {
    pub from: usize,
    pub to: usize,
    pub bytes_per_second: u64,
}

/// Return the maximum aggregate source-to-sink capacity in bytes per second.
/// Antiparallel and repeated links are allowed and combined without overflow.
pub fn maximum_replica_flow(
    vertices: usize,
    links: &[CapacityLink],
    source: usize,
    sink: usize,
) -> Result<u64, &'static str> {
    if vertices < 2 || vertices > MAX_VERTICES || links.len() > MAX_LINKS
        || source >= vertices || sink >= vertices || source == sink
    {
        return Err("invalid replica flow topology");
    }
    // Reverse residual capacity may exceed u64 when antiparallel links exist.
    let mut residual = vec![vec![0u128; vertices]; vertices];
    for link in links {
        if link.from >= vertices || link.to >= vertices
            || link.from == link.to || link.bytes_per_second == 0
        {
            return Err("invalid replica network link");
        }
        residual[link.from][link.to] = residual[link.from][link.to]
            .checked_add(u128::from(link.bytes_per_second))
            .ok_or("replica link capacity overflow")?;
    }

    let mut total_flow = 0u64;
    loop {
        let mut parent = vec![usize::MAX; vertices];
        parent[source] = source;
        let mut queue = VecDeque::from([source]);
        while let Some(from) = queue.pop_front() {
            for (to, &capacity) in residual[from].iter().enumerate() {
                if capacity > 0 && parent[to] == usize::MAX {
                    parent[to] = from;
                    queue.push_back(to);
                }
            }
            if parent[sink] != usize::MAX { break; }
        }
        if parent[sink] == usize::MAX { return Ok(total_flow); }
        let mut bottleneck = u128::MAX;
        let mut cursor = sink;
        while cursor != source {
            let previous = parent[cursor];
            bottleneck = bottleneck.min(residual[previous][cursor]);
            cursor = previous;
        }
        // Validate all potentially overflowing updates before modifying the
        // residual graph so a maliciously large capacity never wraps.
        let added_flow = u64::try_from(bottleneck)
            .map_err(|_| "replica total flow overflow")?;
        total_flow = total_flow.checked_add(added_flow)
            .ok_or("replica total flow overflow")?;
        cursor = sink;
        while cursor != source {
            let previous = parent[cursor];
            residual[previous][cursor] -= bottleneck;
            residual[cursor][previous] = residual[cursor][previous]
                .checked_add(bottleneck).ok_or("replica reverse capacity overflow")?;
            cursor = previous;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn link(from: usize, to: usize, cap: u64) -> CapacityLink {
        CapacityLink { from, to, bytes_per_second: cap }
    }

    #[test]
    fn discovers_independent_paths_and_shared_bottlenecks() {
        let links = [link(0,1,3), link(0,2,2), link(1,3,3), link(2,3,2)];
        assert_eq!(maximum_replica_flow(4, &links, 0, 3).unwrap(), 5);
        assert_eq!(maximum_replica_flow(4, &links, 3, 0).unwrap(), 0);
    }

    #[test]
    fn combines_parallel_links_without_overflow() {
        assert_eq!(maximum_replica_flow(2, &[link(0,1,3), link(0,1,4)], 0, 1).unwrap(), 7);
        assert!(maximum_replica_flow(2, &[link(0,1,u64::MAX), link(0,1,1)], 0, 1).is_err());
    }

    #[test]
    fn rejects_malformed_topologies() {
        assert!(maximum_replica_flow(2, &[link(1,1,1)], 0, 1).is_err());
        assert!(maximum_replica_flow(2, &[], 0, 0).is_err());
        assert!(maximum_replica_flow(2, &[link(0,2,1)], 0, 1).is_err());
    }
}

#[cfg(test)]
mod antiparallel_residual_tests {
    use super::*;

    #[test]
    fn maximum_reverse_capacity_does_not_reject_valid_flow() {
        let links = [
            CapacityLink { from: 0, to: 1, bytes_per_second: u64::MAX },
            CapacityLink { from: 1, to: 0, bytes_per_second: u64::MAX },
        ];
        assert_eq!(maximum_replica_flow(2, &links, 0, 1).unwrap(), u64::MAX);
        assert_eq!(maximum_replica_flow(2, &links, 1, 0).unwrap(), u64::MAX);
    }
}
