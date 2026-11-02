//! Maximum feasible flow through a bounded directed replica network.
//! This is a capacity planner; it does not perform transport reservations.
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
    if !(2..=MAX_VERTICES).contains(&vertices)
        || links.len() > MAX_LINKS
        || source >= vertices
        || sink >= vertices
        || source == sink
    {
        return Err("invalid replica flow topology");
    }
    let mut residual = vec![vec![0u64; vertices]; vertices];
    for link in links {
        if link.from >= vertices || link.to >= vertices
            || link.from == link.to || link.bytes_per_second == 0
        {
            return Err("invalid replica network link");
        }
        residual[link.from][link.to] = residual[link.from][link.to]
            .checked_add(link.bytes_per_second).ok_or("replica link capacity overflow")?;
    }

    let mut total_flow = 0u64;
    loop {
        // Each vertex enters the BFS queue at most once. Reuse bounded
        // stack scratch across augmenting paths instead of allocating twice.
        let mut parent = [usize::MAX; MAX_VERTICES];
        let mut queue = [0usize; MAX_VERTICES];
        parent[source] = source;
        queue[0] = source;
        let mut head = 0usize;
        let mut tail = 1usize;
        while head < tail {
            let from = queue[head];
            head += 1;
            for (to, &capacity) in residual[from].iter().enumerate() {
                if capacity > 0 && parent[to] == usize::MAX {
                    parent[to] = from;
                    queue[tail] = to;
                    tail += 1;
                }
            }
            if parent[sink] != usize::MAX { break; }
        }
        if parent[sink] == usize::MAX { return Ok(total_flow); }
        let mut bottleneck = u64::MAX;
        let mut cursor = sink;
        while cursor != source {
            let previous = parent[cursor];
            bottleneck = bottleneck.min(residual[previous][cursor]);
            cursor = previous;
        }
        // Validate all potentially overflowing updates before modifying the
        // residual graph so a maliciously large capacity never wraps.
        total_flow = total_flow.checked_add(bottleneck)
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
    fn dense_multi_path_network_exercises_many_augmentations() {
        let mut links = Vec::new();
        for from in 1..=4 {
            links.push(link(0, from, 10));
        }
        for from in 1..=4 {
            for to in 5..=8 {
                links.push(link(from, to, 10));
            }
        }
        for from in 5..=8 {
            links.push(link(from, 9, 10));
        }
        assert_eq!(maximum_replica_flow(10, &links, 0, 9).unwrap(), 40);
        assert_eq!(maximum_replica_flow(10, &links, 9, 0).unwrap(), 0);
    }

    #[test]
    fn rejects_malformed_topologies() {
        assert!(maximum_replica_flow(2, &[link(1,1,1)], 0, 1).is_err());
        assert!(maximum_replica_flow(2, &[], 0, 0).is_err());
        assert!(maximum_replica_flow(2, &[link(0,2,1)], 0, 1).is_err());
    }
}
