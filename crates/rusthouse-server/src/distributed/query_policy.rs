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
    matches!(select.projection, Projection::GroupedCount { .. }) && is_safe_unpaged_select(select)
}

pub(super) fn is_safe_grouped_sum(select: &Select) -> bool {
    matches!(
        select.projection,
        Projection::GroupedAggregate {
            function: AggregateFunction::Sum,
            ..
        }
    ) && is_safe_unpaged_select(select)
}

pub(super) fn group_key_type(kind: &str) -> bool {
    let base = kind
        .strip_prefix("Nullable(")
        .and_then(|inner| inner.strip_suffix(')'))
        .unwrap_or(kind);
    integer_type(kind) || matches!(base, "String" | "UUID")
}


/// A global unordered OFFSET + LIMIT page depends on at most OFFSET + LIMIT
/// rows from each shard. Reparse the rewritten SQL to preserve literals.
pub(super) fn bounded_scan_sql(sql: &str, select: &Select, end: usize) -> Option<String> {
    let unpaged = unpaged_scan_sql(sql, select)?;
    let bounded = format!("{unpaged} LIMIT {end}");
    let mut expected = select.clone();
    expected.limit = Some(end);
    expected.offset = None;
    (parse_select(&bounded).ok()? == expected).then_some(bounded)
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
