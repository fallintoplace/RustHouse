//! Reorder safe INNER aggregate chains so a large relation can be streamed.
use super::*;

pub(super) fn plan(query: &JoinSelect, sizes: &[u64]) -> Option<(JoinSelect, Vec<usize>)> {
    if sizes.len() != query.steps.len() + 2
        || query.kind != JoinType::Inner
        || query.any
        || !query.using_keys.is_empty()
        || query.columns.is_none()
        || query.aggregate.is_some()
        || query.on_right_like.is_some()
        || query.grouped_count_column.is_some()
        || !query.grouped_case_counts.is_empty()
        || (query.grouped_count.is_none()
            && query.scalar_sum.is_none()
            && query.scalar_formula.is_none())
        || query
            .steps
            .iter()
            .any(|step| step.kind != JoinType::Inner || step.any || !step.using_keys.is_empty())
    {
        return None;
    }
    let aliases = std::iter::once(&query.left_alias)
        .chain(std::iter::once(&query.right_alias))
        .chain(query.steps.iter().map(|s| &s.alias))
        .collect::<Vec<_>>();
    if aliases
        .iter()
        .enumerate()
        .any(|(i, a)| a.is_empty() || aliases[..i].contains(a))
    {
        return None;
    }
    let mut edges = Vec::new();
    for (index, keys) in std::iter::once(&query.keys)
        .chain(query.steps.iter().map(|step| &step.keys))
        .enumerate()
    {
        if keys.is_empty() {
            return None;
        }
        for (left, right) in keys {
            let a = aliases.iter().position(|alias| **alias == left.relation)?;
            let b = aliases.iter().position(|alias| **alias == right.relation)?;
            let added = index + 1;
            if !((a == added && b < added) || (b == added && a < added)) {
                return None;
            }
            edges.push((a, b, (left.clone(), right.clone())));
        }
    }
    let largest = sizes.iter().enumerate().max_by_key(|(_, n)| *n)?.0;
    if sizes[largest] <= sizes[0] || sizes[largest] <= crate::join::MAX_STREAM_JOIN_BUILD_ROWS {
        return None;
    }
    let mut order = vec![largest];
    let mut assigned = vec![false; edges.len()];
    let mut new_steps = Vec::new();
    while order.len() < aliases.len() {
        let next = (0..aliases.len())
            .filter(|i| !order.contains(i))
            .find(|i| {
                edges.iter().any(|(a, b, _)| {
                    (*a == *i && order.contains(b)) || (*b == *i && order.contains(a))
                })
            })?;
        let mut keys = Vec::new();
        for (i, (a, b, key)) in edges.iter().enumerate() {
            if !assigned[i]
                && ((*a == next && order.contains(b)) || (*b == next && order.contains(a)))
            {
                keys.push(key.clone());
                assigned[i] = true;
            }
        }
        let table = match next {
            0 => &query.left,
            1 => &query.right,
            _ => &query.steps[next - 2].table,
        };
        new_steps.push(JoinStep {
            table: table.clone(),
            alias: aliases[next].clone(),
            kind: JoinType::Inner,
            any: false,
            keys,
            using_keys: Vec::new(),
        });
        order.push(next);
    }
    if assigned.iter().any(|v| !v) {
        return None;
    }
    let first = order[0];
    let mut result = query.clone();
    result.left = match first {
        0 => query.left.clone(),
        1 => query.right.clone(),
        _ => query.steps[first - 2].table.clone(),
    };
    result.left_alias = aliases[first].clone();
    let step = new_steps.remove(0);
    result.right = step.table;
    result.right_alias = step.alias;
    result.keys = step.keys;
    result.steps = new_steps;
    Some((result, order))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn query() -> JoinSelect {
        crate::parse_join("SELECT a.id, SUM(c.id) AS n FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id GROUP BY a.id").unwrap().unwrap()
    }
    #[test]
    fn streams_largest_and_keeps_every_join_key() {
        let query = query();
        let (reordered, order) = plan(&query, &[10, 20, 300_000]).unwrap();
        assert_eq!(order, vec![2, 1, 0]);
        assert_eq!(reordered.left_alias, "c");
        assert_eq!(reordered.right_alias, "b");
        assert_eq!(reordered.keys, query.steps[0].keys);
        assert_eq!(reordered.steps[0].keys, query.keys);
        assert!(plan(&reordered, &[300_000, 20, 10]).is_none());
    }
    #[test]
    fn equal_large_inputs_do_not_reorder_repeatedly() {
        let query = query();
        assert!(plan(&query, &[300_000, 20, 300_000]).is_none());
        let (_, order) = plan(&query, &[10, 300_000, 300_000]).unwrap();
        assert_eq!(order[0], 2);
    }
    #[test]
    fn rejects_invalid_original_predicate_scopes() {
        let mut query = query();
        query.steps[0].keys.push(query.keys[0].clone());
        assert!(plan(&query, &[10, 20, 300_000]).is_none());
        query.steps[0].keys.pop();
        query.steps[0].keys[0].0.relation.clear();
        assert!(plan(&query, &[10, 20, 300_000]).is_none());
    }
    #[test]
    fn attaches_each_predicate_in_a_join_cycle() {
        let mut query = query();
        let edge = (query.keys[0].0.clone(), query.steps[0].keys[0].1.clone());
        query.steps[0].keys.push(edge);
        let (reordered, _) = plan(&query, &[10, 20, 300_000]).unwrap();
        assert_eq!(reordered.keys.len() + reordered.steps[0].keys.len(), 3);
        assert_eq!(reordered.steps[0].keys.len(), 2);
    }
    #[test]
    fn leaves_non_inner_and_any_joins_in_place() {
        let mut query = query();
        query.steps[0].kind = JoinType::Left;
        assert!(plan(&query, &[10, 20, 300_000]).is_none());
        query.steps[0].kind = JoinType::Inner;
        query.any = true;
        assert!(plan(&query, &[10, 20, 300_000]).is_none());
        query.any = false;
        query.using_keys.push("id".into());
        assert!(plan(&query, &[10, 20, 300_000]).is_none());
    }
}
