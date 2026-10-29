use super::*;

fn is_safe_table_select(select: &Select) -> bool {
    matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.rename.is_none()
        && select.order_by.is_none()
        && select.order_by_extra.is_empty()
        && select.having.is_none()
}

fn is_safe_unpaged_select(select: &Select) -> bool {
    is_safe_table_select(select) && select.limit.is_none() && select.offset.is_none()
}

fn is_safe_unpaged_projection(
    select: &Select,
    predicate: impl FnOnce(&Projection) -> bool,
) -> bool {
    is_safe_unpaged_select(select) && predicate(&select.projection)
}

pub(super) fn is_safe_scan(select: &Select) -> bool {
    matches!(select.projection, Projection::All | Projection::Columns(_))
        && is_safe_table_select(select)
}

pub(super) fn is_safe_count(select: &Select) -> bool {
    is_safe_unpaged_projection(select, |projection| {
        matches!(projection, Projection::Count | Projection::CountColumn { .. })
    })
}

pub(super) fn is_safe_uniq(select: &Select) -> bool {
    is_safe_unpaged_projection(select, |projection| {
        matches!(projection, Projection::ApproxDistinct { .. })
    })
}

/// Build a shard-local HLL state, not a shard-local estimate. Merging local
/// estimates cannot recover the global union when shard keys overlap.
pub(super) fn uniq_state_shard_sql(sql: &str, select: &Select) -> Option<String> {
    let column = match (&select.projection, &select.source) {
        (Projection::ApproxDistinct { column }, Some(Source::Table(_))) => column,
        _ => return None,
    };
    if !is_safe_uniq(select) {
        return None;
    }
    let start = sql.to_ascii_uppercase().find(" FROM ")?;
    let rewritten = format!("SELECT uniqState({column}){}", &sql[start..]);
    let mut expected = select.clone();
    expected.projection = Projection::ApproxDistinctState { column: column.clone() };
    expected.alias = None;
    (parse_select(&rewritten).ok()? == expected).then_some(rewritten)
}

pub(super) fn is_safe_count_distinct(select: &Select) -> bool {
    is_safe_unpaged_projection(select, |projection| {
        matches!(projection, Projection::CountDistinct { .. })
    })
}

/// Rebuild only the projection while keeping an exactly verified WHERE clause.
/// A failed round trip must not accidentally turn a filtered count into a full scan.
pub(super) fn distinct_shard_sql(sql: &str, select: &Select) -> Option<String> {
    let (column, table) = match (&select.projection, &select.source) {
        (Projection::CountDistinct { column }, Some(Source::Table(table))) => (column, table),
        _ => return None,
    };
    let mut rewritten = format!("SELECT DISTINCT {column} FROM {table}");
    let upper = sql.to_ascii_uppercase();
    if let Some(where_start) = upper.find(" WHERE ") {
        rewritten.push_str(&sql[where_start..]);
    } else if !select.predicates.is_empty() || !select.or_predicates.is_empty() {
        return None;
    }
    let mut expected = select.clone();
    expected.projection = Projection::Columns(vec![column.to_owned()]);
    expected.distinct = true;
    (parse_select(&rewritten).ok()? == expected).then_some(rewritten)
}

pub(super) fn is_safe_sum(select: &Select) -> bool {
    is_safe_unpaged_projection(select, |projection| {
        matches!(
            projection,
            Projection::Aggregate {
                function: AggregateFunction::Sum,
                ..
            }
        )
    })
}

pub(super) fn is_safe_count_if(select: &Select) -> bool {
    is_safe_unpaged_projection(select, |projection| {
        matches!(
            projection,
            Projection::Aggregate {
                function: AggregateFunction::CountIf,
                ..
            }
        )
    })
}

pub(super) fn is_safe_extremum(select: &Select) -> bool {
    is_safe_unpaged_projection(select, |projection| {
        matches!(
            projection,
            Projection::Aggregate {
                function: AggregateFunction::Min | AggregateFunction::Max,
                ..
            }
        )
    })
}

fn base_type(kind: &str) -> &str {
    kind.strip_prefix("Nullable(")
        .and_then(|inner| inner.strip_suffix(')'))
        .unwrap_or(kind)
}

pub(super) fn integer_type(kind: &str) -> bool {
    let base = base_type(kind);
    matches!(
        base,
        "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UInt128"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "Int128"
    )
}

pub(super) fn is_safe_grouped_count(select: &Select) -> bool {
    matches!(select.projection, Projection::GroupedCount { .. })
        && matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.rename.is_none()
        && select.alias.is_none()
        // The coordinator can rank merged counts, but not local shard keys.
        && select.order_terms().all(|term| term.column == "count")
}

pub(super) fn is_safe_grouped_sum(select: &Select) -> bool {
    matches!(
        select.projection,
        Projection::GroupedAggregate {
            function: AggregateFunction::Sum,
            ..
        }
    )
        && matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.rename.is_none()
        && select.alias.is_none()
        // The coordinator sorts merged SUMs, not shard-local partials.
        && select.order_terms().all(|order| order.column == "#aggregate")
        && select.having.as_ref().is_none_or(|having| matches!(
            having.operator,
            rusthouse_query::Operator::Eq | rusthouse_query::Operator::Ne
                | rusthouse_query::Operator::Lt | rusthouse_query::Operator::Le
                | rusthouse_query::Operator::Gt | rusthouse_query::Operator::Ge
        ))
}

pub(super) fn group_key_type(kind: &str) -> bool {
    let base = base_type(kind);
    integer_type(kind) || matches!(base, "String" | "UUID")
}

pub(super) fn unpaged_scan_sql(sql: &str, select: &Select) -> Option<String> {
    if select.limit.is_none() && select.offset.is_none() {
        return Some(sql.to_owned());
    }
    let mut expected = select.clone();
    expected.limit = None;
    expected.offset = None;
    for (index, _) in sql.char_indices() {
        let suffix = &sql[index..];
        let keyword = ["LIMIT", "OFFSET"].iter().any(|keyword| {
            suffix
                .get(..keyword.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(keyword))
                && suffix
                    .as_bytes()
                    .get(keyword.len())
                    .is_some_and(u8::is_ascii_whitespace)
        });
        if !keyword || (index > 0 && !sql.as_bytes()[index - 1].is_ascii_whitespace()) {
            continue;
        }
        let candidate = sql[..index].trim_end();
        if parse_select(candidate).is_ok_and(|parsed| parsed == expected) {
            return Some(candidate.to_owned());
        }
    }
    None
}

 
/// Ordinary un-ordered scans need at most OFFSET + LIMIT rows from any
/// one shard. Keep the global OFFSET at the coordinator, and reparse the
/// rewritten query before it can be dispatched.
pub(super) fn bounded_scan_sql(sql: &str, select: &Select) -> Option<String> {
    let bare = unpaged_scan_sql(sql, select)?;
    let Some(limit) = select.limit else {
        return Some(bare);
    };
    let prefix = select.offset.unwrap_or(0).checked_add(limit)?;
    let candidate = format!("{bare} LIMIT {prefix}");
    let mut expected = select.clone();
    expected.limit = Some(prefix);
    expected.offset = None;
    (parse_select(&candidate).ok()? == expected).then_some(candidate)
}

#[cfg(test)]
mod bounded_scan_rewrite_tests {
    use super::*;

    #[test]
    fn pushes_only_verified_prefixes_and_preserves_quoted_keywords() {
        for (sql, expected) in [
            ("SELECT id FROM events LIMIT 5", "SELECT id FROM events LIMIT 5"),
            ("SELECT id FROM events LIMIT 5 OFFSET 7", "SELECT id FROM events LIMIT 12"),
            ("SELECT name FROM events WHERE name = 'LIMIT 9' LIMIT 2 OFFSET 3",
             "SELECT name FROM events WHERE name = 'LIMIT 9' LIMIT 5"),
        ] {
            let select = parse_select(sql).unwrap();
            assert_eq!(bounded_scan_sql(sql, &select).as_deref(), Some(expected));
        }
        let unbounded = "SELECT id FROM events OFFSET 5";
        let select = parse_select(unbounded).unwrap();
        assert_eq!(bounded_scan_sql(unbounded, &select).as_deref(),
            Some("SELECT id FROM events"));
    }
}

/// Remove global HAVING, ORDER BY and pagination before per-shard grouping.
/// Require a full parser round trip to avoid changing WHERE string literals.
pub(super) fn grouped_count_shard_sql(sql: &str, select: &Select) -> Option<String> {
    let mut expected = select.clone();
    expected.having = None;
    expected.order_by = None;
    expected.order_by_extra.clear();
    expected.limit = None;
    expected.offset = None;
    if expected == *select {
        return Some(sql.to_owned());
    }
    for (index, _) in sql.char_indices() {
        if index == 0 || !sql.as_bytes()[index - 1].is_ascii_whitespace() {
            continue;
        }
        let suffix = &sql[index..];
        let boundary = ["HAVING", "ORDER", "LIMIT", "OFFSET"].iter().any(|word| {
            suffix
                .get(..word.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(word))
                && suffix
                    .as_bytes()
                    .get(word.len())
                    .is_some_and(u8::is_ascii_whitespace)
        });
        if !boundary {
            continue;
        }
        let candidate = sql[..index].trim_end();
        if parse_select(candidate).is_ok_and(|parsed| parsed == expected) {
            return Some(candidate.to_owned());
        }
    }
    None
}

/// Exact distributed top-K currently accepts a single unaliased integer
/// projection and ORDER BY that same column. Validate integer types against
/// each peer's schema before dispatch.
pub(super) fn is_safe_sorted_scan(select: &Select) -> bool {
    let (Projection::Columns(columns), Some(order), Some(limit)) =
        (&select.projection, &select.order_by, select.limit)
    else { return false; };
    matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.rename.is_none()
        && select.alias.is_none()
        && select.having.is_none()
        && select.order_by_extra.is_empty()
        && order.nulls_first.is_none()
        && columns.len() == 1
        && columns[0] == order.column
        && select.offset.unwrap_or(0)
            .checked_add(limit)
            .is_some_and(|prefix| prefix <= 10_000)
}

pub(super) fn sorted_scan_sql(sql: &str, select: &Select) -> Option<String> {
    if !is_safe_sorted_scan(select) { return None; }
    let prefix = select.offset.unwrap_or(0).checked_add(select.limit?)?;
    let bare = unpaged_scan_sql(sql, select)?;
    let rewritten = format!("{bare} LIMIT {prefix}");
    let mut expected = select.clone();
    expected.offset = None;
    expected.limit = Some(prefix);
    (parse_select(&rewritten).ok()? == expected).then_some(rewritten)
}

#[cfg(test)]
mod sorted_scan_policy_tests {
    use super::*;

    #[test]
    fn rewrites_validated_ordered_global_pagination() {
        let sql = "SELECT id FROM events WHERE id > 4 ORDER BY id DESC LIMIT 3 OFFSET 2";
        let parsed = parse_select(sql).unwrap();
        assert!(is_safe_sorted_scan(&parsed));
        assert_eq!(
            sorted_scan_sql(sql, &parsed).as_deref(),
            Some("SELECT id FROM events WHERE id > 4 ORDER BY id DESC LIMIT 5")
        );
        assert!(!is_safe_sorted_scan(
            &parse_select("SELECT id, name FROM events ORDER BY id LIMIT 3").unwrap()
        ));
    }
}
