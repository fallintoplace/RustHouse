//! A bounded grouped CTE used by a correlated AVG filter.

use crate::{
    GroupedSum, JoinColumn, JoinCorrelatedAvg, JoinSelect, ParseError, Parser, Token,
    parse_join_correlated_avg_with_schema, parse_join_with_schema, tokenize,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CteOutputColumn {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CteCorrelatedAvg {
    pub(crate) name: String,
    pub(crate) grouped: JoinSelect,
    pub(crate) correlated: JoinCorrelatedAvg,
    pub(crate) columns: Vec<CteOutputColumn>,
}

impl CteCorrelatedAvg {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn grouped(&self) -> &JoinSelect {
        &self.grouped
    }

    pub fn correlated(&self) -> &JoinCorrelatedAvg {
        &self.correlated
    }

    pub fn columns(&self) -> &[CteOutputColumn] {
        &self.columns
    }

    pub fn cap_rows(&mut self, cap: usize) {
        self.correlated.cap_rows(cap);
    }
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn render_token(token: &Token) -> String {
    match token {
        Token::Word(value) | Token::Number(value) => value.clone(),
        Token::QuotedWord(value) | Token::QuotedAlias(value) => quote_identifier(value),
        Token::String(value) => format!("'{}'", value.replace('\'', "''")),
        Token::Star => "*".into(),
        Token::Slash => "/".into(),
        Token::Percent => "%".into(),
        Token::Plus => "+".into(),
        Token::Minus => "-".into(),
        Token::Comma => ",".into(),
        Token::Dot => ".".into(),
        Token::LParen => "(".into(),
        Token::RParen => ")".into(),
        Token::Semicolon => ";".into(),
        Token::Operator(operator) => match operator {
            crate::Operator::Eq => "=".into(),
            crate::Operator::Ne => "!=".into(),
            crate::Operator::Lt => "<".into(),
            crate::Operator::Le => "<=".into(),
            crate::Operator::Gt => ">".into(),
            crate::Operator::Ge => ">=".into(),
            crate::Operator::Modulo(value) => format!("%{value}"),
            crate::Operator::ModuloNe(value) => format!("%{value} !="),
        },
    }
}

fn render_tokens(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(render_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_single_cte(sql: &str) -> Result<Option<(String, String, String)>, ParseError> {
    let tokens = tokenize(sql)?;
    let mut parser = Parser { tokens, index: 0 };
    if !parser.keyword("WITH") {
        return Ok(None);
    }
    let name = parser.bounded_word()?;
    if !parser.keyword("AS") || !parser.take(Token::LParen) {
        return Ok(None);
    }
    let start = parser.index;
    let mut depth = 1usize;
    while parser.index < parser.tokens.len() {
        match parser.tokens[parser.index] {
            Token::LParen => depth += 1,
            Token::RParen => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            break;
        }
        parser.index += 1;
    }
    if depth != 0 {
        return Err(ParseError("CTE requires closing ')'".into()));
    }
    let inner = render_tokens(&parser.tokens[start..parser.index]);
    parser.index += 1;
    if !matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT"))
    {
        return Ok(None);
    }
    let outer = render_tokens(&parser.tokens[parser.index..]);
    Ok(Some((name, inner, outer)))
}

fn sources(query: &JoinSelect) -> Vec<(&str, &str)> {
    let mut sources = vec![
        (query.left.as_str(), query.left_alias.as_str()),
        (query.right.as_str(), query.right_alias.as_str()),
    ];
    sources.extend(
        query
            .steps
            .iter()
            .map(|step| (step.table.as_str(), step.alias.as_str())),
    );
    sources
}

fn source_type(
    query: &JoinSelect,
    column: &JoinColumn,
    resolve: &mut impl FnMut(&str) -> Result<Vec<CteOutputColumn>, ParseError>,
) -> Result<Option<String>, ParseError> {
    let mut found = None;
    for (table, alias) in sources(query) {
        if !column.relation.is_empty() && column.relation != alias && column.relation != table {
            continue;
        }
        let schema = resolve(table)?;
        if let Some(metadata) = schema.iter().find(|item| item.name == column.name) {
            if found.replace(metadata.data_type.clone()).is_some() {
                return Ok(None);
            }
        }
    }
    Ok(found)
}

fn output_schema(
    grouped: &JoinSelect,
    resolve: &mut impl FnMut(&str) -> Result<Vec<CteOutputColumn>, ParseError>,
) -> Result<Option<Vec<CteOutputColumn>>, ParseError> {
    let (Some(keys), Some(grouped_keys), Some(GroupedSum::Column(metric))) = (
        grouped.columns.as_ref(),
        grouped.grouped_count.as_ref(),
        grouped.grouped_sum.as_ref(),
    ) else {
        return Ok(None);
    };
    if keys.len() != grouped_keys.len()
        || grouped.grouped_sum_extras.len() != 0
        || !grouped.grouped_avg_metrics.is_empty()
        || !grouped.grouped_case_counts.is_empty()
        || grouped.grouped_window_ratio.is_some()
        || grouped.grouped_formula.is_some()
    {
        return Ok(None);
    }
    let Some(metric_source_type) = source_type(grouped, metric, resolve)? else {
        return Ok(None);
    };
    let base_metric_type = metric_source_type
        .strip_prefix("Nullable(")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(&metric_source_type);
    if !matches!(base_metric_type, "Float32" | "Float64") {
        return Ok(None);
    }
    let metric_name = grouped
        .aggregate_alias
        .clone()
        .unwrap_or_else(|| format!("sum({})", metric.name));
    let sum_position = grouped.grouped_sum_position.unwrap_or(keys.len());
    if sum_position > keys.len() {
        return Ok(None);
    }

    let mut columns = Vec::with_capacity(keys.len() + 1);
    let mut key_index = 0;
    for position in 0..=keys.len() {
        if position == sum_position {
            columns.push(CteOutputColumn {
                name: metric_name.clone(),
                data_type: "Nullable(Float64)".into(),
            });
            continue;
        }
        let key = &keys[key_index];
        let Some(data_type) = source_type(grouped, key, resolve)? else {
            return Ok(None);
        };
        let name = grouped
            .grouped_key_aliases
            .iter()
            .find_map(|(index, alias)| (*index == key_index).then(|| alias.clone()))
            .unwrap_or_else(|| key.name.clone());
        columns.push(CteOutputColumn { name, data_type });
        key_index += 1;
    }
    Ok(Some(columns))
}

/// Parse a single grouped floating-point SUM CTE referenced by a correlated AVG.
pub fn parse_cte_correlated_avg_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<CteOutputColumn>, ParseError>,
) -> Result<Option<CteCorrelatedAvg>, ParseError> {
    let Some((name, grouped_sql, outer_sql)) = split_single_cte(sql)? else {
        return Ok(None);
    };
    let grouped = match parse_join_with_schema(&grouped_sql, |table| {
        resolve(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
    }) {
        Ok(Some(grouped)) => grouped,
        Ok(None) | Err(_) => return Ok(None),
    };
    if grouped.kind != crate::JoinType::Inner
        || grouped
            .steps
            .iter()
            .any(|step| step.kind != crate::JoinType::Inner)
        || grouped.limit.is_some()
        || grouped.offset != 0
        || !grouped.order_by.is_empty()
        || grouped.grouped_count_column.is_some()
        || grouped.grouped_distinct.is_some()
        || grouped.scalar_sum.is_some()
        || grouped.scalar_formula.is_some()
    {
        return Ok(None);
    }
    let Some(columns) = output_schema(&grouped, &mut resolve)? else {
        return Ok(None);
    };
    let mut outer_resolve = |table: &str| {
        if table.eq_ignore_ascii_case(&name) {
            Ok(columns.iter().map(|column| column.name.clone()).collect())
        } else {
            resolve(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
        }
    };
    let correlated = match parse_join_correlated_avg_with_schema(&outer_sql, &mut outer_resolve) {
        Ok(Some(correlated)) => correlated,
        Ok(None) | Err(_) => return Ok(None),
    };
    let outer_has_cte = sources(correlated.outer())
        .iter()
        .any(|(table, _)| table.eq_ignore_ascii_case(&name));
    if !outer_has_cte
        || !correlated
            .inner_tables()
            .iter()
            .any(|table| table.eq_ignore_ascii_case(&name))
    {
        return Ok(None);
    }
    Ok(Some(CteCorrelatedAvg {
        name,
        grouped,
        correlated,
        columns,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(table: &str) -> Result<Vec<CteOutputColumn>, ParseError> {
        let columns = match table {
            "store_returns" => vec![
                ("sr_customer_sk", "Nullable(Int64)"),
                ("sr_store_sk", "Nullable(Int64)"),
                ("sr_returned_date_sk", "Nullable(Int64)"),
                ("sr_return_amt", "Nullable(Float64)"),
            ],
            "date_dim" => vec![
                ("d_date_sk", "Nullable(Int64)"),
                ("d_year", "Nullable(Int64)"),
            ],
            "store" => vec![
                ("s_store_sk", "Nullable(Int64)"),
                ("s_state", "Nullable(String)"),
            ],
            "customer" => vec![
                ("c_customer_sk", "Nullable(Int64)"),
                ("c_customer_id", "Nullable(String)"),
            ],
            _ => return Err(ParseError(format!("unknown test table: {table}"))),
        };
        Ok(columns
            .into_iter()
            .map(|(name, data_type)| CteOutputColumn {
                name: name.into(),
                data_type: data_type.into(),
            })
            .collect())
    }

    #[test]
    fn parses_grouped_cte_with_composite_key_and_trailing_avg_factor() {
        let sql = "WITH customer_total_return AS (SELECT sr_customer_sk AS ctr_customer_sk, sr_store_sk AS ctr_store_sk, SUM(sr_return_amt) AS ctr_total_return FROM store_returns, date_dim WHERE sr_returned_date_sk = d_date_sk AND d_year = 2000 GROUP BY sr_customer_sk, sr_store_sk) SELECT c_customer_id FROM customer_total_return ctr1, store, customer WHERE ctr1.ctr_total_return > (SELECT AVG(ctr_total_return) * 1.2 FROM customer_total_return ctr2 WHERE ctr1.ctr_store_sk = ctr2.ctr_store_sk) AND s_store_sk = ctr1.ctr_store_sk AND s_state = 'TN' AND ctr1.ctr_customer_sk = c_customer_sk ORDER BY c_customer_id LIMIT 100";
        let plan = parse_cte_correlated_avg_with_schema(sql, schema)
            .unwrap()
            .unwrap();
        assert_eq!(plan.name(), "customer_total_return");
        assert_eq!(plan.columns().len(), 3);
        assert_eq!(plan.columns()[0].name, "ctr_customer_sk");
        assert_eq!(plan.columns()[1].name, "ctr_store_sk");
        assert_eq!(plan.columns()[2].name, "ctr_total_return");
        assert_eq!(plan.correlated().factor, "1.2");
        assert!(plan.correlated().outer().columns.is_some());
        assert_eq!(plan.correlated().inner_tables(), &["customer_total_return"]);
    }

    #[test]
    fn leaves_multi_cte_statements_for_other_query_paths() {
        let sql = "WITH a AS (SELECT x FROM one), b AS (SELECT y FROM two) SELECT x FROM a";
        assert!(
            parse_cte_correlated_avg_with_schema(sql, schema)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn does_not_materialize_integer_sum_as_float_cte() {
        let sql = "WITH totals AS (SELECT sr_customer_sk AS customer_sk, SUM(sr_customer_sk) AS total FROM store_returns GROUP BY sr_customer_sk) SELECT c_customer_id FROM totals t, customer WHERE t.total > (SELECT AVG(total) FROM totals i WHERE t.customer_sk = i.customer_sk) AND t.customer_sk = c_customer_sk";
        assert!(
            parse_cte_correlated_avg_with_schema(sql, schema)
                .unwrap()
                .is_none()
        );
    }
}
