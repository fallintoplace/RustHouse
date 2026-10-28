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

pub(super) fn is_safe_scan(select: &Select) -> bool {
    matches!(select.projection, Projection::All | Projection::Columns(_))
        && is_safe_table_select(select)
}

pub(super) fn is_safe_count(select: &Select) -> bool {
    matches!(
        select.projection,
        Projection::Count | Projection::CountColumn { .. }
    ) && is_safe_unpaged_select(select)
}

pub(super) fn is_safe_count_distinct(select: &Select) -> bool {
    matches!(select.projection, Projection::CountDistinct { .. }) && is_safe_unpaged_select(select)
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
    matches!(
        select.projection,
        Projection::Aggregate {
            function: AggregateFunction::Sum,
            ..
        }
    ) && is_safe_unpaged_select(select)
}

pub(super) fn is_safe_count_if(select: &Select) -> bool {
    matches!(
        select.projection,
        Projection::Aggregate {
            function: AggregateFunction::CountIf,
            ..
        }
    ) && is_safe_unpaged_select(select)
}

pub(super) fn is_safe_extremum(select: &Select) -> bool {
    matches!(
        select.projection,
        Projection::Aggregate {
            function: AggregateFunction::Min | AggregateFunction::Max,
            ..
        }
    ) && is_safe_unpaged_select(select)
}

pub(super) fn integer_type(kind: &str) -> bool {
    let base = kind
        .strip_prefix("Nullable(")
        .and_then(|inner| inner.strip_suffix(')'))
        .unwrap_or(kind);
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
    matches!(select.projection, Projection::GroupedCount { .. } | Projection::GroupedMultiCount { .. })
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
    ) && matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.rename.is_none()
        && select.alias.is_none()
        && select.order_terms().all(|term| term.column == "#aggregate")
}

pub(super) fn group_key_type(kind: &str) -> bool {
    let base = kind
        .strip_prefix("Nullable(")
        .and_then(|inner| inner.strip_suffix(')'))
        .unwrap_or(kind);
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

/// Strip coordinator-only clauses before shard-level grouped numeric aggregation.
/// The candidate must parse to an identical query minus the global clauses.
pub(super) fn grouped_aggregate_shard_sql(sql: &str, select: &Select) -> Option<String> {
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
        if boundary {
            let candidate = sql[..index].trim_end();
            if parse_select(candidate).is_ok_and(|parsed| parsed == expected) {
                return Some(candidate.to_owned());
            }
        }
    }
    None
}

/// Allow grouped MIN/MAX of exact integers with global aggregate ordering.
pub(super) fn is_safe_grouped_extremum(select: &Select) -> bool {
    matches!(
        select.projection,
        Projection::GroupedAggregate {
            function: AggregateFunction::Min | AggregateFunction::Max,
            ..
        }
    ) && matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.rename.is_none()
        && select.alias.is_none()
        && select.order_terms().all(|term| term.column == "#aggregate")
}

/// A single exact SUM over several dimensions reduces by composite key.
pub(super) fn is_safe_grouped_multikey_sum(select: &Select) -> bool {
    matches!(
        &select.projection,
        Projection::GroupedMultiAggregates { columns, items }
        if (2..=8).contains(&columns.len())
            && items.len() == 1
            && items[0].0 == AggregateFunction::Sum
    ) && is_safe_unpaged_select(select)
}
