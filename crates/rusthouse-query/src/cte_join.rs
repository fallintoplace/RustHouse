//! Materialized CTEs whose inputs and final query are supported JOIN plans.

use crate::{
    GroupedSum, JoinColumn, JoinSelect, ParseError, Parser, Token, parse_join_with_schema, tokenize,
};

use super::cte_correlated_avg::CteOutputColumn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CteMultiJoinSource {
    name: String,
    query: JoinSelect,
    columns: Vec<CteOutputColumn>,
}

impl CteMultiJoinSource {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn query(&self) -> &JoinSelect {
        &self.query
    }

    pub fn columns(&self) -> &[CteOutputColumn] {
        &self.columns
    }
}

/// A set of materialized JOIN CTEs followed by a JOIN query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CteMultiJoin {
    sources: Vec<CteMultiJoinSource>,
    outer: JoinSelect,
}

impl CteMultiJoin {
    pub fn sources(&self) -> &[CteMultiJoinSource] {
        &self.sources
    }

    pub fn outer(&self) -> &JoinSelect {
        &self.outer
    }

    pub fn cap_rows(&mut self, cap: usize) {
        self.outer.limit = Some(self.outer.limit.map_or(cap, |limit| limit.min(cap)));
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

fn split_ctes(sql: &str) -> Result<Option<(Vec<(String, String)>, String)>, ParseError> {
    let tokens = tokenize(sql)?;
    let mut parser = Parser { tokens, index: 0 };
    if !parser.keyword("WITH") {
        return Ok(None);
    }

    let mut sources = Vec::new();
    loop {
        let Ok(name) = parser.bounded_word() else {
            return Ok(None);
        };
        if sources
            .iter()
            .any(|(existing, _): &(String, String)| existing.eq_ignore_ascii_case(&name))
        {
            return Ok(None);
        }
        if !parser.keyword("AS") || !parser.take(Token::LParen) {
            return Ok(None);
        }

        let start = parser.index;
        let mut depth = 1usize;
        while parser.index < parser.tokens.len() && depth > 0 {
            match parser.tokens[parser.index] {
                Token::LParen => depth += 1,
                Token::RParen => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                parser.index += 1;
            }
        }
        if depth != 0 {
            return Ok(None);
        }
        sources.push((name, render_tokens(&parser.tokens[start..parser.index])));
        parser.index += 1;
        if !parser.take(Token::Comma) {
            break;
        }
    }

    if !matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT"))
    {
        return Ok(None);
    }
    let outer = render_tokens(&parser.tokens[parser.index..]);
    Ok(Some((sources, outer)))
}

fn table_sources(query: &JoinSelect) -> Vec<(&str, &str)> {
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
    mut resolve: impl FnMut(&str) -> Result<Vec<CteOutputColumn>, ParseError>,
) -> Result<Option<String>, ParseError> {
    let mut found = None;
    for (table, alias) in table_sources(query) {
        if !column.relation.is_empty()
            && !column.relation.eq_ignore_ascii_case(alias)
            && !column.relation.eq_ignore_ascii_case(table)
        {
            continue;
        }
        let schema = resolve(table)?;
        if let Some(metadata) = schema
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(&column.name))
        {
            if found.replace(metadata.data_type.clone()).is_some() {
                return Ok(None);
            }
        }
    }
    Ok(found)
}

fn output_schema(
    query: &JoinSelect,
    mut resolve: impl FnMut(&str) -> Result<Vec<CteOutputColumn>, ParseError>,
) -> Result<Option<Vec<CteOutputColumn>>, ParseError> {
    let Some(keys) = query.columns.as_ref() else {
        return Ok(None);
    };
    if keys.is_empty()
        || keys.iter().any(|key| key.name == "*")
        || query.aggregate.is_some()
        || !query.scalar_aggregates.is_empty()
        || query.scalar_sum.is_some()
        || query.scalar_formula.is_some()
        || query.grouped_count_column.is_some()
        || query.grouped_distinct.is_some()
        || !query.grouped_case_counts.is_empty()
        || query.grouped_min.is_some()
        || !query.grouped_min_metrics.is_empty()
        || !query.grouped_max_metrics.is_empty()
        || !query.grouped_avg_metrics.is_empty()
        || query.grouped_window_average.is_some()
        || query.grouped_window_ratio.is_some()
        || query.grouped_formula.is_some()
        || query.grouped_year.is_some()
        || query.grouped_sum_having.is_some()
        || query.grouped_count_having.is_some()
    {
        return Ok(None);
    }

    let has_sum = query.grouped_sum.is_some();
    if has_sum {
        let (Some(grouped_keys), Some(GroupedSum::Column(metric))) =
            (query.grouped_count.as_ref(), query.grouped_sum.as_ref())
        else {
            return Ok(None);
        };
        if keys.len() != grouped_keys.len()
            || query.grouped_sum_extras.len() != 0
            || query.grouped_rollup
            || query.kind != crate::JoinType::Inner
            || query
                .steps
                .iter()
                .any(|step| step.kind != crate::JoinType::Inner)
        {
            return Ok(None);
        }
        let Some(metric_type) = source_type(query, metric, &mut resolve)? else {
            return Ok(None);
        };
        let base_type = metric_type
            .strip_prefix("Nullable(")
            .and_then(|value| value.strip_suffix(')'))
            .unwrap_or(&metric_type);
        if !matches!(base_type, "Float32" | "Float64") {
            return Ok(None);
        }
        let sum_position = query.grouped_sum_position.unwrap_or(keys.len());
        if sum_position > keys.len() {
            return Ok(None);
        }
        let sum_name = query
            .aggregate_alias
            .clone()
            .unwrap_or_else(|| format!("sum({})", metric.name));
        let mut columns = Vec::with_capacity(keys.len() + 1);
        let mut key_index = 0;
        for position in 0..=keys.len() {
            if position == sum_position {
                columns.push(CteOutputColumn {
                    name: sum_name.clone(),
                    data_type: "Nullable(Float64)".into(),
                });
                continue;
            }
            let key = &keys[key_index];
            let Some(data_type) = source_type(query, key, &mut resolve)? else {
                return Ok(None);
            };
            let name = query
                .grouped_key_aliases
                .iter()
                .find_map(|(index, alias)| (*index == key_index).then(|| alias.clone()))
                .unwrap_or_else(|| key.name.clone());
            columns.push(CteOutputColumn { name, data_type });
            key_index += 1;
        }
        return Ok(Some(columns));
    }

    if query.grouped_keys_only {
        let mut columns = Vec::with_capacity(keys.len());
        for (position, key) in keys.iter().enumerate() {
            let Some(data_type) = source_type(query, key, &mut resolve)? else {
                return Ok(None);
            };
            let name = query
                .grouped_key_aliases
                .iter()
                .find_map(|(index, alias)| (*index == position).then(|| alias.clone()))
                .unwrap_or_else(|| key.name.clone());
            if columns
                .iter()
                .any(|column: &CteOutputColumn| column.name.eq_ignore_ascii_case(&name))
            {
                return Ok(None);
            }
            columns.push(CteOutputColumn { name, data_type });
        }
        return Ok(Some(columns));
    }

    if query.grouped_count.is_some()
        || query.grouped_sum.is_some()
        || !query.grouped_sum_extras.is_empty()
        || query.grouped_rollup
        || query.limit.is_some()
        || query.offset != 0
        || !query.order_by.is_empty()
    {
        return Ok(None);
    }

    let mut columns = Vec::with_capacity(keys.len());
    for (position, key) in keys.iter().enumerate() {
        let Some(data_type) = source_type(query, key, &mut resolve)? else {
            return Ok(None);
        };
        let name = query
            .grouped_key_aliases
            .iter()
            .find_map(|(index, alias)| (*index == position).then(|| alias.clone()))
            .unwrap_or_else(|| key.name.clone());
        if columns
            .iter()
            .any(|column: &CteOutputColumn| column.name.eq_ignore_ascii_case(&name))
        {
            return Ok(None);
        }
        columns.push(CteOutputColumn { name, data_type });
    }
    Ok(Some(columns))
}

/// Parse one or more CTE JOIN plans followed by a supported JOIN query.
pub fn parse_cte_multi_join_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<CteOutputColumn>, ParseError>,
) -> Result<Option<CteMultiJoin>, ParseError> {
    let Some((definitions, outer_sql)) = split_ctes(sql)? else {
        return Ok(None);
    };
    let mut sources: Vec<CteMultiJoinSource> = Vec::with_capacity(definitions.len());
    for (name, body) in definitions {
        let parse_result = parse_join_with_schema(&body, |table| {
            if let Some(source) = sources
                .iter()
                .find(|source| source.name.eq_ignore_ascii_case(table))
            {
                Ok(source
                    .columns
                    .iter()
                    .map(|column| column.name.clone())
                    .collect())
            } else {
                resolve(table)
                    .map(|columns| columns.into_iter().map(|column| column.name).collect())
            }
        });
        let Ok(Some(query)) = parse_result else {
            return Ok(None);
        };
        let schema_result = output_schema(&query, |table| {
            if let Some(source) = sources
                .iter()
                .find(|source| source.name.eq_ignore_ascii_case(table))
            {
                Ok(source.columns.clone())
            } else {
                resolve(table)
            }
        });
        let Ok(Some(columns)) = schema_result else {
            return Ok(None);
        };
        sources.push(CteMultiJoinSource {
            name,
            query,
            columns,
        });
    }

    let parse_result = parse_join_with_schema(&outer_sql, |table| {
        if let Some(source) = sources
            .iter()
            .find(|source| source.name.eq_ignore_ascii_case(table))
        {
            Ok(source
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect())
        } else {
            resolve(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
        }
    });
    let Ok(Some(outer)) = parse_result else {
        return Ok(None);
    };
    if outer.limit.is_none() && outer.offset != 0 {
        return Ok(None);
    }
    Ok(Some(CteMultiJoin { sources, outer }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(table: &str) -> Result<Vec<CteOutputColumn>, ParseError> {
        let columns = match table {
            "store_sales" => vec![
                ("ss_addr_sk", "Nullable(Int64)"),
                ("ss_customer_sk", "Nullable(Int64)"),
                ("ss_item_sk", "Nullable(Int64)"),
                ("ss_sold_date_sk", "Nullable(Int64)"),
                ("ss_ext_sales_price", "Nullable(Float64)"),
            ],
            "web_sales" => vec![
                ("ws_addr_sk", "Nullable(Int64)"),
                ("ws_sold_date_sk", "Nullable(Int64)"),
                ("ws_ext_sales_price", "Nullable(Float64)"),
            ],
            "date_dim" => vec![
                ("d_date_sk", "Nullable(Int64)"),
                ("d_qoy", "Nullable(Int64)"),
                ("d_year", "Nullable(Int64)"),
                ("d_month_seq", "Nullable(Int64)"),
            ],
            "customer_address" => vec![
                ("ca_address_sk", "Nullable(Int64)"),
                ("ca_county", "Nullable(String)"),
            ],
            _ => return Err(ParseError(format!("unknown test table {table}"))),
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
    fn parses_multiple_grouped_sum_ctes_and_outer_join() {
        let sql = "WITH ss AS (SELECT ca_county, d_qoy, d_year, SUM(ss_ext_sales_price) AS store_sales FROM store_sales, date_dim, customer_address WHERE ss_sold_date_sk = d_date_sk AND ss_addr_sk = ca_address_sk GROUP BY ca_county, d_qoy, d_year), ws AS (SELECT ca_county, d_qoy, d_year, SUM(ws_ext_sales_price) AS web_sales FROM web_sales, date_dim, customer_address WHERE ws_sold_date_sk = d_date_sk AND ws_addr_sk = ca_address_sk GROUP BY ca_county, d_qoy, d_year) SELECT ss.ca_county, ss.store_sales, ws.web_sales FROM ss, ws WHERE ss.ca_county = ws.ca_county ORDER BY ss.ca_county";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("supported grouped CTE query");

        assert_eq!(plan.sources().len(), 2);
        assert_eq!(plan.sources()[0].name(), "ss");
        assert_eq!(
            plan.sources()[0]
                .columns()
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            ["ca_county", "d_qoy", "d_year", "store_sales"]
        );
        assert_eq!(
            plan.sources()[0].columns()[3].data_type,
            "Nullable(Float64)"
        );
        assert_eq!(plan.outer().left, "ss");
        assert_eq!(plan.outer().right, "ws");
    }

    #[test]
    fn resolves_a_later_cte_from_an_earlier_cte() {
        let sql = "WITH ss AS (SELECT ca_county, d_qoy, d_year, SUM(ss_ext_sales_price) AS store_sales FROM store_sales, date_dim, customer_address WHERE ss_sold_date_sk = d_date_sk AND ss_addr_sk = ca_address_sk GROUP BY ca_county, d_qoy, d_year), ss_copy AS (SELECT ss.ca_county, ss.store_sales FROM ss, date_dim WHERE ss.d_year = d_date_sk) SELECT ss.ca_county, ss_copy.store_sales FROM ss, ss_copy WHERE ss.ca_county = ss_copy.ca_county";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("dependent CTE query");

        assert_eq!(plan.sources().len(), 2);
        assert_eq!(plan.sources()[1].name(), "ss_copy");
        assert_eq!(plan.sources()[1].columns()[1].name, "store_sales");
    }

    #[test]
    fn parses_grouped_ctes_with_case_comparison_in_the_outer_query() {
        let sql = "WITH ss AS (SELECT ca_county, d_qoy, d_year, SUM(ss_ext_sales_price) AS store_sales FROM store_sales, date_dim, customer_address WHERE ss_sold_date_sk = d_date_sk AND ss_addr_sk = ca_address_sk GROUP BY ca_county, d_qoy, d_year), ws AS (SELECT ca_county, d_qoy, d_year, SUM(ws_ext_sales_price) AS web_sales FROM web_sales, date_dim, customer_address WHERE ws_sold_date_sk = d_date_sk AND ws_addr_sk = ca_address_sk GROUP BY ca_county, d_qoy, d_year) SELECT ss1.ca_county, ss1.d_year, (ws2.web_sales * 1.0) / ws1.web_sales AS web_growth FROM ss ss1, ss ss2, ws ws1, ws ws2 WHERE ss1.ca_county = ss2.ca_county AND ws1.ca_county = ws2.ca_county AND ss1.ca_county = ws1.ca_county AND CASE WHEN ws1.web_sales > 0 THEN (ws2.web_sales * 1.0) / ws1.web_sales ELSE NULL END > CASE WHEN ss1.store_sales > 0 THEN (ss2.store_sales * 1.0) / ss1.store_sales ELSE NULL END ORDER BY ss1.ca_county";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("supported grouped CTE query");

        assert_eq!(plan.sources().len(), 2);
        assert_eq!(plan.outer().scalar_projections.len(), 1);
        assert_eq!(plan.outer().scalar_projections[0].alias, "web_growth");
        assert_eq!(
            plan.outer().projection_order,
            [
                crate::JoinProjectionOrder::Column(0),
                crate::JoinProjectionOrder::Column(1),
                crate::JoinProjectionOrder::Scalar(0),
            ]
        );
        assert!(plan.outer().scalar_where.is_some());
    }

    #[test]
    fn leaves_union_ctes_for_a_union_plan() {
        let sql = "WITH combined AS (SELECT ca_county FROM customer_address UNION ALL SELECT ca_county FROM customer_address) SELECT a.ca_county, b.ca_county FROM customer_address a, customer_address b WHERE a.ca_county = b.ca_county";
        assert!(
            parse_cte_multi_join_with_schema(sql, schema)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn infers_key_only_cte_columns_from_a_two_table_join() {
        let sql = "WITH ssci AS (SELECT ss_customer_sk AS customer_sk, ss_item_sk AS item_sk FROM store_sales, date_dim WHERE ss_sold_date_sk = d_date_sk AND d_month_seq BETWEEN 1200 AND 1211 GROUP BY ss_customer_sk, ss_item_sk) SELECT ssci.customer_sk, ssci.item_sk FROM ssci, ssci other WHERE ssci.customer_sk = other.customer_sk ORDER BY ssci.customer_sk LIMIT 100";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("key-only CTE query");

        assert_eq!(plan.sources().len(), 1);
        assert_eq!(
            plan.sources()[0]
                .columns()
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            ["customer_sk", "item_sk"]
        );
        assert!(plan.sources()[0].query().grouped_keys_only);
    }
}
