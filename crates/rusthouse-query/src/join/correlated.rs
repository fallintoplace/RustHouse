//! Parse bounded conjunctive EXISTS predicates over joined rows.

use super::{JoinColumn, JoinSelect, ParseError, Parser, Token, alias, projected, tokenize};
use crate::Operator;

/// A correlated lookup with one equality key and an optional inequality key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinExistsFilter {
    pub table: String,
    pub alias: String,
    pub negated: bool,
    /// Inner column followed by outer column.
    pub equality: (JoinColumn, JoinColumn),
    /// Inner column followed by outer column.
    pub inequality: Option<(JoinColumn, JoinColumn)>,
    pub inner_comparisons: Vec<(JoinColumn, Operator, JoinColumn)>,
}

/// Parsed plan only; callers must bind and evaluate each EXISTS filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinCorrelatedExists {
    pub(crate) outer: JoinSelect,
    pub filters: Vec<JoinExistsFilter>,
}

impl JoinCorrelatedExists {
    /// Outer sources and output settings; execute the complete plan with its filters.
    pub fn outer(&self) -> &JoinSelect {
        &self.outer
    }
    pub fn cap_rows(&mut self, cap: usize) {
        self.outer.limit = Some(self.outer.limit.map_or(cap, |limit| limit.min(cap)));
    }
}

fn keyword(token: &Token, name: &str) -> bool {
    matches!(token, Token::Word(word) if word.eq_ignore_ascii_case(name))
}

fn parse_filter(piece: &[Token]) -> Result<JoinExistsFilter, ParseError> {
    let mut parser = Parser {
        tokens: piece.to_vec(),
        index: 0,
    };
    let negated = parser.keyword("NOT");
    if !parser.keyword("EXISTS")
        || !parser.take(Token::LParen)
        || !parser.keyword("SELECT")
        || !parser.take(Token::Star)
        || !parser.keyword("FROM")
    {
        return Err(ParseError(
            "JOIN EXISTS requires SELECT * FROM table".into(),
        ));
    }
    let table = parser.bounded_word()?;
    let name = alias(&mut parser, &table)?;
    if !parser.keyword("WHERE") {
        return Err(ParseError("JOIN EXISTS requires WHERE".into()));
    }
    let mut equality = None;
    let mut inequality = None;
    let mut inner_comparisons = Vec::new();
    loop {
        let left = projected(&mut parser)?;
        let operator = parser.operator()?;
        let right = projected(&mut parser)?;
        if left.relation.is_empty()
            || right.relation.is_empty()
            || left.name == "*"
            || right.name == "*"
        {
            return Err(ParseError(
                "JOIN EXISTS comparisons require qualified columns".into(),
            ));
        }
        let left_inner = left.relation == name;
        let right_inner = right.relation == name;
        match (left_inner, right_inner, operator) {
            (
                true,
                true,
                Operator::Eq
                | Operator::Ne
                | Operator::Lt
                | Operator::Le
                | Operator::Gt
                | Operator::Ge,
            ) => {
                if inner_comparisons.len() == 8 {
                    return Err(ParseError(
                        "JOIN EXISTS supports at most 8 inner comparisons".into(),
                    ));
                }
                inner_comparisons.push((left, operator, right));
            }
            (true, false, Operator::Eq | Operator::Ne)
            | (false, true, Operator::Eq | Operator::Ne) => {
                let pair = if left_inner {
                    (left, right)
                } else {
                    (right, left)
                };
                let target = if operator == Operator::Eq {
                    &mut equality
                } else {
                    &mut inequality
                };
                if target.replace(pair).is_some() {
                    return Err(ParseError(
                        "JOIN EXISTS supports one correlation per operator".into(),
                    ));
                }
            }
            _ => return Err(ParseError("unsupported JOIN EXISTS comparison".into())),
        }
        if !parser.keyword("AND") {
            break;
        }
    }
    if !parser.take(Token::RParen) || parser.index != parser.tokens.len() {
        return Err(ParseError("unsupported JOIN EXISTS syntax".into()));
    }
    Ok(JoinExistsFilter {
        table,
        alias: name,
        negated,
        equality: equality
            .ok_or_else(|| ParseError("JOIN EXISTS requires an equality correlation".into()))?,
        inequality,
        inner_comparisons,
    })
}

/// Parse an INNER join and up to two conjunctive correlated EXISTS predicates.
/// The returned outer plan must not be executed without applying the filters.
pub fn parse_join_correlated_exists_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<JoinCorrelatedExists>, ParseError> {
    let tokens = tokenize(sql)?;
    let mut depth = 0usize;
    let mut where_index = None;
    let mut end = tokens.len();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && where_index.is_none() && keyword(token, "WHERE") => {
                where_index = Some(index)
            }
            _ if depth == 0
                && where_index.is_some()
                && ["GROUP", "ORDER", "LIMIT", "OFFSET"]
                    .iter()
                    .any(|name| keyword(token, name)) =>
            {
                end = index;
                break;
            }
            _ => {}
        }
    }
    let Some(where_index) = where_index else {
        return Ok(None);
    };
    let mut pieces = Vec::new();
    let mut start = where_index + 1;
    depth = 0;
    for index in where_index + 1..end {
        match &tokens[index] {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| ParseError("unbalanced JOIN WHERE".into()))?
            }
            token if depth == 0 && keyword(token, "BETWEEN") => return Ok(None),
            token if depth == 0 && keyword(token, "OR") => return Ok(None),
            token if depth == 0 && keyword(token, "AND") => {
                pieces.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(ParseError("unbalanced JOIN WHERE".into()));
    }
    pieces.push(&tokens[start..end]);
    let mut filters = Vec::new();
    let mut remaining = Vec::new();
    for piece in pieces {
        if piece.is_empty() {
            return Err(ParseError("incomplete JOIN WHERE".into()));
        }
        let exists = keyword(&piece[0], "EXISTS")
            || (piece.len() > 1 && keyword(&piece[0], "NOT") && keyword(&piece[1], "EXISTS"));
        if exists {
            if filters.len() == 2 {
                return Err(ParseError("JOIN supports at most 2 EXISTS filters".into()));
            }
            filters.push(parse_filter(piece)?);
        } else {
            remaining.push(piece);
        }
    }
    if filters.is_empty() {
        return Ok(None);
    }
    let mut outer = tokens[..where_index].to_vec();
    if !remaining.is_empty() {
        outer.push(Token::Word("WHERE".into()));
        for (index, piece) in remaining.iter().enumerate() {
            if index != 0 {
                outer.push(Token::Word("AND".into()));
            }
            outer.extend_from_slice(piece);
        }
    }
    outer.extend_from_slice(&tokens[end..]);
    let rewritten = super::implicit::rewrite(outer, Some(&mut resolve))?;
    let outer = super::parse_join_tokens(rewritten, false)?
        .ok_or_else(|| ParseError("EXISTS requires an outer JOIN".into()))?;
    if outer.kind != super::JoinType::Inner
        || outer
            .steps
            .iter()
            .any(|step| step.kind != super::JoinType::Inner)
    {
        return Err(ParseError("JOIN EXISTS requires INNER joins".into()));
    }
    let aliases = [&outer.left_alias, &outer.right_alias]
        .into_iter()
        .chain(outer.steps.iter().map(|step| &step.alias))
        .collect::<Vec<_>>();
    for filter in &filters {
        if aliases.contains(&&filter.alias) {
            return Err(ParseError(
                "JOIN EXISTS inner alias shadows an outer alias".into(),
            ));
        }
        for (_, column) in std::iter::once(&filter.equality).chain(filter.inequality.iter()) {
            if !aliases.contains(&&column.relation) {
                return Err(ParseError("unknown JOIN EXISTS outer qualifier".into()));
            }
        }
    }
    Ok(Some(JoinCorrelatedExists { outer, filters }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(table: &str) -> Result<Vec<String>, ParseError> {
        let names: &[&str] = match table {
            "supplier" => &["s_name", "s_suppkey", "s_nationkey"],
            "lineitem" => &["l_orderkey", "l_suppkey", "l_receiptdate", "l_commitdate"],
            "orders" => &["o_orderkey", "o_orderstatus"],
            "nation" => &["n_nationkey", "n_name"],
            _ => return Err(ParseError("unknown table".into())),
        };
        Ok(names.iter().map(|name| (*name).to_owned()).collect())
    }

    #[test]
    fn detects_only_top_level_exists_tokens() {
        assert!(super::super::has_top_level_exists("SELECT * FROM a JOIN b ON a.id = b.id WHERE NOT EXISTS (SELECT * FROM c WHERE c.id = a.id)").unwrap());
        assert!(!super::super::has_top_level_exists("SELECT * FROM a JOIN b ON a.id = b.id WHERE a.name = 'EXISTS' -- EXISTS\n ORDER BY a.id").unwrap());
        assert!(!super::super::has_top_level_exists("SELECT * FROM a JOIN b ON a.id = b.id WHERE a.id IN (SELECT id FROM c WHERE EXISTS (SELECT * FROM d))").unwrap());
    }

    #[test]
    fn parses_tpch_q21_filters_and_preserves_outer_plan() {
        let sql = include_str!("../../../../bench/tpch/queries/q21.sql");
        let plan = parse_join_correlated_exists_with_schema(sql, schema)
            .unwrap()
            .unwrap();
        assert_eq!(plan.filters.len(), 2);
        assert!(!plan.filters[0].negated);
        assert!(plan.filters[1].negated);
        assert_eq!(plan.filters[0].equality.0.display_name(), "l2.l_orderkey");
        assert_eq!(plan.filters[0].equality.1.display_name(), "l1.l_orderkey");
        assert_eq!(
            plan.filters[0]
                .inequality
                .as_ref()
                .unwrap()
                .1
                .display_name(),
            "l1.l_suppkey"
        );
        assert_eq!(plan.filters[1].inner_comparisons.len(), 1);
        assert_eq!(plan.outer.steps.len(), 2);
        assert!(plan.outer.grouped_count.is_some());
        assert_eq!(plan.outer.limit, Some(100));
    }

    #[test]
    fn rejects_unsafe_or_unbounded_shapes() {
        let base = "SELECT s_name, count(*) AS n FROM supplier, lineitem l1 WHERE s_suppkey = l1.l_suppkey AND EXISTS (SELECT * FROM lineitem l2 WHERE l2.l_orderkey = l1.l_orderkey AND l2.l_suppkey <> l1.l_suppkey) GROUP BY s_name";
        assert!(
            parse_join_correlated_exists_with_schema(base, schema)
                .unwrap()
                .is_some()
        );
        for bad in [
            base.replace("AND EXISTS", "OR EXISTS"),
            base.replace(
                "l2.l_orderkey = l1.l_orderkey",
                "l2.l_orderkey > l1.l_orderkey",
            ),
            base.replace(
                "l2.l_suppkey <> l1.l_suppkey",
                "l2.l_suppkey <> missing.l_suppkey",
            ),
            base.replace("lineitem l2 WHERE", "lineitem l1 WHERE"),
            base.replace(
                "l2.l_orderkey = l1.l_orderkey",
                "l_orderkey = l1.l_orderkey",
            ),
            base.replace(
                "l2.l_suppkey <> l1.l_suppkey",
                "l2.l_suppkey <> l1.l_suppkey OR l2.l_orderkey = l1.l_orderkey",
            ),
        ] {
            assert!(
                !matches!(
                    parse_join_correlated_exists_with_schema(&bad, schema),
                    Ok(Some(_))
                ),
                "{bad}"
            );
        }
    }
}
