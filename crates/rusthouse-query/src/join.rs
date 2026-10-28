//! Bounded two-table equi-join syntax for persistent SQL tables.

mod syntax;

// Keep explained execution bounds and the executor's actual limits in sync.
pub(crate) const MAX_BUFFERED_JOIN_ROWS: u64 = 32_768;
pub(crate) const MAX_STREAM_JOIN_BUILD_ROWS: u64 = 262_144;
pub(crate) const MAX_JOIN_RESULT_ROWS: usize = 131_072;
pub(crate) const MAX_JOIN_BUILD_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_JOIN_CHAIN_BYTES: usize = 64 * 1024 * 1024;

use self::syntax::*;

use super::{Having, ParseError, Parser, Token, tokenize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinColumn {
    pub relation: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinStep {
    pub table: String,
    pub alias: String,
    pub kind: JoinType,
    pub any: bool,
    pub keys: Vec<(JoinColumn, JoinColumn)>,
    pub using_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinAggregate {
    CountStar,
    CountDistinct(JoinColumn),
    Sum(JoinColumn),
    GroupCount(JoinColumn),
    GroupMultiCount(Vec<JoinColumn>),
    GroupMultiSum { keys: Vec<JoinColumn>, value: JoinColumn },
    GroupSumCount { keys: Vec<JoinColumn>, value: JoinColumn },
    GroupSum { group: JoinColumn, value: JoinColumn },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinSelect {
    pub left: String,
    pub right: String,
    pub left_alias: String,
    pub right_alias: String,
    pub kind: JoinType,
    /// Select one right row per left row for INNER/LEFT, or one left row per right for RIGHT.
    pub any: bool,
    /// None projects every column from both tables.
    pub columns: Option<Vec<JoinColumn>>,
    pub aggregate: Option<JoinAggregate>,
    pub keys: Vec<(JoinColumn, JoinColumn)>,
    /// Shared-key names for JOIN USING; star output projects each key once.
    pub using_keys: Vec<String>,
    /// Optional post-join NULL filter (`true` means `IS NOT NULL`).
    pub where_null: Option<(JoinColumn, bool)>,
    /// Ordered key, descending flag, and optional explicit NULL placement.
    pub order_by: Vec<(JoinColumn, bool, Option<bool>)>,
    /// Aggregate ORDER BY for grouped joined facts.
    pub group_order_by: Option<(bool, Option<bool>)>,
    pub group_having: Option<Having>,
    pub limit: Option<usize>,
    pub offset: usize,
    /// Additional JOIN steps after the first two-table join.
    pub steps: Vec<JoinStep>,
}

/// Return None for queries without a JOIN keyword after FROM.
pub fn parse_join(sql: &str) -> Result<Option<JoinSelect>, ParseError> {
    let tokens = tokenize(sql)?;
    if !matches!(tokens.first(), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT")) {
        return Ok(None);
    }
    let Some(from) = tokens
        .iter()
        .position(|token| matches!(token, Token::Word(word) if word.eq_ignore_ascii_case("FROM")))
    else {
        return Ok(None);
    };
    if !tokens
        .iter()
        .skip(from + 1)
        .any(|token| matches!(token, Token::Word(word) if word.eq_ignore_ascii_case("JOIN")))
    {
        return Ok(None);
    }
    let mut parser = Parser { tokens, index: 0 };
    parser.keyword("SELECT");
    let aggregate = if parser.keyword("COUNT") {
        if !parser.take(Token::LParen) {
            return Err(ParseError("JOIN COUNT requires '('".into()));
        }
        let parsed = if parser.take(Token::Star) {
            JoinAggregate::CountStar
        } else if parser.keyword("DISTINCT") {
            JoinAggregate::CountDistinct(qualified(&mut parser)?)
        } else {
            return Err(ParseError("JOIN COUNT requires * or DISTINCT relation.column".into()));
        };
        if !parser.take(Token::RParen) {
            return Err(ParseError("JOIN COUNT requires ')'".into()));
        }
        Some(parsed)
    } else if parser.keyword("SUM") {
        if !parser.take(Token::LParen) {
            return Err(ParseError("JOIN SUM requires SUM(alias.column)".into()));
        }
        let column = qualified(&mut parser)?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("JOIN SUM requires a closing ')'".into()));
        }
        Some(JoinAggregate::Sum(column))
    } else {
        let saved = parser.index;
        let grouped = if let Ok(first) = qualified(&mut parser) {
            let mut keys = vec![first];
            let mut result = None;
            while parser.take(Token::Comma) {
                if parser.keyword("COUNT") && parser.take(Token::LParen)
                    && parser.take(Token::Star) && parser.take(Token::RParen)
                {
                    result = Some(if keys.len() == 1 {
                        JoinAggregate::GroupCount(keys.remove(0))
                    } else {
                        JoinAggregate::GroupMultiCount(keys)
                    });
                    break;
                }
                if parser.keyword("SUM") && parser.take(Token::LParen) {
                    let value = qualified(&mut parser)?;
                    if !parser.take(Token::RParen) {
                        return Err(ParseError("JOIN grouped SUM requires ')'".into()));
                    }
                    if parser.take(Token::Comma) {
                        if !parser.keyword("COUNT") || !parser.take(Token::LParen)
                            || !parser.take(Token::Star) || !parser.take(Token::RParen) {
                            return Err(ParseError("JOIN fused SUM requires trailing COUNT(*)".into()));
                        }
                        result = Some(JoinAggregate::GroupSumCount { keys, value });
                    } else {
                        result = Some(if keys.len() == 1 {
                            JoinAggregate::GroupSum { group: keys.remove(0), value }
                        } else {
                            JoinAggregate::GroupMultiSum { keys, value }
                        });
                    }
                    break;
                }
                if keys.len() == 8 {
                    return Err(ParseError("JOIN GROUP BY supports at most eight dimensions".into()));
                }
                let Ok(next) = qualified(&mut parser) else { break };
                keys.push(next);
            }
            result
        } else { None };
        if grouped.is_none() { parser.index = saved; }
        grouped
    };
    let columns = if aggregate.is_some() || parser.take(Token::Star) {
        None
    } else {
        let mut columns = Vec::new();
        loop {
            if columns.len() == 64 {
                return Err(ParseError(
                    "JOIN supports at most 64 projected columns".into(),
                ));
            }
            columns.push(projected(&mut parser)?);
            if !parser.take(Token::Comma) {
                break;
            }
        }
        Some(columns)
    };
    if !parser.keyword("FROM") {
        return Err(ParseError("JOIN requires FROM".into()));
    }
    let left = parser.word()?;
    let left_alias = alias(&mut parser, &left)?;
    if left.len() > 128 || left_alias.len() > 128 {
        return Err(ParseError(
            "table identifiers cannot exceed 128 bytes".into(),
        ));
    }
    let leading_strictness = if parser.keyword("ANY") {
        Some(true)
    } else if parser.keyword("ALL") {
        Some(false)
    } else {
        None
    };
    let cross = parser.keyword("CROSS");
    let kind = if cross {
        JoinType::Inner
    } else {
        parse_join_kind(&mut parser)
    };
    let trailing_strictness = if parser.keyword("ANY") {
        Some(true)
    } else if parser.keyword("ALL") {
        Some(false)
    } else {
        None
    };
    if leading_strictness.is_some() && trailing_strictness.is_some() {
        return Err(ParseError("JOIN strictness specified twice".into()));
    }
    let any = leading_strictness.or(trailing_strictness).unwrap_or(false);
    if cross && (leading_strictness.is_some() || trailing_strictness.is_some()) {
        return Err(ParseError("CROSS JOIN does not accept ANY or ALL".into()));
    }
    if any && !matches!(kind, JoinType::Inner | JoinType::Left | JoinType::Right) {
        return Err(ParseError(
            "ANY JOIN supports only INNER, LEFT and RIGHT joins".into(),
        ));
    }
    if !parser.keyword("JOIN") {
        return Err(ParseError("expected JOIN after join type".into()));
    }
    let right = parser.word()?;
    let right_alias = alias(&mut parser, &right)?;
    if right.len() > 128 || right_alias.len() > 128 {
        return Err(ParseError(
            "table identifiers cannot exceed 128 bytes".into(),
        ));
    }
    if left_alias == right_alias {
        return Err(ParseError("joined tables require distinct aliases".into()));
    }
    let (keys, using_keys) = if cross {
        (Vec::new(), Vec::new())
    } else if parser.keyword("USING") {
        if !parser.take(Token::LParen) {
            return Err(ParseError("JOIN USING requires '('".into()));
        }
        let mut keys = Vec::new();
        let mut names = Vec::new();
        loop {
            if names.len() >= 8 {
                return Err(ParseError("JOIN USING supports at most 8 keys".into()));
            }
            let name = parser.word()?;
            if names.contains(&name) {
                return Err(ParseError("duplicate JOIN USING column".into()));
            }
            keys.push((
                JoinColumn {
                    relation: left_alias.clone(),
                    name: name.clone(),
                },
                JoinColumn {
                    relation: right_alias.clone(),
                    name: name.clone(),
                },
            ));
            names.push(name);
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("JOIN USING requires ')'".into()));
        }
        (keys, names)
    } else if parser.keyword("ON") {
        let mut keys = Vec::new();
        equality_group(&mut parser, &mut keys, 0)?;
        let mut unique: Vec<(JoinColumn, JoinColumn)> = Vec::with_capacity(keys.len());
        for (first, second) in keys {
            if !unique.iter().any(|(left, right)| {
                (left == &first && right == &second) || (left == &second && right == &first)
            }) {
                unique.push((first, second));
            }
        }
        if unique.len() > 8 {
            return Err(ParseError("JOIN supports at most 8 equality keys".into()));
        }
        (unique, Vec::new())
    } else {
        return Err(ParseError("JOIN requires ON or USING".into()));
    };
    for (first, second) in &keys {
        for column in [first, second] {
            if column.relation != left_alias && column.relation != right_alias {
                return Err(ParseError(format!(
                    "unknown JOIN qualifier: {}",
                    column.relation
                )));
            }
        }
        if first.relation == second.relation {
            return Err(ParseError(
                "JOIN ON must compare one column from each table".into(),
            ));
        }
    }
    let mut steps: Vec<JoinStep> = Vec::new();
    while matches!(
        parser.tokens.get(parser.index),
        Some(Token::Word(word))
            if ["JOIN", "INNER", "LEFT", "RIGHT", "FULL", "CROSS", "ANY", "ALL"]
                .iter()
                .any(|keyword| word.eq_ignore_ascii_case(keyword))
    ) {
        if steps.len() >= 6 {
            return Err(ParseError("JOIN supports at most 8 tables".into()));
        }
        let leading_any = parser.keyword("ANY");
        let leading_all = !leading_any && parser.keyword("ALL");
        let step_cross = parser.keyword("CROSS");
        let step_kind = if step_cross {
            JoinType::Inner
        } else {
            parse_join_kind(&mut parser)
        };
        let trailing_any = parser.keyword("ANY");
        let trailing_all = !trailing_any && parser.keyword("ALL");
        if (leading_any || leading_all) && (trailing_any || trailing_all) {
            return Err(ParseError("JOIN strictness specified twice".into()));
        }
        let step_any = leading_any || trailing_any;
        if step_cross && (leading_any || leading_all || trailing_any || trailing_all) {
            return Err(ParseError("CROSS JOIN does not accept ANY or ALL".into()));
        }
        if !parser.keyword("JOIN") {
            return Err(ParseError("expected JOIN after join type".into()));
        }
        let table = parser.word()?;
        let alias = alias(&mut parser, &table)?;
        if table.len() > 128 || alias.len() > 128 {
            return Err(ParseError(
                "table identifiers cannot exceed 128 bytes".into(),
            ));
        }
        if alias == left_alias
            || alias == right_alias
            || steps.iter().any(|step| step.alias == alias)
        {
            return Err(ParseError("joined tables require distinct aliases".into()));
        }
        if (step_any
            && !matches!(
                step_kind,
                JoinType::Inner | JoinType::Left | JoinType::Right
            ))
            || (any && !matches!(kind, JoinType::Inner | JoinType::Left | JoinType::Right))
        {
            return Err(ParseError(
                "chained ANY JOIN supports only INNER, LEFT and RIGHT joins".into(),
            ));
        }
        let (step_keys, step_using) = if step_cross {
            (Vec::new(), Vec::new())
        } else if parser.keyword("USING") {
            if !parser.take(Token::LParen) {
                return Err(ParseError("JOIN USING requires '('".into()));
            }
            let mut names = Vec::new();
            loop {
                if names.len() >= 8 {
                    return Err(ParseError("JOIN USING supports at most 8 keys".into()));
                }
                let name = parser.word()?;
                if names.contains(&name) {
                    return Err(ParseError("duplicate JOIN USING column".into()));
                }
                names.push(name);
                if !parser.take(Token::Comma) {
                    break;
                }
            }
            if !parser.take(Token::RParen) {
                return Err(ParseError("JOIN USING requires ')'".into()));
            }
            (Vec::new(), names)
        } else if parser.keyword("ON") {
            let mut raw = Vec::new();
            equality_group(&mut parser, &mut raw, 0)?;
            let mut keys: Vec<(JoinColumn, JoinColumn)> = Vec::new();
            for (first, second) in raw {
                for column in [&first, &second] {
                    if column.relation != left_alias
                        && column.relation != right_alias
                        && column.relation != alias
                        && !steps.iter().any(|step| step.alias == column.relation)
                    {
                        return Err(ParseError(format!(
                            "unknown JOIN qualifier: {}",
                            column.relation
                        )));
                    }
                }
                let first_is_new = first.relation == alias;
                let second_is_new = second.relation == alias;
                if first_is_new == second_is_new {
                    return Err(ParseError(
                        "JOIN ON must compare an existing relation to the new table".into(),
                    ));
                }
                if !keys.iter().any(|(left, right)| {
                    (left == &first && right == &second) || (left == &second && right == &first)
                }) {
                    keys.push((first, second));
                }
            }
            if keys.len() > 8 {
                return Err(ParseError("JOIN supports at most 8 equality keys".into()));
            }
            (keys, Vec::new())
        } else {
            return Err(ParseError("JOIN requires ON or USING".into()));
        };
        steps.push(JoinStep {
            table,
            alias,
            kind: step_kind,
            any: step_any,
            keys: step_keys,
            using_keys: step_using,
        });
    }
    let where_null = if parser.keyword("WHERE") {
        let column = qualified(&mut parser)?;
        if !parser.keyword("IS") {
            return Err(ParseError(
                "JOIN WHERE supports only qualified IS [NOT] NULL".into(),
            ));
        }
        let not_null = parser.keyword("NOT");
        if !parser.keyword("NULL") {
            return Err(ParseError(
                "JOIN WHERE supports only qualified IS [NOT] NULL".into(),
            ));
        }
        Some((column, not_null))
    } else {
        None
    };
    let group_by = if parser.keyword("GROUP") {
        if !parser.keyword("BY") {
            return Err(ParseError("JOIN GROUP requires BY".into()));
        }
        let mut keys = vec![qualified(&mut parser)?];
        while parser.take(Token::Comma) {
            if keys.len() >= 8 {
                return Err(ParseError("JOIN GROUP BY supports at most eight dimensions".into()));
            }
            keys.push(qualified(&mut parser)?);
        }
        keys
    } else { Vec::new() };
    let group_matches = match &aggregate {
        Some(JoinAggregate::GroupCount(key)) =>
            group_by.as_slice() == std::slice::from_ref(key),
        Some(JoinAggregate::GroupSum { group, .. }) =>
            group_by.as_slice() == std::slice::from_ref(group),
        Some(JoinAggregate::GroupMultiCount(keys)) |
        Some(JoinAggregate::GroupMultiSum { keys, .. }) |
        Some(JoinAggregate::GroupSumCount { keys, .. }) => group_by == *keys,
        _ => group_by.is_empty(),
    };
    if !group_matches {
        return Err(ParseError("JOIN GROUP BY must match the selected dimensions".into()));
    }
    let group_having = if parser.keyword("HAVING") {
        match &aggregate {
            Some(JoinAggregate::GroupCount(_) | JoinAggregate::GroupMultiCount(_)) => {
                if !parser.keyword("COUNT") || !parser.take(Token::LParen)
                    || !parser.take(Token::Star) || !parser.take(Token::RParen) {
                    return Err(ParseError("JOIN HAVING requires count(*)".into()));
                }
            }
            Some(JoinAggregate::GroupSum { value, .. }) |
            Some(JoinAggregate::GroupMultiSum { value, .. }) |
            Some(JoinAggregate::GroupSumCount { value, .. }) => {
                if !parser.keyword("SUM") || !parser.take(Token::LParen) {
                    return Err(ParseError("JOIN HAVING requires sum(column)".into()));
                }
                let selected = qualified(&mut parser)?;
                if selected != *value || !parser.take(Token::RParen) {
                    return Err(ParseError("JOIN HAVING must match selected SUM".into()));
                }
            }
            _ => return Err(ParseError("JOIN HAVING requires grouped COUNT or SUM".into())),
        }
        let operator = parser.operator()?;
        let value = parser.number()?.parse::<u64>()
            .map_err(|_| ParseError("JOIN HAVING threshold must be UInt64".into()))?;
        Some(Having { operator, value })
    } else { None };
    let mut group_order_by = None;
    let order_by = if parser.keyword("ORDER") {
        if !parser.keyword("BY") {
            return Err(ParseError("ORDER requires BY".into()));
        }
        if let Some(aggregate @ (JoinAggregate::GroupCount(_) |
            JoinAggregate::GroupMultiCount(_) |
            JoinAggregate::GroupSum { .. } |
            JoinAggregate::GroupMultiSum { .. } | JoinAggregate::GroupSumCount { .. })) = aggregate.as_ref() {
            let position = matches!(parser.tokens.get(parser.index),
                Some(Token::Number(value)) if value == "2");
            if position {
                parser.index += 1;
            } else {
                match aggregate {
                    JoinAggregate::GroupCount(_) | JoinAggregate::GroupMultiCount(_) => {
                        if !parser.keyword("COUNT") || !parser.take(Token::LParen)
                            || !parser.take(Token::Star) || !parser.take(Token::RParen) {
                            return Err(ParseError("JOIN grouped ORDER BY requires count(*)".into()));
                        }
                    }
                    JoinAggregate::GroupSum { value, .. } |
                    JoinAggregate::GroupMultiSum { value, .. } |
                    JoinAggregate::GroupSumCount { value, .. } => {
                        if !parser.keyword("SUM") || !parser.take(Token::LParen) {
                            return Err(ParseError("JOIN grouped ORDER BY requires sum(column)".into()));
                        }
                        let selected = qualified(&mut parser)?;
                        if selected != *value || !parser.take(Token::RParen) {
                            return Err(ParseError("JOIN ORDER BY must match selected SUM".into()));
                        }
                    }
                    _ => unreachable!(),
                }
            }
            let descending = if parser.keyword("DESC") { true } else {
                parser.keyword("ASC");
                false
            };
            let nulls_first = if parser.keyword("NULLS") {
                if parser.keyword("FIRST") { Some(true) }
                else if parser.keyword("LAST") { Some(false) }
                else { return Err(ParseError("JOIN ORDER BY NULLS requires FIRST or LAST".into())); }
            } else { None };
            group_order_by = Some((descending, nulls_first));
            Vec::new()
        } else {
        let mut terms = Vec::new();
        loop {
            if terms.len() == 8 {
                return Err(ParseError("JOIN ORDER BY supports at most 8 keys".into()));
            }
            let key = qualified(&mut parser)?;
            let descending = if parser.keyword("DESC") {
                true
            } else {
                parser.keyword("ASC");
                false
            };
            let nulls_first = if parser.keyword("NULLS") {
                if parser.keyword("FIRST") {
                    Some(true)
                } else if parser.keyword("LAST") {
                    Some(false)
                } else {
                    return Err(ParseError(
                        "JOIN ORDER BY NULLS requires FIRST or LAST".into(),
                    ));
                }
            } else {
                None
            };
            terms.push((key, descending, nulls_first));
            if !parser.take(Token::Comma) {
                break;
            }
        }
        terms
        }
    } else {
        Vec::new()
    };
    let known_qualifier = |relation: &str| {
        relation == left_alias
            || relation == right_alias
            || steps.iter().any(|step| step.alias == relation)
    };
    if let Some(columns) = &columns {
        for column in columns {
            if !known_qualifier(&column.relation) {
                return Err(ParseError(format!(
                    "unknown JOIN qualifier: {}",
                    column.relation
                )));
            }
        }
    }
    if let Some((column, _)) = &where_null {
        if !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    for (column, _, _) in &order_by {
        if !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    let (limit, comma_offset) = if parser.keyword("LIMIT") {
        let first = bound(&mut parser, "LIMIT")?;
        if parser.take(Token::Comma) {
            (Some(bound(&mut parser, "LIMIT")?), Some(first))
        } else {
            (Some(first), None)
        }
    } else {
        (None, None)
    };
    let offset = if let Some(offset) = comma_offset {
        offset
    } else if parser.keyword("OFFSET") {
        bound(&mut parser, "OFFSET")?
    } else {
        0
    };
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError(
            "unexpected JOIN syntax; supported: equi-JOIN, ORDER BY, LIMIT, OFFSET".into(),
        ));
    }
    if aggregate.is_some() &&
        (kind != JoinType::Inner || !steps.is_empty() || !order_by.is_empty()) {
        return Err(ParseError(
            "streamed JOIN aggregate requires a two-table INNER JOIN without ORDER BY".into()
        ));
    }
    Ok(Some(JoinSelect {
        left,
        right,
        left_alias,
        right_alias,
        kind,
        any,
        columns,
        aggregate,
        keys,
        using_keys,
        where_null,
        order_by,
        group_order_by,
        group_having,
        limit,
        offset,
        steps,
    }))
}

pub fn explain_join(query: &JoinSelect) -> String {
    let conditions = query
        .keys
        .iter()
        .map(|(left, right)| {
            format!(
                "{}.{} = {}.{}",
                left.relation, left.name, right.relation, right.name
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let predicate = if query.keys.is_empty() && query.using_keys.is_empty() {
        "cartesian product".to_owned()
    } else if query.using_keys.is_empty() {
        format!("ON {conditions}")
    } else {
        format!("USING ({})", query.using_keys.join(", "))
    };
    let mut plan = format!(
        "Source: table {} AS {}\nSource: table {} AS {}\nJoin: {} {}",
        query.left,
        query.left_alias,
        query.right,
        query.right_alias,
        if query.keys.is_empty() && query.using_keys.is_empty() {
            "Cross".to_owned()
        } else {
            format!("{}{:?}", if query.any { "Any " } else { "" }, query.kind)
        },
        predicate,
    );
    for step in &query.steps {
        let conditions = step
            .keys
            .iter()
            .map(|(left, right)| {
                format!(
                    "{}.{} = {}.{}",
                    left.relation, left.name, right.relation, right.name
                )
            })
            .collect::<Vec<_>>()
            .join(" AND ");
        let predicate = if step.keys.is_empty() && step.using_keys.is_empty() {
            "cartesian product".to_owned()
        } else if step.using_keys.is_empty() {
            format!("ON {conditions}")
        } else {
            format!("USING ({})", step.using_keys.join(", "))
        };
        plan.push_str(&format!(
            "\nSource: table {} AS {}\nJoin: {} {}",
            step.table,
            step.alias,
            if step.keys.is_empty() && step.using_keys.is_empty() {
                "Cross".to_owned()
            } else {
                format!("{}{:?}", if step.any { "Any " } else { "" }, step.kind)
            },
            predicate
        ));
    }
    plan.push_str(&format!(
        "\nProject: {}\nLimit: {}",
        if let Some(aggregate) = &query.aggregate {
            match aggregate {
                JoinAggregate::CountStar => "count(*)".to_owned(),
                JoinAggregate::CountDistinct(col) => {
                    format!("count(DISTINCT {}.{})", col.relation, col.name)
                },
                JoinAggregate::Sum(col) => format!("sum({}.{})", col.relation, col.name),
                JoinAggregate::GroupCount(col) => {
                    format!("{}.{}, count(*)", col.relation, col.name)
                },
                JoinAggregate::GroupMultiCount(keys) => {
                    let columns = keys.iter().map(|key|
                        format!("{}.{}", key.relation, key.name)
                    ).collect::<Vec<_>>().join(", ");
                    format!("{columns}, count(*)")
                },
                JoinAggregate::GroupMultiSum { keys, value } => {
                    let columns = keys.iter().map(|key|
                        format!("{}.{}", key.relation, key.name)
                    ).collect::<Vec<_>>().join(", ");
                    format!("{columns}, sum({}.{})", value.relation, value.name)
                },
                JoinAggregate::GroupSumCount { keys, value } => {
                    let columns = keys.iter().map(|key|
                        format!("{}.{}", key.relation, key.name)
                    ).collect::<Vec<_>>().join(", ");
                    format!("{columns}, sum({}.{}), count(*)", value.relation, value.name)
                },
                JoinAggregate::GroupSum { group, value } => format!(
                    "{}.{}, sum({}.{})",
                    group.relation, group.name, value.relation, value.name
                ),
            }
        } else { query.columns.as_ref().map_or("*".to_owned(), |columns| {
            columns
                .iter()
                .map(|col| format!("{}.{}", col.relation, col.name))
                .collect::<Vec<_>>()
                .join(", ")
        }) },
        query
            .limit
            .map_or("unbounded".to_owned(), |n| n.to_string())
    ));
    if !query.using_keys.is_empty() {
        plan.push_str(&format!("\nUsing: {}", query.using_keys.join(", ")));
    }
    if let Some((column, not_null)) = &query.where_null {
        plan.push_str(&format!(
            "\nFilter: {}.{} IS {}NULL",
            column.relation,
            column.name,
            if *not_null { "NOT " } else { "" }
        ));
    }
    if !query.order_by.is_empty() {
        let terms = query
            .order_by
            .iter()
            .map(|(column, descending, nulls_first)| {
                let placement = match nulls_first {
                    Some(true) => " NULLS FIRST",
                    Some(false) => " NULLS LAST",
                    None => "",
                };
                format!(
                    "{}.{} {}{}",
                    column.relation,
                    column.name,
                    if *descending { "DESC" } else { "ASC" },
                    placement
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        plan.push_str(&format!("\nOrder: {terms}"));
    }
    if let Some(having) = &query.group_having {
        plan.push_str(&format!("\nHaving: {:?} {}", having.operator, having.value));
    }
    if let Some((descending, _)) = query.group_order_by {
        plan.push_str(&format!("\nAggregate order: {}", if descending { "DESC" } else { "ASC" }));
    }
    if query.offset > 0 {
        plan.push_str(&format!("\nOffset: {}", query.offset));
    }
    if query.steps.is_empty() {
        plan.push_str(&format!(
            "\nExecution bounds: buffered input <= {} rows/side; streaming INNER build <= {} rows; result <= {} rows; build budget {} MiB",
            MAX_BUFFERED_JOIN_ROWS,
            MAX_STREAM_JOIN_BUILD_ROWS,
            MAX_JOIN_RESULT_ROWS,
            MAX_JOIN_BUILD_BYTES / (1024 * 1024)
        ));
    } else {
        plan.push_str(&format!(
            "\nExecution bounds: chained intermediate <= {} rows and {} MiB; individual joins retain input limits",
            MAX_JOIN_RESULT_ROWS,
            MAX_JOIN_CHAIN_BYTES / (1024 * 1024)
        ));
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explain_uses_the_same_resource_limits_as_join_execution() {
        let query = parse_join("SELECT * FROM a x JOIN b y ON x.id = y.id")
            .unwrap()
            .unwrap();
        let plan = explain_join(&query);
        assert!(plan.contains("buffered input <= 32768 rows/side"), "{plan}");
        assert!(
            plan.contains("streaming INNER build <= 65536 rows"),
            "{plan}"
        );
        assert!(plan.contains("build budget 32 MiB"), "{plan}");

        let chained =
            parse_join("SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON y.id = z.id")
                .unwrap()
                .unwrap();
        let chain_plan = explain_join(&chained);
        assert!(
            chain_plan.contains("chained intermediate <= 131072 rows and 64 MiB"),
            "{chain_plan}"
        );
    }

    #[test]
    fn chained_join_accepts_nested_on_equalities() {
        let query = parse_join(
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON (y.id = z.id AND (x.k = z.k))",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.steps[0].keys.len(), 2);
        for statement in [
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON (y.id = z.id",
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON (y.id = z.id OR x.k = z.k)",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
    }

    #[test]
    fn chained_join_rejects_equalities_on_the_same_side() {
        for statement in [
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON x.id = y.id",
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON z.id = z.other",
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON z.id = x.id AND x.k = y.k",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
        assert!(
            parse_join("SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON y.id = z.id",)
                .is_ok()
        );
    }

    #[test]
    fn explain_shows_the_original_using_predicate() {
        let query = parse_join("SELECT * FROM a x FULL JOIN b y USING (id, tenant)")
            .unwrap()
            .unwrap();
        let plan = explain_join(&query);
        assert!(plan.contains("Join: Full USING (id, tenant)"), "{plan}");
        assert!(!plan.contains("Join: Full ON"), "{plan}");
    }

    #[test]
    fn chained_join_rejects_unknown_on_qualifiers_at_parse_time() {
        for statement in [
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON missing.id = z.id",
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON z.id = missing.id",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
        assert!(
            parse_join("SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON x.id = z.id",)
                .is_ok()
        );
    }

    #[test]
    fn explain_lists_chained_sources_before_project_and_limit() {
        let query = parse_join(
            "SELECT x.id FROM a x JOIN b y ON x.id = y.id JOIN c z ON y.id = z.id LIMIT 3",
        )
        .unwrap()
        .unwrap();
        let plan = explain_join(&query);
        let step = plan.find("Source: table c AS z").unwrap();
        let project = plan.find("Project: x.id").unwrap();
        let limit = plan.find("Limit: 3").unwrap();
        assert!(step < project && project < limit, "{plan}");
    }

    #[test]
    fn chained_joins_accept_explicit_all_strictness() {
        for statement in [
            "SELECT * FROM a x JOIN b y ON x.id = y.id ALL JOIN c z ON y.id = z.id",
            "SELECT * FROM a x JOIN b y ON x.id = y.id INNER ALL JOIN c z ON y.id = z.id",
            "SELECT * FROM a x JOIN b y ON x.id = y.id LEFT ALL JOIN c z USING (id)",
        ] {
            let query = parse_join(statement).unwrap().unwrap();
            assert_eq!(query.steps.len(), 1);
            assert!(!query.steps[0].any, "{statement}");
        }
        for statement in [
            "SELECT * FROM a x JOIN b y ON x.id = y.id ALL ALL JOIN c z USING (id)",
            "SELECT * FROM a x JOIN b y ON x.id = y.id ANY INNER ANY JOIN c z USING (id)",
            "SELECT * FROM a x JOIN b y ON x.id = y.id ALL INNER ANY JOIN c z USING (id)",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
    }

    #[test]
    fn explain_includes_using_filter_sort_and_offset() {
        let query = parse_join(
            "SELECT a.id FROM a FULL JOIN b USING (id) WHERE b.id IS NULL ORDER BY a.id DESC NULLS LAST LIMIT 5 OFFSET 2",
        )
        .unwrap()
        .unwrap();
        let plan = explain_join(&query);
        for expected in [
            "Using: id",
            "Filter: b.id IS NULL",
            "Order: a.id DESC NULLS LAST",
            "Offset: 2",
        ] {
            assert!(plan.contains(expected), "{plan}");
        }
    }

    #[test]
    fn explain_lists_multiple_join_sort_columns() {
        let query = parse_join(
            "SELECT a.id, b.name FROM a JOIN b ON a.id = b.id ORDER BY a.id ASC, b.name DESC NULLS LAST LIMIT 3",
        )
        .unwrap()
        .unwrap();
        let plan = explain_join(&query);
        assert!(
            plan.contains("Order: a.id ASC, b.name DESC NULLS LAST"),
            "{plan}"
        );
    }

    #[test]
    fn parses_chained_using_keys_and_rejects_duplicates() {
        let query = parse_join(
            "SELECT * FROM facts f JOIN customers c ON f.customer_id = c.id JOIN regions r USING (region_id)",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.steps[0].using_keys, ["region_id"]);
        assert!(query.steps[0].keys.is_empty());
        let query = parse_join(
            "SELECT * FROM facts f JOIN customers c USING (customer_id) JOIN regions r ON c.customer_id = r.customer_id",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.using_keys, ["customer_id"]);
        assert!(parse_join(
            "SELECT * FROM facts f JOIN customers c ON f.id = c.id JOIN regions r USING (id, id)"
        )
        .is_err());
    }

    #[test]
    fn parses_chained_any_join_strictness() {
        let query = parse_join(
            "SELECT * FROM a x LEFT ANY JOIN b y ON x.id = y.id ANY LEFT JOIN c z ON y.id = z.id",
        )
        .unwrap()
        .unwrap();
        assert!(query.any);
        assert!(query.steps[0].any);
        assert_eq!(query.steps[0].kind, JoinType::Left);
        assert!(
            parse_join(
                "SELECT * FROM a x JOIN b y ON x.id = y.id FULL ANY JOIN c z ON y.id = z.id"
            )
            .is_err()
        );
        let right_any = parse_join(
            "SELECT * FROM a x RIGHT ANY JOIN b y ON x.id = y.id LEFT JOIN c z ON y.id = z.id",
        )
        .unwrap()
        .unwrap();
        assert_eq!(right_any.kind, JoinType::Right);
        assert!(right_any.any);
        assert_eq!(right_any.steps.len(), 1);
        assert!(!right_any.steps[0].any);
    }

    #[test]
    fn parses_chained_right_and_full_joins() {
        let query = parse_join(
            "SELECT o.id, r.id FROM orders o JOIN customers c ON o.id = c.id RIGHT JOIN regions r ON c.id = r.id",
        )
        .unwrap().unwrap();
        assert_eq!(query.steps[0].kind, JoinType::Right);
        let query = parse_join(
            "SELECT * FROM orders o FULL JOIN customers c ON o.id = c.id FULL OUTER JOIN regions r ON c.id = r.id",
        )
        .unwrap().unwrap();
        assert_eq!(query.kind, JoinType::Full);
        assert_eq!(query.steps[0].kind, JoinType::Full);
    }

    #[test]
    fn parses_chained_left_joins() {
        let query = parse_join(
            "SELECT o.id, r.label FROM orders o LEFT JOIN customers c ON o.customer_id = c.id LEFT OUTER JOIN regions r ON c.region_id = r.id",
        )
        .unwrap().unwrap();
        assert_eq!(query.kind, JoinType::Left);
        assert_eq!(query.steps[0].kind, JoinType::Left);
        assert_eq!(query.steps[0].keys[0].0.relation, "c");
    }

    #[test]
    fn validates_projected_aliases_after_parsing_join_chain() {
        let valid = "SELECT r.label FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r ON c.region_id = r.id";
        assert_eq!(
            parse_join(valid).unwrap().unwrap().columns.unwrap()[0].relation,
            "r"
        );
        let invalid = "SELECT z.label FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r ON c.region_id = r.id";
        assert!(parse_join(invalid).is_err());
    }

    #[test]
    fn parses_three_table_inner_joins() {
        let query = parse_join("SELECT o.id, c.name, r.label FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r ON c.region_id = r.id ORDER BY o.id LIMIT 5").unwrap().unwrap();
        assert_eq!(query.steps.len(), 1);
        assert_eq!(query.steps[0].table, "regions");
        assert_eq!(query.steps[0].keys[0].0.relation, "c");
        assert_eq!(query.order_by[0].0.relation, "o");
        assert!(
            parse_join("SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c x ON y.id = x.id")
                .is_err()
        );
        assert!(
            parse_join(
                "SELECT * FROM a x JOIN b y ON x.id = y.id RIGHT ANY JOIN c z ON y.id = z.id"
            )
            .is_ok()
        );
    }

    #[test]
    fn repeated_on_predicates_do_not_exhaust_unique_key_limit() {
        let repeated = (0..12)
            .map(|i| {
                if i % 2 == 0 {
                    "a.id = b.id"
                } else {
                    "b.id = a.id"
                }
            })
            .collect::<Vec<_>>()
            .join(" AND ");
        let query = parse_join(&format!("SELECT * FROM l a JOIN r b ON {repeated}"))
            .unwrap()
            .unwrap();
        assert_eq!(query.keys.len(), 1);
        let distinct = (0..9)
            .map(|i| format!("a.k{i} = b.k{i}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        assert!(parse_join(&format!("SELECT * FROM l a JOIN r b ON {distinct}")).is_err());
    }

    #[test]
    fn rejects_unknown_filter_and_sort_qualifiers_during_parse() {
        for statement in [
            "SELECT * FROM l a JOIN r b ON a.id = b.id WHERE c.id IS NULL",
            "SELECT * FROM l a JOIN r b ON a.id = b.id ORDER BY c.id",
            "SELECT * FROM l a JOIN r b ON a.id = b.id ORDER BY a.id, c.id DESC",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
        assert!(
            parse_join(
                "SELECT * FROM l a JOIN r b ON a.id = b.id WHERE b.id IS NOT NULL ORDER BY a.id"
            )
            .is_ok()
        );
        assert!(
            parse_join(
                "SELECT * FROM l a JOIN r b ON a.id = b.id JOIN x c ON b.id = c.id ORDER BY c.id"
            )
            .is_ok()
        );
    }

    #[test]
    fn parses_inner_joins_with_aliases_and_bounds() {
        let query = parse_join(
            "SELECT u.id, p.label FROM users AS u INNER JOIN plans p ON u.plan_id = p.id ORDER BY u.id DESC LIMIT 5 OFFSET 2;",
        )
        .unwrap().unwrap();
        assert_eq!(query.left, "users");
        assert_eq!(query.right_alias, "p");
        assert_eq!(query.keys[0].0.name, "plan_id");
        assert_eq!(query.limit, Some(5));
        assert_eq!(query.offset, 2);
        assert!(parse_join("SELECT id FROM users").unwrap().is_none());
    }

    #[test]
    fn parses_composite_join_predicates() {
        let query = parse_join(
            "SELECT a.id, b.id FROM first a LEFT JOIN second b ON a.tenant = b.tenant AND b.user_id = a.user_id ORDER BY a.id LIMIT 2",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.keys.len(), 2);
        assert_eq!(query.keys[1].0.relation, "b");
        let excess = format!(
            "SELECT * FROM a JOIN b ON {}",
            (0..9)
                .map(|index| format!("a.k{index} = b.k{index}"))
                .collect::<Vec<_>>()
                .join(" AND ")
        );
        assert!(parse_join(&excess).is_err());
        assert!(
            parse_join("SELECT * FROM a JOIN b ON a.id = b.id AND a.user_id != b.user_id").is_err()
        );
    }

    #[test]
    fn parses_parenthesized_join_equalities() {
        for statement in [
            "SELECT * FROM a JOIN b ON (a.id = b.id)",
            "SELECT * FROM a JOIN b ON (a.id = b.id AND (a.k = b.k))",
            "SELECT * FROM a JOIN b ON (a.id = b.id) AND (a.k = b.k)",
        ] {
            let parsed = parse_join(statement).unwrap().unwrap();
            assert_eq!(parsed.keys.len(), statement.matches(" = ").count());
        }
        for statement in [
            "SELECT * FROM a JOIN b ON (a.id = b.id",
            "SELECT * FROM a JOIN b ON a.id = b.id)",
            "SELECT * FROM a JOIN b ON (a.id = b.id OR a.k = b.k)",
            "SELECT * FROM a JOIN b ON ()",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
        let deep = format!(
            "SELECT * FROM a JOIN b ON {}a.id = b.id{}",
            "(".repeat(17),
            ")".repeat(17)
        );
        assert!(parse_join(&deep).is_err());
    }

    #[test]
    fn removes_duplicate_and_reversed_join_equalities() {
        let parsed = parse_join(
            "SELECT * FROM a JOIN b ON a.id = b.id AND b.id = a.id AND a.k = b.k AND a.id = b.id",
        )
        .unwrap()
        .unwrap();
        assert_eq!(parsed.keys.len(), 2);
        assert_eq!(parsed.keys[0].0.name, "id");
        assert_eq!(parsed.keys[1].0.name, "k");
    }

    #[test]
    fn join_on_requires_two_known_distinct_relations() {
        for statement in [
            "SELECT * FROM a x JOIN b y ON x.id = x.parent",
            "SELECT * FROM a x JOIN b y ON y.id = y.parent",
            "SELECT * FROM a x JOIN b y ON x.id = z.id",
            "SELECT * FROM a x JOIN b y ON z.id = y.id",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
        assert!(
            parse_join("SELECT * FROM a x JOIN b y ON y.id = x.id")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn parses_using_keys_with_aliases_and_rejects_invalid_lists() {
        let query = parse_join(
            "SELECT l.id, r.name FROM lefts AS l FULL JOIN rights AS r USING (id, region) LIMIT 5",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.kind, JoinType::Full);
        assert_eq!(query.using_keys, ["id", "region"]);
        assert_eq!(query.keys[0].0.relation, "l");
        assert_eq!(query.keys[0].1.relation, "r");
        assert_eq!(query.keys[1].0.name, "region");
        for sql in [
            "SELECT * FROM l JOIN r USING id",
            "SELECT * FROM l JOIN r USING ()",
            "SELECT * FROM l JOIN r USING (id, id)",
            "SELECT * FROM l JOIN r USING (id,) ",
            "SELECT * FROM l JOIN r USING (id) ON l.id = r.id",
        ] {
            assert!(parse_join(sql).is_err(), "{sql}");
        }
        let too_many = format!(
            "SELECT * FROM l JOIN r USING ({})",
            (0..9)
                .map(|n| format!("key{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(parse_join(&too_many).is_err());
    }

    #[test]
    fn parses_qualified_wildcard_projections() {
        let query =
            parse_join("SELECT l.*, r.label, r.* FROM lefts l JOIN rights r ON l.id = r.id")
                .unwrap()
                .unwrap();
        assert_eq!(
            query
                .columns
                .unwrap()
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["*", "label", "*"]
        );
        assert!(parse_join("SELECT id, r.* FROM lefts l JOIN rights r ON l.id = r.id").is_err());
    }

    #[test]
    fn parses_clickhouse_style_join_limit_offset_count() {
        let query = parse_join("SELECT * FROM l JOIN r ON l.id = r.id LIMIT 7, 3")
            .unwrap()
            .unwrap();
        assert_eq!(query.offset, 7);
        assert_eq!(query.limit, Some(3));
        let query = parse_join("SELECT * FROM l JOIN r ON l.id = r.id LIMIT 3 OFFSET 7")
            .unwrap()
            .unwrap();
        assert_eq!(query.offset, 7);
        assert_eq!(query.limit, Some(3));
        assert!(parse_join("SELECT * FROM l JOIN r ON l.id = r.id LIMIT 2, 3 OFFSET 1").is_err());
        assert!(parse_join("SELECT * FROM l JOIN r ON l.id = r.id LIMIT 2, -1").is_err());
    }

    #[test]
    fn parses_offset_without_limit() {
        let query = parse_join("SELECT * FROM l JOIN r ON l.id = r.id OFFSET 3")
            .unwrap()
            .unwrap();
        assert_eq!(query.limit, None);
        assert_eq!(query.offset, 3);
        let query = parse_join("SELECT * FROM l JOIN r ON l.id = r.id ORDER BY l.id OFFSET 2")
            .unwrap()
            .unwrap();
        assert_eq!(query.offset, 2);
        assert!(parse_join("SELECT * FROM l JOIN r ON l.id = r.id OFFSET -1").is_err());
    }

    #[test]
    fn parses_any_inner_and_left_join_strictness() {
        for statement in [
            "SELECT * FROM a ANY JOIN b ON a.id = b.id",
            "SELECT * FROM a ANY INNER JOIN b ON a.id = b.id",
            "SELECT * FROM a LEFT ANY JOIN b ON a.id = b.id",
            "SELECT * FROM a ANY LEFT JOIN b ON a.id = b.id",
        ] {
            assert!(parse_join(statement).unwrap().unwrap().any, "{statement}");
        }
        for statement in [
            "SELECT * FROM a RIGHT ANY JOIN b ON a.id = b.id",
            "SELECT * FROM a ANY RIGHT JOIN b ON a.id = b.id",
        ] {
            let query = parse_join(statement).unwrap().unwrap();
            assert!(query.any);
            assert_eq!(query.kind, JoinType::Right);
        }
        let ordinary = parse_join("SELECT * FROM a JOIN b ON a.id = b.id")
            .unwrap()
            .unwrap();
        assert!(!ordinary.any);
        let statement = "SELECT * FROM a ANY FULL JOIN b ON a.id = b.id";
        assert!(parse_join(statement).is_err(), "{statement}");
    }

    #[test]
    fn parses_explicit_all_join_strictness() {
        for statement in [
            "SELECT * FROM a ALL JOIN b ON a.id = b.id",
            "SELECT * FROM a ALL INNER JOIN b ON a.id = b.id",
            "SELECT * FROM a INNER ALL JOIN b ON a.id = b.id",
            "SELECT * FROM a LEFT ALL JOIN b USING (id)",
            "SELECT * FROM a RIGHT ALL JOIN b USING (id)",
            "SELECT * FROM a FULL ALL JOIN b USING (id)",
        ] {
            assert!(!parse_join(statement).unwrap().unwrap().any, "{statement}");
        }
        for statement in [
            "SELECT * FROM a ALL ANY JOIN b ON a.id = b.id",
            "SELECT * FROM a ANY INNER ALL JOIN b ON a.id = b.id",
            "SELECT * FROM a ALL INNER ALL JOIN b ON a.id = b.id",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
    }

    #[test]
    fn parses_post_join_null_filters() {
        let query = parse_join(
            "SELECT l.id FROM lefts l LEFT JOIN rights r USING (id) WHERE r.id IS NULL ORDER BY l.id",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.where_null.unwrap().0.relation, "r");
        let query = parse_join(
            "SELECT l.id FROM lefts l JOIN rights r ON l.id = r.id WHERE l.id IS NOT NULL",
        )
        .unwrap()
        .unwrap();
        assert!(query.where_null.unwrap().1);
        for sql in [
            "SELECT * FROM l JOIN r ON l.id = r.id WHERE r.id = 1",
            "SELECT * FROM l JOIN r ON l.id = r.id WHERE r.id IS UNKNOWN",
        ] {
            assert!(parse_join(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_explicit_join_null_sort_order() {
        for (suffix, expected) in [
            ("NULLS FIRST", Some(true)),
            ("DESC NULLS LAST", Some(false)),
            ("ASC", None),
        ] {
            let statement =
                format!("SELECT * FROM l LEFT JOIN r ON l.id = r.id ORDER BY r.id {suffix}");
            let query = parse_join(&statement).unwrap().unwrap();
            assert_eq!(query.order_by[0].2, expected);
        }
        assert!(
            parse_join("SELECT * FROM l JOIN r ON l.id = r.id ORDER BY l.id NULLS MIDDLE").is_err()
        );
    }

    #[test]
    fn parses_multiple_join_sort_columns() {
        let query = parse_join(
            "SELECT l.id, r.rank FROM lefts l JOIN rights r USING (id) ORDER BY r.rank DESC NULLS LAST, l.id ASC LIMIT 10",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.order_by.len(), 2);
        assert_eq!(query.order_by[0].0.relation, "r");
        assert!(query.order_by[0].1);
        assert_eq!(query.order_by[0].2, Some(false));
        assert_eq!(query.order_by[1].0.name, "id");
        assert!(!query.order_by[1].1);
        let too_many = format!(
            "SELECT * FROM l JOIN r USING (id) ORDER BY {}",
            ["l.id"; 9].join(", ")
        );
        assert!(parse_join(&too_many).is_err());
    }

    #[test]
    fn rejects_ambiguous_or_unsupported_join_syntax() {
        for sql in [
            "SELECT * FROM users JOIN users ON users.id = users.id",
            "SELECT * FROM a JOIN b ON a.id > b.id",
            "SELECT id FROM a JOIN b ON a.id = b.id",
            "SELECT * FROM a JOIN b ON a.id = b.id OR a.id = b.id",
        ] {
            assert!(parse_join(sql).is_err(), "{sql}");
        }
    }
    #[test]
    fn rejects_unknown_projected_qualifiers_during_parse() {
        for statement in [
            "SELECT c.id FROM lefts a JOIN rights b ON a.id = b.id",
            "SELECT c.* FROM lefts a JOIN rights b ON a.id = b.id LIMIT 0",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
        assert!(parse_join("SELECT a.*, b.id FROM lefts a JOIN rights b ON a.id = b.id").is_ok());
    }
}
