//! Parse bounded conjunctive EXISTS predicates over joined rows.

use super::{JoinColumn, JoinSelect, ParseError, Parser, Token, alias, projected, tokenize};
use crate::{Operator, Select};

const MAX_EXISTS_FILTERS: usize = 8;
const MAX_EXISTS_GROUPS: usize = 16;
const MAX_EXISTS_GROUP_SIZE: usize = 8;

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
    /// Optional date-dimension prefilter for fact-table EXISTS scans.
    pub date_filter: Option<JoinExistsDateFilter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinExistsDateFilter {
    pub table: String,
    pub alias: String,
    /// Inner fact column followed by the date-dimension key.
    pub keys: (JoinColumn, JoinColumn),
    pub select: Box<Select>,
}

/// Parsed plan only; callers must bind and evaluate each EXISTS filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinCorrelatedExists {
    pub(crate) outer: JoinSelect,
    pub filters: Vec<JoinExistsFilter>,
    /// Bounded disjunction of conjunctions over `filters`.
    pub expression: Vec<Vec<usize>>,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum InnerRelation {
    Fact,
    Date,
    Outer,
}

fn same_name(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn qualify_inner(mut column: JoinColumn, relation: &str) -> JoinColumn {
    if column.relation.is_empty() {
        column.relation = relation.to_owned();
    }
    column
}

fn contains_name(names: &[String], name: &str) -> bool {
    names.iter().any(|candidate| same_name(candidate, name))
}

fn relation_for(
    column: &JoinColumn,
    table: &str,
    alias: &str,
    fact_columns: &[String],
    date: Option<(&str, &str, &[String])>,
) -> Result<InnerRelation, ParseError> {
    if !column.relation.is_empty() {
        if same_name(&column.relation, table) || same_name(&column.relation, alias) {
            return Ok(InnerRelation::Fact);
        }
        if date.is_some_and(|(table, alias, _)| {
            same_name(&column.relation, table) || same_name(&column.relation, alias)
        }) {
            return Ok(InnerRelation::Date);
        }
        return Ok(InnerRelation::Outer);
    }
    let fact = contains_name(fact_columns, &column.name);
    let date_column = date.is_some_and(|(_, _, names)| contains_name(names, &column.name));
    match (fact, date_column) {
        (true, true) => Err(ParseError(format!(
            "ambiguous JOIN EXISTS column: {}",
            column.name
        ))),
        (true, false) => Ok(InnerRelation::Fact),
        (false, true) => Ok(InnerRelation::Date),
        (false, false) => Ok(InnerRelation::Outer),
    }
}

fn referenced_columns(
    tokens: &[Token],
    fact_columns: &[String],
    date_columns: &[String],
) -> Vec<JoinColumn> {
    let mut columns = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if let (Some(Token::Word(relation) | Token::QuotedWord(relation)), Some(Token::Dot)) =
            (tokens.get(index), tokens.get(index + 1))
            && let Some(Token::Word(name) | Token::QuotedWord(name)) = tokens.get(index + 2)
        {
            columns.push(JoinColumn {
                relation: relation.clone(),
                name: name.clone(),
            });
            index += 3;
            continue;
        }
        if let Some(Token::Word(name) | Token::QuotedWord(name)) = tokens.get(index)
            && (contains_name(fact_columns, name) || contains_name(date_columns, name))
        {
            columns.push(JoinColumn {
                relation: String::new(),
                name: name.clone(),
            });
        }
        index += 1;
    }
    columns
}

fn split_boolean<'a>(tokens: &'a [Token], separator: &str) -> Result<Vec<&'a [Token]>, ParseError> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut between = false;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| ParseError("unbalanced JOIN boolean expression".into()))?;
            }
            token if depth == 0 && keyword(token, "BETWEEN") => between = true,
            token if depth == 0 && keyword(token, "AND") && between => between = false,
            token if depth == 0 && keyword(token, separator) => {
                pieces.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(ParseError("unbalanced JOIN boolean expression".into()));
    }
    pieces.push(&tokens[start..]);
    Ok(pieces)
}

fn strip_outer_parentheses(mut tokens: &[Token]) -> &[Token] {
    loop {
        if !matches!(tokens.first(), Some(Token::LParen))
            || !matches!(tokens.last(), Some(Token::RParen))
        {
            return tokens;
        }
        let mut depth = 0usize;
        let mut wraps_all = true;
        for (index, token) in tokens.iter().enumerate() {
            match token {
                Token::LParen => depth += 1,
                Token::RParen => depth = depth.saturating_sub(1),
                _ => {}
            }
            if depth == 0 && index + 1 != tokens.len() {
                wraps_all = false;
                break;
            }
        }
        if !wraps_all {
            return tokens;
        }
        tokens = &tokens[1..tokens.len() - 1];
    }
}

fn combine_exists_and(
    left: Vec<Vec<usize>>,
    right: Vec<Vec<usize>>,
) -> Result<Vec<Vec<usize>>, ParseError> {
    if left.len().saturating_mul(right.len()) > MAX_EXISTS_GROUPS {
        return Err(ParseError(
            "JOIN EXISTS boolean expression is too large".into(),
        ));
    }
    let mut combined = Vec::with_capacity(left.len() * right.len());
    for a in &left {
        for b in &right {
            if a.len().saturating_add(b.len()) > MAX_EXISTS_GROUP_SIZE {
                return Err(ParseError("JOIN EXISTS group is too large".into()));
            }
            let mut group = a.clone();
            group.extend_from_slice(b);
            combined.push(group);
        }
    }
    Ok(combined)
}

fn parse_exists_expression(
    tokens: &[Token],
    filters: &mut Vec<JoinExistsFilter>,
    resolve: &mut impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<Vec<Vec<usize>>>, ParseError> {
    let checkpoint = filters.len();
    let tokens = strip_outer_parentheses(tokens);
    let or_pieces = split_boolean(tokens, "OR")?;
    if or_pieces.len() > 1 {
        let mut result = Vec::new();
        for piece in or_pieces {
            let Some(groups) = parse_exists_expression(piece, filters, resolve)? else {
                filters.truncate(checkpoint);
                return Ok(None);
            };
            result.extend(groups);
            if result.len() > MAX_EXISTS_GROUPS {
                return Err(ParseError(
                    "JOIN EXISTS boolean expression is too large".into(),
                ));
            }
        }
        return Ok(Some(result));
    }
    let and_pieces = split_boolean(tokens, "AND")?;
    if and_pieces.len() > 1 {
        let mut result = vec![Vec::new()];
        for piece in and_pieces {
            let Some(groups) = parse_exists_expression(piece, filters, resolve)? else {
                filters.truncate(checkpoint);
                return Ok(None);
            };
            result = combine_exists_and(result, groups)?;
        }
        return Ok(Some(result));
    }
    let starts_exists = matches!(tokens.first(), Some(Token::Word(word)) if word.eq_ignore_ascii_case("EXISTS"))
        || (tokens.len() > 1 && keyword(&tokens[0], "NOT") && keyword(&tokens[1], "EXISTS"));
    if !starts_exists {
        return Ok(None);
    }
    if filters.len() == MAX_EXISTS_FILTERS {
        return Err(ParseError(format!(
            "JOIN supports at most {MAX_EXISTS_FILTERS} EXISTS filters"
        )));
    }
    let index = filters.len();
    filters.push(parse_filter(tokens, resolve)?);
    Ok(Some(vec![vec![index]]))
}

fn parse_filter(
    piece: &[Token],
    resolve: &mut impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<JoinExistsFilter, ParseError> {
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
    let date_source = if parser.take(Token::Comma) {
        let date_table = parser.bounded_word()?;
        let date_alias = alias(&mut parser, &date_table)?;
        Some((date_table, date_alias))
    } else {
        None
    };
    if !parser.keyword("WHERE") {
        return Err(ParseError("JOIN EXISTS requires WHERE".into()));
    }
    if !matches!(parser.tokens.last(), Some(Token::RParen)) {
        return Err(ParseError("JOIN EXISTS requires a closing ')'".into()));
    }
    let condition_end = parser.tokens.len() - 1;
    let conditions = split_boolean(&parser.tokens[parser.index..condition_end], "AND")?;
    let fact_columns = resolve(&table)?;
    let date_columns = if let Some((date_table, _)) = &date_source {
        resolve(date_table)?
    } else {
        Vec::new()
    };
    let date_relation = date_source
        .as_ref()
        .map(|(table, alias)| (table.as_str(), alias.as_str(), date_columns.as_slice()));
    let mut equality = None;
    let mut inequality = None;
    let mut inner_comparisons = Vec::new();
    let mut date_keys = None;
    let mut date_predicates = Vec::new();
    for condition in conditions {
        if condition.is_empty() {
            return Err(ParseError("incomplete JOIN EXISTS predicate".into()));
        }
        let columns = referenced_columns(condition, &fact_columns, &date_columns);
        if columns.is_empty() {
            return Err(ParseError("JOIN EXISTS predicate requires a column".into()));
        }
        let sources = columns
            .iter()
            .map(|column| relation_for(column, &table, &name, &fact_columns, date_relation))
            .collect::<Result<Vec<_>, _>>()?;
        if date_source.is_some() && sources.iter().all(|source| *source == InnerRelation::Date) {
            date_predicates.push(condition);
            continue;
        }
        let mut comparison = Parser {
            tokens: condition.to_vec(),
            index: 0,
        };
        let left = projected(&mut comparison)?;
        let operator = comparison.operator()?;
        let right = projected(&mut comparison)?;
        if comparison.index != comparison.tokens.len() || left.name == "*" || right.name == "*" {
            return Err(ParseError("unsupported JOIN EXISTS comparison".into()));
        }
        if date_source.is_none() && (left.relation.is_empty() || right.relation.is_empty()) {
            return Err(ParseError(
                "JOIN EXISTS comparisons require qualified columns".into(),
            ));
        }
        let left_source = relation_for(&left, &table, &name, &fact_columns, date_relation)?;
        let right_source = relation_for(&right, &table, &name, &fact_columns, date_relation)?;
        match (left_source, right_source, operator) {
            (
                InnerRelation::Fact,
                InnerRelation::Fact,
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
                inner_comparisons.push((
                    qualify_inner(left, &name),
                    operator,
                    qualify_inner(right, &name),
                ));
            }
            (InnerRelation::Fact, InnerRelation::Outer, Operator::Eq | Operator::Ne)
            | (InnerRelation::Outer, InnerRelation::Fact, Operator::Eq | Operator::Ne) => {
                let pair = if left_source == InnerRelation::Fact {
                    (qualify_inner(left, &name), right)
                } else {
                    (qualify_inner(right, &name), left)
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
            (InnerRelation::Fact, InnerRelation::Date, Operator::Eq)
            | (InnerRelation::Date, InnerRelation::Fact, Operator::Eq) => {
                if date_keys.is_some() {
                    return Err(ParseError("JOIN EXISTS supports one fact/date key".into()));
                }
                date_keys = Some(if left_source == InnerRelation::Fact {
                    (qualify_inner(left, &name), right)
                } else {
                    (qualify_inner(right, &name), left)
                });
            }
            _ => return Err(ParseError("unsupported JOIN EXISTS comparison".into())),
        }
    }
    let date_filter = match date_source {
        Some((date_table, date_alias)) => {
            let (fact_key, date_key) = date_keys.ok_or_else(|| {
                ParseError("JOIN EXISTS date join requires a key equality".into())
            })?;
            let mut tokens = vec![
                Token::Word("SELECT".into()),
                Token::Word(date_key.name.clone()),
                Token::Word("FROM".into()),
                Token::Word(date_table.clone()),
            ];
            if !date_predicates.is_empty() {
                tokens.push(Token::Word("WHERE".into()));
                for (index, condition) in date_predicates.iter().enumerate() {
                    if index != 0 {
                        tokens.push(Token::Word("AND".into()));
                    }
                    tokens.extend_from_slice(condition);
                }
            }
            let select = super::super::parse_select_tokens(tokens)?;
            Some(JoinExistsDateFilter {
                table: date_table,
                alias: date_alias.clone(),
                keys: (
                    JoinColumn {
                        relation: name.clone(),
                        name: fact_key.name,
                    },
                    JoinColumn {
                        relation: date_alias,
                        name: date_key.name,
                    },
                ),
                select: Box::new(select),
            })
        }
        None => {
            if date_keys.is_some() || !date_predicates.is_empty() {
                return Err(ParseError(
                    "JOIN EXISTS date predicates require a date table".into(),
                ));
            }
            None
        }
    };
    Ok(JoinExistsFilter {
        table,
        alias: name,
        negated,
        equality: equality
            .ok_or_else(|| ParseError("JOIN EXISTS requires an equality correlation".into()))?,
        inequality,
        inner_comparisons,
        date_filter,
    })
}

/// Parse an INNER join with a bounded boolean expression of correlated EXISTS predicates.
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
    let pieces = split_boolean(&tokens[where_index + 1..end], "AND")?;
    let mut filters = Vec::new();
    let mut remaining = Vec::new();
    let mut expression = vec![Vec::new()];
    for piece in pieces {
        if piece.is_empty() {
            return Err(ParseError("incomplete JOIN WHERE".into()));
        }
        if let Some(groups) = parse_exists_expression(piece, &mut filters, &mut resolve)? {
            expression = combine_exists_and(expression, groups)?;
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
        if aliases.iter().any(|alias| same_name(alias, &filter.alias))
            || filter
                .date_filter
                .as_ref()
                .is_some_and(|date| aliases.iter().any(|alias| same_name(alias, &date.alias)))
        {
            return Err(ParseError(
                "JOIN EXISTS inner alias shadows an outer alias".into(),
            ));
        }
        for (_, column) in std::iter::once(&filter.equality).chain(filter.inequality.iter()) {
            if !aliases
                .iter()
                .any(|alias| same_name(alias, &column.relation))
            {
                return Err(ParseError("unknown JOIN EXISTS outer qualifier".into()));
            }
        }
    }
    Ok(Some(JoinCorrelatedExists {
        outer,
        filters,
        expression: if expression == vec![Vec::new()] {
            Vec::new()
        } else {
            expression
        },
    }))
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

    fn tpcds_schema(table: &str) -> Result<Vec<String>, ParseError> {
        let names: &[&str] = match table {
            "customer" => &["c_customer_sk", "c_current_addr_sk"],
            "customer_address" => &["ca_address_sk"],
            "store_sales" => &["ss_customer_sk", "ss_sold_date_sk"],
            "web_sales" => &["ws_bill_customer_sk", "ws_sold_date_sk"],
            "catalog_sales" => &["cs_ship_customer_sk", "cs_sold_date_sk"],
            "date_dim" => &["d_date_sk", "d_year", "d_moy", "d_qoy"],
            _ => return Err(ParseError(format!("unknown table: {table}"))),
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
    fn scalar_exists_keeps_between_and_split_from_following_predicates() {
        let outer_sql = "SELECT COUNT(*) AS n, COUNT(o.o_orderkey) AS present FROM orders o JOIN nation n ON o.o_orderkey = n.n_nationkey JOIN supplier s ON n.n_nationkey = s.s_nationkey WHERE o_orderkey BETWEEN 1 AND 10";
        let outer = super::super::parse_join(outer_sql).unwrap().unwrap();
        assert_eq!(outer.scalar_aggregates.len(), 2);

        let sql = "SELECT COUNT(*) AS n, COUNT(o.o_orderkey) AS present FROM orders o JOIN nation n ON o.o_orderkey = n.n_nationkey JOIN supplier s ON n.n_nationkey = s.s_nationkey WHERE o_orderkey BETWEEN 1 AND 10 AND EXISTS (SELECT * FROM lineitem l WHERE l.l_orderkey = o.o_orderkey)";
        let plan = parse_join_correlated_exists_with_schema(sql, schema)
            .unwrap()
            .unwrap();

        assert_eq!(plan.outer.scalar_aggregates.len(), 2);
        assert_eq!(plan.filters.len(), 1);
        assert!(plan.outer.scalar_where.is_some());
    }

    #[test]
    fn parses_tpcds_fact_date_exists_with_or_and_not_groups() {
        let query = "SELECT c.c_customer_sk, count(*) AS n FROM customer c JOIN customer_address ca ON c.c_current_addr_sk = ca.ca_address_sk WHERE EXISTS (SELECT * FROM store_sales, date_dim WHERE c.c_customer_sk = ss_customer_sk AND ss_sold_date_sk = d_date_sk AND d_year = 2002 AND d_moy BETWEEN 1 AND 1+3) AND (EXISTS (SELECT * FROM web_sales, date_dim WHERE c.c_customer_sk = ws_bill_customer_sk AND ws_sold_date_sk = d_date_sk AND d_year = 2002 AND d_moy BETWEEN 1 AND 1+3) OR EXISTS (SELECT * FROM catalog_sales, date_dim WHERE c.c_customer_sk = cs_ship_customer_sk AND cs_sold_date_sk = d_date_sk AND d_year = 2002 AND d_moy BETWEEN 1 AND 1+3)) GROUP BY c.c_customer_sk";
        let plan = parse_join_correlated_exists_with_schema(query, tpcds_schema)
            .unwrap()
            .unwrap();
        assert_eq!(plan.filters.len(), 3);
        assert_eq!(plan.expression, vec![vec![0, 1], vec![0, 2]]);
        let date = plan.filters[0].date_filter.as_ref().unwrap();
        assert_eq!(date.table, "date_dim");
        assert_eq!(date.keys.0.display_name(), "store_sales.ss_sold_date_sk");
        assert_eq!(date.keys.1.display_name(), "date_dim.d_date_sk");
        assert_eq!(date.select.predicates.len(), 3);

        let negated = query
            .replace(
                "(EXISTS (SELECT * FROM web_sales",
                "(NOT EXISTS (SELECT * FROM web_sales",
            )
            .replace(
                " OR EXISTS (SELECT * FROM catalog_sales",
                " AND NOT EXISTS (SELECT * FROM catalog_sales",
            );
        let plan = parse_join_correlated_exists_with_schema(&negated, tpcds_schema)
            .unwrap()
            .unwrap();
        assert_eq!(plan.expression, vec![vec![0, 1, 2]]);
        assert!(plan.filters[1].negated);
        assert!(plan.filters[2].negated);
    }

    #[test]
    fn keeps_repeated_grouped_counts_in_projection_order() {
        let sql = "SELECT c.c_current_addr_sk, count(*) AS cnt1, c.c_customer_sk, count(*) AS cnt2 FROM customer c, customer_address ca WHERE c.c_current_addr_sk = ca.ca_address_sk GROUP BY c.c_current_addr_sk, c.c_customer_sk";
        let plan = super::super::parse_join_with_schema(sql, tpcds_schema)
            .unwrap()
            .unwrap();
        assert_eq!(
            plan.grouped_output,
            vec![
                super::super::JoinGroupedOutput::Key(0),
                super::super::JoinGroupedOutput::Count("cnt1".into()),
                super::super::JoinGroupedOutput::Key(1),
                super::super::JoinGroupedOutput::Count("cnt2".into()),
            ]
        );
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
