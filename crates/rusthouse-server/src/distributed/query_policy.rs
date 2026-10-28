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
        && match &select.projection {
            Projection::GroupedCount { column } => {
                select.order_terms().all(|term| term.column == "count" || term.column == *column)
            }
            Projection::GroupedMultiCount { columns } => {
                select.order_terms().all(|term| {
                    term.column == "count" || columns.contains(&term.column)
                })
            }
            _ => false,
        }
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
        && match &select.projection {
            Projection::GroupedAggregate { column, .. } => {
                select.order_terms().all(|term| {
                    term.column == "#aggregate" || term.column == *column
                })
            }
            _ => false,
        }
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
        && match &select.projection {
            Projection::GroupedAggregate { column, .. } => {
                select.order_terms().all(|term| {
                    term.column == "#aggregate" || term.column == *column
                })
            }
            _ => false,
        }
}

/// A single exact SUM over several dimensions reduces by composite key.
pub(super) fn is_safe_grouped_multikey_sum(select: &Select) -> bool {
    matches!(
        &select.projection,
        Projection::GroupedMultiAggregates { columns, items }
        if (2..=8).contains(&columns.len())
            && items.len() == 1
            && items[0].0 == AggregateFunction::Sum
            && select.order_terms().all(|term| columns.contains(&term.column))
    ) && matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.rename.is_none()
        && select.alias.is_none()
        && select.having.is_none()
}

/// Multiple dimensions with one exact integer extremum may reduce per composite key.
pub(super) fn is_safe_grouped_multikey_extremum(select: &Select) -> bool {
    matches!(
        &select.projection,
        Projection::GroupedMultiAggregates { columns, items }
        if (2..=8).contains(&columns.len())
            && items.len() == 1
            && matches!(items[0].0, AggregateFunction::Min | AggregateFunction::Max)
            && select.order_terms().all(|term| columns.contains(&term.column))
    ) && matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.rename.is_none()
        && select.alias.is_none()
        && select.having.is_none()
}

/// Fused exact integer aggregates are decomposable across disjoint shards.
pub(super) fn is_safe_grouped_fused_metrics(select: &Select) -> bool {
    let items = match &select.projection {
        Projection::GroupedAggregates { items, .. }
        | Projection::GroupedMultiAggregates { items, .. } => items,
        _ => return false,
    };
    (2..=16).contains(&items.len())
        && items.iter().all(|(function, _)| matches!(
            function,
            AggregateFunction::Count
                | AggregateFunction::Sum
                | AggregateFunction::Min
                | AggregateFunction::Max
        ))
        && is_safe_unpaged_select(select)
}

/// Exact grouped distinct requires shipping unique (group, value) pairs,
/// never summing independent shard-level distinct counts.
pub(super) fn is_safe_grouped_distinct(select: &Select) -> bool {
    matches!(select.projection, Projection::GroupedDistinctCount { .. })
        && matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.alias.is_none()
        && select.rename.is_none()
        && match &select.projection {
            Projection::GroupedDistinctCount { column, .. } =>
                select.order_terms().all(|term| term.column == "count" || term.column == *column),
            _ => false,
        }
}

/// Build a DISTINCT pair projection while preserving a fully parsed WHERE.
/// WHERE text is retained only if the candidate AST matches the expected AST.
pub(super) fn grouped_distinct_shard_sql(sql: &str, select: &Select) -> Option<String> {
    let (column, value_column) = match &select.projection {
        Projection::GroupedDistinctCount { column, value_column } => (column, value_column),
        _ => return None,
    };
    let mut expected = select.clone();
    expected.projection = Projection::Columns(vec![column.clone(), value_column.clone()]);
    expected.distinct = true;
    expected.having = None;
    expected.order_by = None;
    expected.order_by_extra.clear();
    expected.limit = None;
    expected.offset = None;
    let upper = sql.to_ascii_uppercase();
    let from_start = upper.find(" FROM ")?;
    for (group_start, _) in sql.char_indices() {
        if group_start <= from_start || !upper[group_start..].starts_with(" GROUP BY ") {
            continue;
        }
        let mut candidate = format!("SELECT DISTINCT {column}, {value_column}");
        candidate.push_str(&sql[from_start..group_start]);
        if parse_select(&candidate).is_ok_and(|parsed| parsed == expected) {
            return Some(candidate);
        }
    }
    None
}

/// AVG is decomposable only as SUM(value) and COUNT(value), not shard AVG.
pub(super) fn is_safe_weighted_avg(select: &Select) -> bool {
    matches!(
        select.projection,
        Projection::Aggregate { function: AggregateFunction::Avg, .. }
    ) && is_safe_unpaged_select(select)
        && select.alias.is_none()
        && select.rename.is_none()
}

/// Rewrite an AVG into partial SUM and COUNT only after re-parsing exact SQL.
pub(super) fn weighted_avg_shard_sql(sql: &str, select: &Select) -> Option<String> {
    let column = match &select.projection {
        Projection::Aggregate { function: AggregateFunction::Avg, column } => column,
        _ => return None,
    };
    let mut expected = select.clone();
    expected.projection = Projection::MultipleAggregates(vec![
        AggregateItem {
            function: Some(AggregateFunction::Sum),
            argument: Some(ScalarExpr::Column(column.clone())),
            distinct: false,
            alias: None,
        },
        AggregateItem {
            function: None,
            argument: Some(ScalarExpr::Column(column.clone())),
            distinct: false,
            alias: None,
        },
    ]);
    let upper = sql.to_ascii_uppercase();
    for (index, _) in sql.char_indices() {
        if !upper[index..].starts_with(" FROM ") {
            continue;
        }
        let candidate = format!(
            "SELECT sum({column}), count({column}){}", &sql[index..],
        );
        if parse_select(&candidate).is_ok_and(|parsed| parsed == expected) {
            return Some(candidate);
        }
    }
    None
}

/// A grouped AVG must reduce per-group SUM and non-NULL COUNT separately.
pub(super) fn is_safe_grouped_weighted_avg(select: &Select) -> bool {
    matches!(
        select.projection,
        Projection::GroupedAggregate { function: AggregateFunction::Avg, .. }
    ) && matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.alias.is_none()
        && select.rename.is_none()
        && match &select.projection {
            Projection::GroupedAggregate { column, .. } =>
                select.order_terms().all(|term|
                    term.column == "#aggregate" || term.column == *column),
            _ => false,
        }
}

/// Generate the two partial group metrics, preserving only the original
/// FROM/WHERE/GROUP BY semantics after a full AST equivalence check.
pub(super) fn grouped_weighted_avg_shard_sql(sql: &str, select: &Select) -> Option<String> {
    let (key, value) = match &select.projection {
        Projection::GroupedAggregate {
            function: AggregateFunction::Avg,
            column,
            value_column,
        } => (column, value_column),
        _ => return None,
    };
    let mut expected = select.clone();
    expected.projection = Projection::GroupedAggregates {
        column: key.clone(),
        items: vec![
            (AggregateFunction::Sum, value.clone()),
            (AggregateFunction::Count, value.clone()),
        ],
    };
    expected.having = None;
    expected.order_by = None;
    expected.order_by_extra.clear();
    expected.limit = None;
    expected.offset = None;
    let upper = sql.to_ascii_uppercase();
    for (from_start, _) in sql.char_indices() {
        if !upper[from_start..].starts_with(" FROM ") {
            continue;
        }
        for (end, _) in sql.char_indices() {
            if end < from_start { continue; }
            let suffix = &upper[end..];
            if end != sql.len() && ![" HAVING ", " ORDER ", " LIMIT ", " OFFSET "]
                .iter().any(|keyword| suffix.starts_with(keyword)) {
                continue;
            }
            let candidate = format!(
                "SELECT {key}, sum({value}), count({value}){}",
                &sql[from_start..end],
            );
            if parse_select(&candidate).is_ok_and(|parsed| parsed == expected) {
                return Some(candidate);
            }
        }
        let candidate = format!(
            "SELECT {key}, sum({value}), count({value}){}",
            &sql[from_start..],
        );
        if parse_select(&candidate).is_ok_and(|parsed| parsed == expected) {
            return Some(candidate);
        }
    }
    None
}


/// Two or more grouping dimensions with one AVG metric use exact integer
/// partial SUM and COUNT, never a mean of shard-local means.
pub(super) fn is_safe_multikey_weighted_avg(select: &Select) -> bool {
    matches!(
        &select.projection,
        Projection::GroupedMultiAggregates { columns, items }
            if (2..=8).contains(&columns.len())
            && items.len() == 1
            && items[0].0 == AggregateFunction::Avg
            && select.order_terms().all(|term| columns.contains(&term.column))
    ) && matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.alias.is_none()
        && select.rename.is_none()
        && select.having.is_none()
}

/// Require full semantic equivalence after replacing AVG with SUM and COUNT.
pub(super) fn multikey_weighted_avg_shard_sql(
    sql: &str,
    select: &Select,
) -> Option<String> {
    let (columns, value) = match &select.projection {
        Projection::GroupedMultiAggregates { columns, items }
            if items.len() == 1 && items[0].0 == AggregateFunction::Avg =>
            (columns, &items[0].1),
        _ => return None,
    };
    let mut expected = select.clone();
    expected.projection = Projection::GroupedMultiAggregates {
        columns: columns.clone(),
        items: vec![
            (AggregateFunction::Sum, value.clone()),
            (AggregateFunction::Count, value.clone()),
        ],
    };
    expected.having = None;
    expected.order_by = None;
    expected.order_by_extra.clear();
    expected.limit = None;
    expected.offset = None;
    let upper = sql.to_ascii_uppercase();
    let head = format!(
        "SELECT {}, sum({value}), count({value})",
        columns.join(", "),
    );
    for (from_start, _) in sql.char_indices() {
        if !upper[from_start..].starts_with(" FROM ") {
            continue;
        }
        for (end, _) in sql.char_indices() {
            if end < from_start { continue; }
            let suffix = &upper[end..];
            if ![" ORDER ", " LIMIT ", " OFFSET "]
                .iter().any(|keyword| suffix.starts_with(keyword)) {
                continue;
            }
            let candidate = format!("{head}{}", &sql[from_start..end]);
            if parse_select(&candidate).is_ok_and(|parsed| parsed == expected) {
                return Some(candidate);
            }
        }
        let candidate = format!("{head}{}", &sql[from_start..]);
        if parse_select(&candidate).is_ok_and(|parsed| parsed == expected) {
            return Some(candidate);
        }
    }
    None
}

/// COUNT(column) ignores NULL, unlike COUNT(*). The integer partial counts
/// remain exactly additive across disjoint shard partitions.
pub(super) fn is_safe_grouped_nonnull_count(select: &Select) -> bool {
    matches!(
        select.projection,
        Projection::GroupedAggregate {
            function: AggregateFunction::Count,
            ..
        }
    ) && matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.alias.is_none()
        && select.rename.is_none()
        && match &select.projection {
            Projection::GroupedAggregate { column, .. } =>
                select.order_terms().all(|term| {
                    term.column == "#aggregate" || term.column == *column
                }),
            _ => false,
        }
}
