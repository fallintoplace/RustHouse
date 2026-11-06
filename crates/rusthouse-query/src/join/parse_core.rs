//! Keep JOIN AST, validation helpers, and the large JOIN parser in separate modules.

use super::*;

pub(super) fn parse_join_tokens_internal(
    tokens: Vec<Token>,
    scalar_subquery: bool,
    allow_two_table_grouped_avg: bool,
    allow_ordered_comparison_filters: bool,
) -> Result<Option<JoinSelect>, ParseError> {
    if !matches!(tokens.first(), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT")) {
        return Ok(None);
    }
    // A JOIN inside a derived table or EXISTS is not an outer JOIN.
    // Inspect only top-level keywords before selecting the JOIN parser.
    let mut depth = 0usize;
    let mut seen_from = false;
    let mut outer_join = false;
    for token in &tokens {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("FROM") => {
                seen_from = true;
            }
            Token::Word(word) if depth == 0 && seen_from && word.eq_ignore_ascii_case("JOIN") => {
                outer_join = true;
                break;
            }
            _ => {}
        }
    }
    if !outer_join {
        return Ok(None);
    }
    let mut parser = Parser { tokens, index: 0 };
    parser.keyword("SELECT");
    parser.keyword("ALL");
    let mut scalar_sum = None;
    let mut scalar_sum_qualifier = None;
    let mut aggregate_distinct = false;
    let starts_with_grouped_case_sums = matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("SUM"))
        && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
        && matches!(parser.tokens.get(parser.index + 2), Some(Token::Word(name)) if name.eq_ignore_ascii_case("CASE"));
    let parsed_formula = parse_join_formula(&mut parser)?;
    let scalar_formula = parsed_formula.as_ref().map(|(formula, _)| formula.clone());
    let mut aggregate = if scalar_formula.is_some() {
        None
    } else if starts_with_grouped_case_sums {
        None
    } else if parser.keyword("COUNT") {
        if !parser.take(Token::LParen) {
            return Err(ParseError("JOIN COUNT requires '('".into()));
        }
        aggregate_distinct = parser.keyword("DISTINCT");
        let result = if !aggregate_distinct && parser.take(Token::Star) {
            JoinAggregate::CountStar
        } else if !aggregate_distinct
            && matches!(
                parser.tokens.get(parser.index),
                Some(Token::Number(value)) if value == "0" || value == "1"
            )
        {
            parser.index += 1;
            JoinAggregate::CountStar
        } else {
            let column = projected(&mut parser)?;
            if column.name == "*" {
                return Err(ParseError("JOIN COUNT requires a column".into()));
            }
            JoinAggregate::CountColumn(column)
        };
        if !parser.take(Token::RParen) {
            return Err(ParseError("JOIN COUNT requires ')'".into()));
        }
        Some(result)
    } else {
        let function = if parser.keyword("SUM") {
            Some(crate::AggregateFunction::Sum)
        } else if parser.keyword("AVG") {
            Some(crate::AggregateFunction::Avg)
        } else if parser.keyword("MIN") {
            Some(crate::AggregateFunction::Min)
        } else if parser.keyword("MAX") {
            Some(crate::AggregateFunction::Max)
        } else {
            None
        };
        if let Some(function) = function {
            if !parser.take(Token::LParen) {
                return Err(ParseError("JOIN aggregate requires '('".into()));
            }
            let start = parser.index;
            let column = projected(&mut parser).ok().filter(|column| {
                column.name != "*" && parser.tokens.get(parser.index) == Some(&Token::RParen)
            });
            if let Some(column) = column {
                parser.take(Token::RParen);
                Some(JoinAggregate::Numeric { function, column })
            } else if function == crate::AggregateFunction::Sum {
                parser.index = start;
                let expression = crate::expression::parse_expression_tokens(&mut parser)?;
                if !numeric_join_expression(&expression) || !parser.take(Token::RParen) {
                    return Err(ParseError(
                        "JOIN SUM requires a numeric arithmetic expression".into(),
                    ));
                }
                scalar_sum = Some(expression);
                None
            } else {
                return Err(ParseError("JOIN aggregate requires a column".into()));
            }
        } else {
            None
        }
    };
    if scalar_subquery {
        if let Some(JoinAggregate::Numeric {
            function: crate::AggregateFunction::Sum,
            column,
        }) = &aggregate
        {
            scalar_sum = Some(crate::ScalarExpr::Column(column.name.clone()));
            if !column.relation.is_empty() {
                scalar_sum_qualifier = Some(column.relation.clone());
            }
            aggregate = None;
        }
    }
    let mut aggregate_alias = if let Some((_, alias)) = parsed_formula {
        Some(alias)
    } else if aggregate.is_some() || scalar_sum.is_some() {
        parser.select_output_alias()?
    } else {
        None
    };
    let mut scalar_aggregates = Vec::new();
    if matches!(parser.tokens.get(parser.index), Some(Token::Comma)) {
        let first = aggregate
            .clone()
            .ok_or_else(|| ParseError("multiple JOIN aggregates must use simple columns".into()))?;
        let alias = aggregate_alias
            .take()
            .unwrap_or_else(|| join_scalar_aggregate_header(&first, aggregate_distinct));
        scalar_aggregates.push(JoinScalarAggregate {
            aggregate: first,
            alias,
            distinct: aggregate_distinct,
        });
        while parser.take(Token::Comma) {
            if scalar_aggregates.len() == 16 {
                return Err(ParseError(
                    "JOIN supports at most 16 scalar aggregates".into(),
                ));
            }
            let (next, distinct) = if parser.keyword("COUNT") {
                if !parser.take(Token::LParen) {
                    return Err(ParseError("JOIN COUNT requires '('".into()));
                }
                let distinct = parser.keyword("DISTINCT");
                let aggregate = if !distinct && parser.take(Token::Star) {
                    JoinAggregate::CountStar
                } else {
                    let column = projected(&mut parser)?;
                    if column.name == "*" {
                        return Err(ParseError("JOIN COUNT requires a column".into()));
                    }
                    JoinAggregate::CountColumn(column)
                };
                if !parser.take(Token::RParen) {
                    return Err(ParseError("JOIN COUNT requires ')'".into()));
                }
                (aggregate, distinct)
            } else {
                let function = if parser.keyword("SUM") {
                    Some(crate::AggregateFunction::Sum)
                } else if parser.keyword("AVG") {
                    Some(crate::AggregateFunction::Avg)
                } else if parser.keyword("MIN") {
                    Some(crate::AggregateFunction::Min)
                } else if parser.keyword("MAX") {
                    Some(crate::AggregateFunction::Max)
                } else {
                    None
                }
                .ok_or_else(|| {
                    ParseError("multiple JOIN projections must be scalar aggregates".into())
                })?;
                if !parser.take(Token::LParen) {
                    return Err(ParseError("JOIN aggregate requires '('".into()));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("JOIN aggregate requires one column".into()));
                }
                (JoinAggregate::Numeric { function, column }, false)
            };
            let alias = parser
                .select_output_alias()?
                .unwrap_or_else(|| join_scalar_aggregate_header(&next, distinct));
            scalar_aggregates.push(JoinScalarAggregate {
                aggregate: next,
                alias,
                distinct,
            });
        }
        aggregate = None;
        scalar_sum = None;
        scalar_sum_qualifier = None;
    }
    if aggregate_distinct && scalar_aggregates.is_empty() {
        return Err(ParseError(
            "JOIN COUNT DISTINCT requires a scalar aggregate list".into(),
        ));
    }
    let mut grouped_count_projection = false;
    let mut grouped_count_column = None;
    let mut grouped_distinct = None;
    let mut grouped_case_counts = Vec::new();
    let mut grouped_sum = None;
    let mut grouped_sum_extras = Vec::new();
    let mut grouped_min = None;
    let mut grouped_min_metrics = Vec::new();
    let mut grouped_max_metrics = Vec::new();
    let mut grouped_avg_metrics = Vec::new();
    let mut grouped_window_average = None;
    let mut grouped_rollup = false;
    let mut grouped_formula = None;
    let mut grouped_window_ratio = None;
    let mut grouped_year: Option<(usize, String)> = None;
    let mut grouped_key_aliases: Vec<(usize, String)> = Vec::new();
    let mut grouped_sum_position = None;
    let mut grouped_output_candidate = Vec::new();
    let mut scalar_projections = Vec::new();
    let mut projection_order = Vec::new();
    if starts_with_grouped_case_sums {
        parse_global_case_metrics(&mut parser, &mut grouped_case_counts)?;
        grouped_count_projection = true;
    }
    let columns = if starts_with_grouped_case_sums {
        Some(Vec::new())
    } else if aggregate.is_some()
        || !scalar_aggregates.is_empty()
        || scalar_sum.is_some()
        || scalar_formula.is_some()
        || parser.take(Token::Star)
    {
        None
    } else {
        let mut columns = Vec::new();
        loop {
            if grouped_window_ratio.is_none() {
                if let Some(ratio) = parse_grouped_window_ratio(&mut parser, grouped_sum.as_ref())?
                {
                    grouped_output_candidate
                        .push(JoinGroupedOutput::WindowRatio(ratio.alias.clone()));
                    grouped_window_ratio = Some(ratio);
                    grouped_count_projection = true;
                    break;
                }
            }
            if grouped_sum.is_some()
                && matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("SUM"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                if matches!(parser.tokens.get(parser.index + 2), Some(Token::Word(name)) if name.eq_ignore_ascii_case("CASE"))
                {
                    return Err(ParseError(
                        "JOIN cannot mix grouped SUM and CASE metrics".into(),
                    ));
                }
                if grouped_sum_extras.len() == 8 {
                    return Err(ParseError(
                        "JOIN supports at most 9 grouped SUM metrics".into(),
                    ));
                }
                let (metric, alias) = parse_grouped_sum_projection(&mut parser)?;
                let duplicate_alias = aggregate_alias
                    .as_ref()
                    .is_some_and(|previous| previous.eq_ignore_ascii_case(&alias))
                    || grouped_sum_extras
                        .iter()
                        .any(|(_, previous): &(GroupedSum, String)| {
                            previous.eq_ignore_ascii_case(&alias)
                        })
                    || columns
                        .iter()
                        .any(|column: &JoinColumn| column.name.eq_ignore_ascii_case(&alias))
                    || grouped_key_aliases
                        .iter()
                        .any(|(_, previous)| previous.eq_ignore_ascii_case(&alias));
                if duplicate_alias {
                    return Err(ParseError(
                        "duplicate or ambiguous grouped SUM alias".into(),
                    ));
                }
                grouped_sum_extras.push((metric, alias));
                grouped_output_candidate.push(JoinGroupedOutput::SumExtra(
                    grouped_sum_extras.len() - 1,
                    grouped_sum_extras
                        .last()
                        .expect("grouped SUM just pushed")
                        .1
                        .clone(),
                ));
                grouped_count_projection = true;
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            if let Some(window_average) = parse_grouped_window_average(&mut parser)? {
                if grouped_window_average.is_some() {
                    return Err(ParseError("JOIN supports one grouped window AVG".into()));
                }
                if grouped_output_alias_exists(
                    &grouped_output_candidate,
                    &grouped_key_aliases,
                    &columns,
                    &window_average.alias,
                ) {
                    return Err(ParseError(
                        "duplicate or ambiguous grouped window AVG alias".into(),
                    ));
                }
                grouped_output_candidate.push(JoinGroupedOutput::WindowAverage(
                    window_average.alias.clone(),
                ));
                grouped_window_average = Some(window_average);
                grouped_count_projection = true;
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("MIN"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                parser.keyword("MIN");
                if grouped_sum.is_some() || grouped_formula.is_some() {
                    return Err(ParseError("JOIN cannot mix grouped MIN with SUM".into()));
                }
                if grouped_min_metrics.len() == 8 {
                    return Err(ParseError(
                        "JOIN supports at most 8 grouped MIN metrics".into(),
                    ));
                }
                if !parser.take(Token::LParen) {
                    return Err(ParseError("grouped JOIN MIN requires '('".into()));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("grouped JOIN MIN requires one column".into()));
                }
                let alias = parser
                    .select_output_alias()?
                    .unwrap_or_else(|| format!("min({})", column.display_name()));
                if grouped_output_alias_exists(
                    &grouped_output_candidate,
                    &grouped_key_aliases,
                    &columns,
                    &alias,
                ) {
                    return Err(ParseError(
                        "duplicate or ambiguous grouped MIN alias".into(),
                    ));
                }
                let metric_index = grouped_min_metrics.len();
                if grouped_min.is_none() {
                    grouped_min = Some(column.clone());
                    aggregate_alias.get_or_insert_with(|| alias.clone());
                    grouped_sum_position.get_or_insert(columns.len());
                }
                grouped_min_metrics.push((column, alias.clone()));
                grouped_output_candidate.push(JoinGroupedOutput::Min(metric_index, alias));
                grouped_count_projection = true;
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("MAX"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                parser.keyword("MAX");
                if grouped_sum.is_some() || grouped_formula.is_some() {
                    return Err(ParseError("JOIN cannot mix grouped MAX with SUM".into()));
                }
                if grouped_max_metrics.len() == 8 {
                    return Err(ParseError(
                        "JOIN supports at most 8 grouped MAX metrics".into(),
                    ));
                }
                if !parser.take(Token::LParen) {
                    return Err(ParseError("grouped JOIN MAX requires '('".into()));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("grouped JOIN MAX requires one column".into()));
                }
                let alias = parser
                    .select_output_alias()?
                    .unwrap_or_else(|| format!("max({})", column.display_name()));
                if grouped_output_alias_exists(
                    &grouped_output_candidate,
                    &grouped_key_aliases,
                    &columns,
                    &alias,
                ) {
                    return Err(ParseError(
                        "duplicate or ambiguous grouped MAX alias".into(),
                    ));
                }
                let metric_index = grouped_max_metrics.len();
                grouped_max_metrics.push((column, alias.clone()));
                grouped_output_candidate.push(JoinGroupedOutput::Max(metric_index, alias));
                grouped_count_projection = true;
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("AVG"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                if grouped_avg_metrics.len() == 8 {
                    return Err(ParseError(
                        "JOIN supports at most 8 grouped AVG metrics".into(),
                    ));
                }
                parser.keyword("AVG");
                if !parser.take(Token::LParen) {
                    return Err(ParseError("grouped JOIN AVG requires '('".into()));
                }
                let column = projected_or_cast(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("grouped JOIN AVG requires one column".into()));
                }
                let alias = parser
                    .select_output_alias()?
                    .unwrap_or_else(|| format!("avg({})", column.display_name()));
                if grouped_output_alias_exists(
                    &grouped_output_candidate,
                    &grouped_key_aliases,
                    &columns,
                    &alias,
                ) {
                    return Err(ParseError(
                        "duplicate or ambiguous grouped AVG alias".into(),
                    ));
                }
                let metric_index = grouped_avg_metrics.len();
                grouped_avg_metrics.push((column, alias.clone()));
                grouped_output_candidate.push(JoinGroupedOutput::Average(metric_index, alias));
                grouped_count_projection = true;
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            if columns.len() + scalar_projections.len() == 64 {
                return Err(ParseError(
                    "JOIN supports at most 64 projected columns".into(),
                ));
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("EXTRACT"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                parser.keyword("EXTRACT");
                if grouped_year.is_some()
                    || !parser.take(Token::LParen)
                    || !parser.keyword("YEAR")
                    || !parser.keyword("FROM")
                {
                    return Err(ParseError(
                        "JOIN supports one EXTRACT(YEAR FROM column)".into(),
                    ));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) || !parser.keyword("AS") {
                    return Err(ParseError(
                        "JOIN EXTRACT YEAR requires a column and alias".into(),
                    ));
                }
                let alias = parser.output_alias_word()?;
                grouped_year = Some((columns.len(), alias));
                grouped_output_candidate.push(JoinGroupedOutput::Key(columns.len()));
                projection_order.push(JoinProjectionOrder::Column(columns.len()));
                columns.push(column);
            } else if scalar_projection_ahead(&parser) {
                let (encoded, original_ends) =
                    encode_qualified_expression_tokens(&parser.tokens[parser.index..]);
                let mut expression_parser = Parser {
                    tokens: encoded,
                    index: 0,
                };
                let expression =
                    crate::expression::parse_expression_tokens(&mut expression_parser)?;
                if expression_parser.index == 0 {
                    return Err(ParseError(
                        "JOIN scalar projection requires an expression".into(),
                    ));
                }
                let consumed = original_ends
                    .get(expression_parser.index.saturating_sub(1))
                    .copied()
                    .ok_or_else(|| {
                        ParseError("JOIN scalar projection requires an expression".into())
                    })?;
                parser.index += consumed;
                let alias = parser
                    .select_output_alias()?
                    .unwrap_or_else(|| implicit_scalar_projection_alias(&expression));
                let index = scalar_projections.len();
                scalar_projections.push(JoinScalarProjection { expression, alias });
                projection_order.push(JoinProjectionOrder::Scalar(index));
            } else {
                let column = projected(&mut parser)?;
                if let Some(alias) = parser.select_output_alias()? {
                    if column.name == "*" {
                        return Err(ParseError("JOIN group alias requires a column".into()));
                    }
                    grouped_key_aliases.push((columns.len(), alias));
                }
                grouped_output_candidate.push(JoinGroupedOutput::Key(columns.len()));
                projection_order.push(JoinProjectionOrder::Column(columns.len()));
                columns.push(column);
            }
            if !parser.take(Token::Comma) {
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if ["MIN", "MAX", "AVG"].iter().any(|metric| name.eq_ignore_ascii_case(metric)))
            {
                continue;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("AVG"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                loop {
                    if grouped_avg_metrics.len() == 8 {
                        return Err(ParseError(
                            "JOIN supports at most 8 grouped AVG metrics".into(),
                        ));
                    }
                    if !parser.keyword("AVG") || !parser.take(Token::LParen) {
                        return Err(ParseError("grouped JOIN expects AVG(".into()));
                    }
                    let column = projected_or_cast(&mut parser)?;
                    if column.name == "*" || !parser.take(Token::RParen) {
                        return Err(ParseError("grouped JOIN AVG requires one column".into()));
                    }
                    let alias = parser
                        .select_output_alias()?
                        .unwrap_or_else(|| format!("avg({})", column.display_name()));
                    if grouped_avg_metrics
                        .iter()
                        .any(|(_, previous): &(JoinColumn, String)| {
                            previous.eq_ignore_ascii_case(&alias)
                        })
                        || grouped_key_aliases
                            .iter()
                            .any(|(_, key)| key.eq_ignore_ascii_case(&alias))
                    {
                        return Err(ParseError(
                            "duplicate or ambiguous grouped AVG alias".into(),
                        ));
                    }
                    grouped_avg_metrics.push((column, alias));
                    if !parser.take(Token::Comma) {
                        break;
                    }
                    if !matches!(parser.tokens.get(parser.index), Some(Token::Word(name))
                        if name.eq_ignore_ascii_case("AVG"))
                    {
                        return Err(ParseError(
                            "grouped AVG projection expects another AVG".into(),
                        ));
                    }
                }
                grouped_count_projection = true;
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("SUM"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                if let Some((formula, name)) = parse_join_formula(&mut parser)? {
                    if grouped_sum.is_some() || grouped_min.is_some() || grouped_formula.is_some() {
                        return Err(ParseError("JOIN supports one grouped aggregate".into()));
                    }
                    grouped_formula = Some(formula);
                    aggregate_alias = Some(name);
                    grouped_sum_position = Some(columns.len());
                    grouped_count_projection = true;
                    break;
                }
                if grouped_sum.is_some() || grouped_min.is_some() || grouped_formula.is_some() {
                    return Err(ParseError("JOIN supports one grouped aggregate".into()));
                }
                parser.keyword("SUM");
                if !matches!(parser.tokens.get(parser.index + 1), Some(Token::Word(word)) if word.eq_ignore_ascii_case("CASE"))
                {
                    if !parser.take(Token::LParen) {
                        return Err(ParseError("JOIN SUM requires '('".into()));
                    }
                    let start = parser.index;
                    let column = projected(&mut parser).ok().filter(|column| {
                        column.name != "*"
                            && parser.tokens.get(parser.index) == Some(&Token::RParen)
                    });
                    let metric = if let Some(column) = column {
                        GroupedSum::Column(column)
                    } else {
                        parser.index = start;
                        let expression = crate::expression::parse_expression_tokens(&mut parser)?;
                        if !numeric_join_expression(&expression) {
                            return Err(ParseError(
                                "JOIN SUM requires a numeric arithmetic expression".into(),
                            ));
                        }
                        GroupedSum::Expression(expression)
                    };
                    if !parser.take(Token::RParen) {
                        return Err(ParseError("JOIN grouped SUM requires ')'".into()));
                    }
                    let name = match &metric {
                        GroupedSum::Column(column) => column.display_name(),
                        GroupedSum::Expression(expression) => {
                            crate::multi_aggregate::render_expr(expression)
                        }
                    };
                    let alias = parser
                        .select_output_alias()?
                        .unwrap_or_else(|| format!("sum({name})"));
                    grouped_output_candidate.push(JoinGroupedOutput::Sum(alias.clone()));
                    aggregate_alias = Some(alias);
                    grouped_sum = Some(metric);
                    grouped_sum_position = Some(columns.len());
                    grouped_count_projection = true;
                    if parser.take(Token::Comma) {
                        continue;
                    }
                    break;
                }
                loop {
                    if grouped_case_counts.len() >= MAX_JOIN_GROUPED_CASE_METRICS {
                        return Err(ParseError(format!(
                            "JOIN supports at most {MAX_JOIN_GROUPED_CASE_METRICS} grouped CASE metrics"
                        )));
                    }
                    if !parser.take(Token::LParen) {
                        return Err(ParseError("JOIN SUM requires '('".into()));
                    }
                    let (metric, filter) =
                        super::super::grouped_metric_value(&mut parser, crate::AggregateFunction::Sum)?;
                    if !matches!(
                        filter,
                        Some(
                            crate::GroupedFilter::CaseElseZero { .. }
                                | crate::GroupedFilter::CaseElseZeroCombined { .. }
                                | crate::GroupedFilter::CaseExpression { .. }
                                | crate::GroupedFilter::CaseNull { .. }
                                | crate::GroupedFilter::Comparison { .. }
                        )
                    ) {
                        return Err(ParseError(
                            "JOIN grouped SUM requires a supported CASE expression".into(),
                        ));
                    }
                    if !parser.take(Token::RParen) {
                        return Err(ParseError(
                            "JOIN grouped CASE requires a closing ')'".into(),
                        ));
                    }
                    parser.keyword("AS");
                    grouped_case_counts.push((
                        filter.expect("CASE filter checked"),
                        grouped_case_value(metric),
                        parser.output_alias_word()?,
                    ));
                    if !parser.take(Token::Comma) {
                        break;
                    }
                    if parser.keyword("COUNT") {
                        if !parser.take(Token::LParen)
                            || !parser.take(Token::Star)
                            || !parser.take(Token::RParen)
                        {
                            return Err(ParseError(
                                "JOIN CASE metrics can be followed only by COUNT(*)".into(),
                            ));
                        }
                        let alias = parser
                            .select_output_alias()?
                            .unwrap_or_else(|| "count(*)".into());
                        if grouped_case_counts
                            .iter()
                            .any(|(_, _, name)| name.eq_ignore_ascii_case(&alias))
                            || grouped_key_aliases
                                .iter()
                                .any(|(_, name)| name.eq_ignore_ascii_case(&alias))
                        {
                            return Err(ParseError(
                                "duplicate or ambiguous grouped CASE alias".into(),
                            ));
                        }
                        grouped_case_counts.push((
                            crate::GroupedFilter::CaseExpression {
                                condition: crate::ScalarExpr::Compare(
                                    Box::new(crate::ScalarExpr::Integer(1)),
                                    crate::Operator::Eq,
                                    Box::new(crate::ScalarExpr::Integer(1)),
                                ),
                                else_zero: true,
                            },
                            GroupedCaseValue::Count,
                            alias,
                        ));
                        break;
                    }
                    if !parser.keyword("SUM") {
                        return Err(ParseError("JOIN grouped CASE expects SUM metrics".into()));
                    }
                }
                grouped_count_projection = true;
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("MIN"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                parser.keyword("MIN");
                if grouped_sum.is_some() || grouped_min.is_some() || grouped_formula.is_some() {
                    return Err(ParseError("JOIN supports one grouped aggregate".into()));
                }
                if !parser.take(Token::LParen) {
                    return Err(ParseError("grouped JOIN MIN requires '('".into()));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("grouped JOIN MIN requires one column".into()));
                }
                aggregate_alias = Some(
                    parser
                        .select_output_alias()?
                        .unwrap_or_else(|| format!("min({})", column.display_name())),
                );
                grouped_min = Some(column);
                grouped_sum_position = Some(columns.len());
                grouped_count_projection = true;
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            if parser.keyword("COUNT") {
                if grouped_sum.is_some() || grouped_formula.is_some() {
                    return Err(ParseError("JOIN supports one grouped aggregate".into()));
                }
                if !parser.take(Token::LParen) {
                    return Err(ParseError("grouped JOIN COUNT requires '('".into()));
                }
                if parser.keyword("DISTINCT") {
                    let column = projected(&mut parser)?;
                    if column.name == "*" || !parser.take(Token::RParen) {
                        return Err(ParseError(
                            "grouped JOIN COUNT DISTINCT requires a column".into(),
                        ));
                    }
                    aggregate_alias = Some(format!("count(DISTINCT {})", column.display_name()));
                    grouped_distinct = Some(column);
                } else if parser.take(Token::Star) {
                    if !parser.take(Token::RParen) {
                        return Err(ParseError("grouped JOIN COUNT requires ')'".into()));
                    }
                    let alias = parser
                        .select_output_alias()?
                        .unwrap_or_else(|| "count(*)".into());
                    if grouped_output_alias_exists(
                        &grouped_output_candidate,
                        &grouped_key_aliases,
                        &columns,
                        &alias,
                    ) {
                        return Err(ParseError(
                            "duplicate or ambiguous grouped COUNT alias".into(),
                        ));
                    }
                    if aggregate_alias.is_none() {
                        aggregate_alias = Some(alias.clone());
                    }
                    grouped_output_candidate.push(JoinGroupedOutput::Count(alias));
                    grouped_count_projection = true;
                    if parser.take(Token::Comma) {
                        continue;
                    }
                    break;
                } else {
                    let column = projected(&mut parser)?;
                    if column.name == "*" || !parser.take(Token::RParen) {
                        return Err(ParseError("grouped JOIN COUNT requires one column".into()));
                    }
                    aggregate_alias = Some(format!("count({})", column.display_name()));
                    grouped_count_column = Some(column);
                }
                grouped_count_projection = true;
                if let Some(alias) = parser.select_output_alias()? {
                    aggregate_alias = Some(alias);
                }
                break;
            }
        }
        Some(columns)
    };
    let grouped_output = if grouped_count_projection
        && grouped_output_candidate
            .iter()
            .any(|item| !matches!(item, JoinGroupedOutput::Key(_)))
    {
        grouped_output_candidate
    } else {
        Vec::new()
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
    let mut on_right_like = None;
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
        if parser.keyword("AND") {
            let column = projected(&mut parser)?;
            let negated = parser.keyword("NOT");
            if !parser.keyword("LIKE") {
                return Err(ParseError(
                    "JOIN ON supports a right-side LIKE filter".into(),
                ));
            }
            let pattern = match parser.tokens.get(parser.index) {
                Some(Token::String(value)) => value.clone(),
                _ => return Err(ParseError("JOIN ON LIKE requires a string literal".into())),
            };
            parser.index += 1;
            on_right_like = Some((column, pattern, negated));
        }
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
            if !column.relation.is_empty()
                && column.relation != left_alias
                && column.relation != right_alias
            {
                return Err(ParseError(format!(
                    "unknown JOIN qualifier: {}",
                    column.relation
                )));
            }
        }
        if !first.relation.is_empty() && first.relation == second.relation {
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
        if steps.len() >= 14 {
            return Err(ParseError("JOIN supports at most 16 tables".into()));
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
                    if !column.relation.is_empty()
                        && column.relation != left_alias
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
                if !first.relation.is_empty()
                    && !second.relation.is_empty()
                    && first_is_new == second_is_new
                {
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
    let mut where_null = None;
    let mut where_comparison = None;
    let mut where_comparisons_extra = Vec::new();
    let mut where_column_comparisons = Vec::new();
    let mut where_substring_comparisons = Vec::new();
    let mut where_in_filters = Vec::new();
    let mut where_like_filters = Vec::new();
    let mut scalar_where = None;
    let mut in_subquery = None;
    if parser.keyword("WHERE") {
        let where_tokens = parser.tokens[parser.index..].iter().take_while(|token| !matches!(token, Token::Word(word) if ["GROUP", "ORDER", "LIMIT", "OFFSET"].iter().any(|name| word.eq_ignore_ascii_case(name)))).collect::<Vec<_>>();
        let boolean_filter = where_tokens
            .iter()
            .any(|token| matches!(token, Token::Word(word) if ["LIKE", "ILIKE", "OR", "BETWEEN", "IN", "CASE"].iter().any(|keyword| word.eq_ignore_ascii_case(keyword))))
            && !where_tokens.iter().any(|token| matches!(token, Token::Word(word) if word.eq_ignore_ascii_case("SELECT")));
        let scalar_expression_filter = where_tokens
            .iter()
            .any(|token| matches!(token, Token::Word(word) if word.eq_ignore_ascii_case("CASE")))
            || {
                let mut depth = 0usize;
                where_tokens.iter().any(|token| match token {
                    Token::LParen => {
                        depth += 1;
                        false
                    }
                    Token::RParen => {
                        depth = depth.saturating_sub(1);
                        false
                    }
                    Token::Plus | Token::Minus | Token::Star | Token::Slash | Token::Percent => {
                        depth == 0
                    }
                    _ => false,
                })
            };
        let complex = scalar_expression_filter
            || grouped_distinct.is_some()
            || boolean_filter
            || where_tokens.iter().any(|token| {
                matches!(token, Token::Word(word) if word.eq_ignore_ascii_case("BETWEEN"))
            })
            || (scalar_sum.is_some()
                || scalar_formula.is_some()
                || !grouped_avg_metrics.is_empty())
                && where_tokens.iter().any(|token| matches!(token, Token::Word(word) if ["OR", "IN", "BETWEEN", "LIKE", "ILIKE", "CASE"].iter().any(|name| word.eq_ignore_ascii_case(name))));
        if grouped_distinct.is_some() {
            if let Some((subquery, branches)) = parse_distinct_not_in(&mut parser)? {
                in_subquery = Some(subquery);
                scalar_where = Some(branches);
            } else {
                scalar_where = Some(crate::where_parser::parse_join_where(&mut parser)?);
            }
        } else if complex {
            scalar_where = Some(crate::where_parser::parse_join_where(&mut parser)?);
        } else {
            let mut count = 0;
            loop {
                count += 1;
                if count > 16 {
                    return Err(ParseError("JOIN supports at most 16 comparisons".into()));
                }
                if matches!(parser.tokens.get(parser.index), Some(Token::Word(name))
                    if name.eq_ignore_ascii_case("SUBSTR")
                        || name.eq_ignore_ascii_case("SUBSTRING"))
                {
                    let (left, start, length) = parse_join_substring(&mut parser)?;
                    let operator = parser.operator()?;
                    if !matches!(
                        operator,
                        crate::Operator::Eq
                            | crate::Operator::Ne
                            | crate::Operator::Lt
                            | crate::Operator::Le
                            | crate::Operator::Gt
                            | crate::Operator::Ge
                    ) {
                        return Err(ParseError(
                            "SUBSTRING requires a comparison operator".into(),
                        ));
                    }
                    let (right, right_start, right_length) = parse_join_substring(&mut parser)?;
                    where_substring_comparisons.push((
                        left,
                        start,
                        length,
                        operator,
                        right,
                        right_start,
                        right_length,
                    ));
                } else {
                    let column = projected(&mut parser)?;
                    if column.name == "*" {
                        return Err(ParseError("JOIN WHERE requires a column".into()));
                    }
                    if parser.keyword("IS") {
                        if count != 1 {
                            return Err(ParseError(
                                "JOIN cannot combine NULL filters with other predicates".into(),
                            ));
                        }
                        let not_null = parser.keyword("NOT");
                        if !parser.keyword("NULL") && !parser.keyword("UNKNOWN") {
                            return Err(ParseError(
                                "JOIN WHERE requires IS [NOT] NULL or UNKNOWN".into(),
                            ));
                        }
                        where_null = Some((column, not_null));
                    } else if parser.keyword("IN") {
                        if !parser.take(Token::LParen) {
                            return Err(ParseError("JOIN IN requires '('".into()));
                        }
                        if matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT"))
                        {
                            if in_subquery.is_some() || !column.relation.is_empty() {
                                return Err(ParseError(
                                    "JOIN supports one IN subquery with an unqualified key".into(),
                                ));
                            }
                            let start = parser.index;
                            let mut depth = 1usize;
                            while parser.index < parser.tokens.len() && depth > 0 {
                                match parser.tokens[parser.index] {
                                    Token::LParen => depth += 1,
                                    Token::RParen => depth -= 1,
                                    _ => {}
                                }
                                parser.index += 1;
                            }
                            if depth != 0 {
                                return Err(ParseError("JOIN IN subquery requires ')'".into()));
                            }
                            let inner = crate::parse_select_tokens(
                                parser.tokens[start..parser.index - 1].to_vec(),
                            )?;
                            let supported_projection = inner.hidden_aggregate
                                && matches!(
                                    &inner.projection,
                                    crate::Projection::GroupedAggregate {
                                        function: crate::AggregateFunction::Sum,
                                        ..
                                    }
                                )
                                || !inner.hidden_aggregate
                                    && matches!(
                                        &inner.projection,
                                        crate::Projection::Columns(columns) if columns.len() == 1
                                    );
                            if !supported_projection
                                || !matches!(inner.source, Some(crate::Source::Table(_)))
                                || inner
                                    .predicates
                                    .iter()
                                    .chain(inner.or_predicates.iter().flatten())
                                    .any(|predicate| {
                                        matches!(
                                            predicate,
                                            crate::Predicate::ColumnComparison { .. }
                                        )
                                    })
                            {
                                return Err(ParseError(
                                    "JOIN IN requires an uncorrelated grouped SUM subquery".into(),
                                ));
                            }
                            in_subquery = Some(JoinInSubquery {
                                column,
                                inner: Box::new(inner),
                                negated: false,
                            });
                        } else {
                            let mut values = Vec::new();
                            loop {
                                if values.len() == 64 {
                                    return Err(ParseError(
                                        "JOIN IN supports at most 64 values".into(),
                                    ));
                                }
                                values.push(parser.literal()?);
                                if !parser.take(Token::Comma) {
                                    break;
                                }
                            }
                            if !parser.take(Token::RParen) {
                                return Err(ParseError("JOIN IN requires ')'".into()));
                            }
                            where_in_filters.push((column, values));
                        }
                    } else if matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if ["LIKE", "ILIKE", "NOT"].iter().any(|kw| word.eq_ignore_ascii_case(kw)))
                    {
                        let negated = parser.keyword("NOT");
                        let case_insensitive = parser.like_keyword().ok_or_else(|| {
                            ParseError("JOIN WHERE expects LIKE or ILIKE after NOT".into())
                        })?;
                        let pattern = parser.literal()?;
                        where_like_filters.push((column, pattern, case_insensitive, negated));
                    } else {
                        let operator = parser.operator()?;
                        if !matches!(
                            operator,
                            crate::Operator::Eq
                                | crate::Operator::Ne
                                | crate::Operator::Lt
                                | crate::Operator::Le
                                | crate::Operator::Gt
                                | crate::Operator::Ge
                        ) {
                            return Err(ParseError(
                                "JOIN WHERE requires a comparison operator".into(),
                            ));
                        }
                        let is_column = match parser.tokens.get(parser.index) {
                            Some(Token::QuotedWord(_)) => true,
                            Some(Token::Word(word)) => ![
                                "CAST",
                                "DATE",
                                "TIMESTAMP",
                                "NULL",
                                "TRUE",
                                "FALSE",
                                "INTERVAL",
                            ]
                            .iter()
                            .any(|kw| word.eq_ignore_ascii_case(kw)),
                            _ => false,
                        };
                        if is_column {
                            let other = projected(&mut parser)?;
                            if other.name == "*" {
                                return Err(ParseError("JOIN WHERE requires a column".into()));
                            }
                            where_column_comparisons.push((column, operator, other));
                        } else {
                            let comparison = (column, operator, parser.literal()?);
                            if where_comparison.is_none() {
                                where_comparison = Some(comparison);
                            } else {
                                where_comparisons_extra.push(comparison);
                            }
                        }
                    }
                }
                if !parser.keyword("AND") {
                    break;
                }
                if where_null.is_some() {
                    return Err(ParseError(
                        "JOIN cannot combine NULL filters with other predicates".into(),
                    ));
                }
            }
        }
    }
    let group_by = if parser.keyword("GROUP") {
        if !parser.keyword("BY") {
            return Err(ParseError("JOIN GROUP requires BY".into()));
        }
        grouped_rollup = parser.keyword("ROLLUP");
        if grouped_rollup && !parser.take(Token::LParen) {
            return Err(ParseError("ROLLUP requires '('".into()));
        }
        let mut keys = vec![projected(&mut parser)?];
        while parser.take(Token::Comma) {
            if keys.len() >= 64 {
                return Err(ParseError("grouped JOIN supports at most 64 keys".into()));
            }
            keys.push(projected(&mut parser)?);
        }
        if grouped_rollup && !parser.take(Token::RParen) {
            return Err(ParseError("ROLLUP requires ')'".into()));
        }
        for key in &mut keys {
            if key.relation.is_empty() {
                if let Some((position, _)) = grouped_key_aliases
                    .iter()
                    .find(|(_, alias)| key.name.eq_ignore_ascii_case(alias))
                {
                    *key = columns.as_ref().expect("alias projection")[*position].clone();
                }
            }
        }
        if let Some((position, alias)) = &grouped_year {
            let source = &columns.as_ref().expect("year projection")[*position];
            for key in &mut keys {
                if key.relation.is_empty() && key.name.eq_ignore_ascii_case(alias) {
                    *key = source.clone();
                } else if key == source {
                    return Err(ParseError(
                        "GROUP BY must use the EXTRACT YEAR alias".into(),
                    ));
                }
            }
        }
        if keys.iter().any(|key| key.name == "*") {
            return Err(ParseError("JOIN GROUP BY requires columns".into()));
        }
        Some(keys)
    } else {
        None
    };
    let grouped_keys_only = !grouped_count_projection && group_by.is_some();
    if grouped_keys_only {
        if kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || aggregate.is_some()
            || columns.as_ref().is_none_or(Vec::is_empty)
            || grouped_year.is_some()
        {
            return Err(ParseError(
                "GROUP BY without aggregates requires projected keys over INNER joins".into(),
            ));
        }
        grouped_count_projection = true;
    }
    let mut grouped_count_having = None;
    let grouped_sum_having = if parser.keyword("HAVING") {
        if grouped_sum.is_none() && grouped_count_projection {
            if parser.keyword("COUNT") {
                if !parser.take(Token::LParen)
                    || !parser.take(Token::Star)
                    || !parser.take(Token::RParen)
                {
                    return Err(ParseError("HAVING COUNT requires count(*)".into()));
                }
            } else {
                let alias = parser.word()?;
                if !aggregate_alias
                    .as_ref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(&alias))
                {
                    return Err(ParseError("JOIN HAVING must use the COUNT alias".into()));
                }
            }
            let operator = parser.operator()?;
            let value = parser.number()?;
            let valid = if value.contains(['.', 'e', 'E']) {
                value.parse::<f64>().is_ok_and(f64::is_finite)
            } else if value.starts_with('-') {
                value.parse::<i128>().is_ok()
            } else {
                value.parse::<u128>().is_ok()
            };
            if !valid {
                return Err(ParseError("invalid JOIN HAVING number".into()));
            }
            grouped_count_having = Some(GroupedCountHaving { operator, value });
            None
        } else {
            let metric = grouped_sum
                .as_ref()
                .ok_or_else(|| ParseError("JOIN HAVING requires a grouped SUM".into()))?;
            if parser.keyword("SUM") {
                if !parser.take(Token::LParen) {
                    return Err(ParseError("HAVING SUM requires '('".into()));
                }
                let start = parser.index;
                let column = projected(&mut parser).ok().filter(|column| {
                    column.name != "*" && parser.tokens.get(parser.index) == Some(&Token::RParen)
                });
                let having_metric = if let Some(column) = column {
                    GroupedSum::Column(column)
                } else {
                    parser.index = start;
                    GroupedSum::Expression(crate::expression::parse_expression_tokens(&mut parser)?)
                };
                if !parser.take(Token::RParen) || &having_metric != metric {
                    return Err(ParseError("HAVING SUM must match the selected SUM".into()));
                }
            } else {
                let name = parser.word()?;
                if !aggregate_alias
                    .as_ref()
                    .is_some_and(|alias| alias.eq_ignore_ascii_case(&name))
                {
                    return Err(ParseError("JOIN HAVING must use the SUM alias".into()));
                }
                if columns.as_ref().is_some_and(|columns| {
                    columns
                        .iter()
                        .any(|column| column.name.eq_ignore_ascii_case(&name))
                }) {
                    return Err(ParseError(
                        "JOIN HAVING SUM alias conflicts with a group key".into(),
                    ));
                }
            }
            let operator = parser.operator()?;
            let value = parser.number()?;
            let valid = if value.contains(['.', 'e', 'E']) {
                value.parse::<f64>().is_ok_and(f64::is_finite)
            } else if value.starts_with('-') {
                value.parse::<i128>().is_ok()
            } else {
                value.parse::<u128>().is_ok()
            };
            if !valid {
                return Err(ParseError("invalid JOIN HAVING number".into()));
            }
            Some(GroupedSumHaving {
                operator,
                value,
                subquery: None,
            })
        }
    } else {
        None
    };
    let order_by = if parser.keyword("ORDER") {
        if !parser.keyword("BY") {
            return Err(ParseError("ORDER requires BY".into()));
        }
        let mut terms = Vec::new();
        loop {
            if terms.len() == 64 {
                return Err(ParseError("JOIN ORDER BY supports at most 64 keys".into()));
            }
            let next_is_count_alias = grouped_count_projection
                && grouped_case_counts.is_empty()
                && matches!(parser.tokens.get(parser.index), Some(Token::Word(name) | Token::QuotedAlias(name))
                    if name.eq_ignore_ascii_case(aggregate_alias.as_deref().unwrap_or("count")))
                && !matches!(
                    parser.tokens.get(parser.index + 1),
                    Some(Token::Dot | Token::LParen)
                );
            let next_case_alias = (!steps.is_empty())
                .then(|| {
                    let Some(Token::Word(name) | Token::QuotedAlias(name)) =
                        parser.tokens.get(parser.index)
                    else {
                        return None;
                    };
                    grouped_case_counts
                        .iter()
                        .position(|(_, _, alias)| alias.eq_ignore_ascii_case(name))
                })
                .flatten();
            let next_avg_alias = grouped_avg_metrics
                .iter()
                .position(|(_, alias)| {
                    matches!(parser.tokens.get(parser.index), Some(Token::Word(name) | Token::QuotedAlias(name))
                        if name.eq_ignore_ascii_case(alias))
                });
            let next_is_window_ratio_alias = grouped_window_ratio
                .as_ref()
                .is_some_and(|ratio| {
                    matches!(parser.tokens.get(parser.index), Some(Token::Word(name) | Token::QuotedAlias(name))
                        if name.eq_ignore_ascii_case(&ratio.alias))
                });
            let next_is_grouped_sum = grouped_sum.is_some()
                && matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("SUM"))
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen);
            let next_is_scalar_sum = (scalar_sum.is_some()
                || matches!(
                    aggregate.as_ref(),
                    Some(JoinAggregate::Numeric {
                        function: crate::AggregateFunction::Sum,
                        ..
                    })
                ))
                && matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("SUM"))
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen);
            let next_scalar_expression_order = scalar_projection_ahead(&parser).then(|| {
                let (encoded, original_ends) =
                    encode_qualified_expression_tokens(&parser.tokens[parser.index..]);
                let mut expression_parser = Parser {
                    tokens: encoded,
                    index: 0,
                };
                let expression =
                    crate::expression::parse_expression_tokens(&mut expression_parser).ok()?;
                if expression_parser.index == 0 {
                    return None;
                }
                let scalar_index = scalar_projections
                    .iter()
                    .position(|projection| projection.expression == expression)?;
                let consumed = original_ends.get(expression_parser.index - 1).copied()?;
                Some((scalar_index, consumed))
            });
            let next_scalar_expression_order = next_scalar_expression_order.flatten();
            let next_scalar_alias_order = scalar_projections.iter().position(|projection| {
                matches!(parser.tokens.get(parser.index), Some(Token::Word(name) | Token::QuotedAlias(name))
                    if name.eq_ignore_ascii_case(&projection.alias))
                    && !matches!(parser.tokens.get(parser.index + 1), Some(Token::Dot | Token::LParen))
            });
            let scalar_count_distinct_order =
                !scalar_aggregates.is_empty() && parser.keyword("COUNT");
            let key = if scalar_count_distinct_order {
                if !parser.take(Token::LParen) {
                    return Err(ParseError("scalar JOIN ORDER BY COUNT requires '('".into()));
                }
                let distinct = parser.keyword("DISTINCT");
                let column = if parser.take(Token::Star) {
                    None
                } else {
                    Some(projected(&mut parser)?)
                };
                if !parser.take(Token::RParen)
                    || !scalar_aggregates.iter().any(|item| {
                        match (&item.aggregate, item.distinct, &column, distinct) {
                            (JoinAggregate::CountStar, false, None, false) => true,
                            (JoinAggregate::CountColumn(expected), false, Some(actual), false)
                            | (JoinAggregate::CountColumn(expected), true, Some(actual), true) => {
                                expected == actual
                            }
                            _ => false,
                        }
                    })
                {
                    return Err(ParseError(
                        "scalar JOIN ORDER BY COUNT must match a selected metric".into(),
                    ));
                }
                JoinColumn::count_order()
            } else if let Some(index) = next_case_alias {
                parser.index += 1;
                JoinColumn::case_order(index)
            } else if let Some(index) = next_avg_alias {
                parser.index += 1;
                JoinColumn::avg_order(index)
            } else if grouped_rollup && parser.keyword("GROUPING") {
                if !parser.take(Token::LParen) {
                    return Err(ParseError("GROUPING requires '('".into()));
                }
                let key = projected(&mut parser)?;
                if !parser.take(Token::RParen) {
                    return Err(ParseError("GROUPING requires ')'".into()));
                }
                let index = columns
                    .as_ref()
                    .and_then(|keys| keys.iter().position(|candidate| candidate == &key))
                    .ok_or_else(|| ParseError("GROUPING requires a projected rollup key".into()))?;
                JoinColumn::grouping_order(index)
            } else if next_is_window_ratio_alias {
                parser.index += 1;
                JoinColumn::window_ratio_order()
            } else if let Some(index) = next_scalar_alias_order {
                parser.index += 1;
                JoinColumn::scalar_order(index)
            } else if next_is_count_alias {
                parser.index += 1;
                JoinColumn::count_order()
            } else if next_is_grouped_sum {
                parser.keyword("SUM");
                if !parser.take(Token::LParen) {
                    return Err(ParseError("ORDER BY SUM requires '('".into()));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("ORDER BY SUM requires one column".into()));
                }
                if grouped_sum.as_ref() != Some(&GroupedSum::Column(column)) {
                    return Err(ParseError(
                        "ORDER BY SUM must match the selected grouped SUM".into(),
                    ));
                }
                JoinColumn::count_order()
            } else if next_is_scalar_sum {
                parser.keyword("SUM");
                if !parser.take(Token::LParen) {
                    return Err(ParseError("ORDER BY SUM requires '('".into()));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("ORDER BY SUM requires one column".into()));
                }
                let selected_column = match (&scalar_sum, &aggregate) {
                    (Some(crate::ScalarExpr::Column(name)), _) => {
                        Some((name.as_str(), scalar_sum_qualifier.as_deref()))
                    }
                    (
                        _,
                        Some(JoinAggregate::Numeric {
                            function: crate::AggregateFunction::Sum,
                            column,
                        }),
                    ) => Some((column.name.as_str(), Some(column.relation.as_str()))),
                    _ => None,
                };
                if selected_column.is_none_or(|(name, qualifier)| {
                    name != column.name
                        || qualifier.is_some_and(|qualifier| qualifier != column.relation)
                }) {
                    return Err(ParseError(
                        "ORDER BY SUM must match the selected scalar SUM".into(),
                    ));
                }
                JoinColumn::count_order()
            } else if grouped_count_projection
                && grouped_case_counts.is_empty()
                && grouped_sum.is_none()
                && grouped_min.is_none()
                && grouped_formula.is_none()
                && grouped_distinct.is_none()
                && grouped_count_column.is_none()
                && parser.keyword("COUNT")
            {
                if !parser.take(Token::LParen)
                    || !parser.take(Token::Star)
                    || !parser.take(Token::RParen)
                {
                    return Err(ParseError(
                        "grouped JOIN ORDER BY COUNT requires COUNT(*)".into(),
                    ));
                }
                JoinColumn::count_order()
            } else if aggregate == Some(JoinAggregate::CountStar) && parser.keyword("COUNT") {
                if !parser.take(Token::LParen)
                    || !parser.take(Token::Star)
                    || !parser.take(Token::RParen)
                {
                    return Err(ParseError(
                        "scalar JOIN ORDER BY COUNT requires COUNT(*)".into(),
                    ));
                }
                // A scalar COUNT returns one row, so this cannot change its order.
                JoinColumn::count_order()
            } else if grouped_count_projection
                && matches!(parser.tokens.get(parser.index), Some(Token::Number(_)))
            {
                let position = parser
                    .number()?
                    .parse::<usize>()
                    .map_err(|_| ParseError("JOIN ORDER BY position is too large".into()))?;
                if position == 0 {
                    return Err(ParseError("JOIN ORDER BY positions start at 1".into()));
                }
                columns
                    .as_ref()
                    .and_then(|columns| columns.get(position - 1))
                    .cloned()
                    .ok_or_else(|| ParseError("JOIN ORDER BY position is out of range".into()))?
            } else if aggregate.is_none()
                && !grouped_count_projection
                && grouped_sum.is_none()
                && grouped_case_counts.is_empty()
                && !grouped_rollup
                && matches!(parser.tokens.get(parser.index), Some(Token::Number(_)))
            {
                let position = parser
                    .number()?
                    .parse::<usize>()
                    .map_err(|_| ParseError("JOIN ORDER BY position is too large".into()))?;
                if position == 0 {
                    return Err(ParseError("JOIN ORDER BY positions start at 1".into()));
                }
                match projection_order.get(position - 1) {
                    Some(JoinProjectionOrder::Column(index)) => columns
                        .as_ref()
                        .and_then(|columns| columns.get(*index))
                        .cloned()
                        .ok_or_else(|| {
                            ParseError("JOIN ORDER BY position is out of range".into())
                        })?,
                    Some(JoinProjectionOrder::Scalar(index)) => JoinColumn::scalar_order(*index),
                    None => {
                        return Err(ParseError("JOIN ORDER BY position is out of range".into()));
                    }
                }
            } else if let Some((index, consumed)) = next_scalar_expression_order {
                parser.index += consumed;
                JoinColumn::scalar_order(index)
            } else {
                let mut key = projected(&mut parser)?;
                if key.name == "*" {
                    return Err(ParseError("JOIN ORDER BY requires a column".into()));
                }
                if key.relation.is_empty() {
                    if let Some((position, _)) = grouped_key_aliases
                        .iter()
                        .find(|(_, alias)| key.name.eq_ignore_ascii_case(alias))
                    {
                        key = columns.as_ref().expect("alias projection")[*position].clone();
                    }
                }
                if let Some((position, alias)) = &grouped_year {
                    let source = &columns.as_ref().expect("year projection")[*position];
                    if key.relation.is_empty() && key.name.eq_ignore_ascii_case(alias) {
                        key = source.clone();
                    } else if &key == source {
                        return Err(ParseError(
                            "ORDER BY must use the EXTRACT YEAR alias".into(),
                        ));
                    }
                }
                key
            };
            let descending = if parser.keyword("USING") {
                match parser.operator()? {
                    crate::Operator::Lt => false,
                    crate::Operator::Gt => true,
                    _ => return Err(ParseError("JOIN ORDER BY USING expects '<' or '>'".into())),
                }
            } else if parser.keyword("DESC") {
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
    } else {
        Vec::new()
    };
    let known_qualifier = |relation: &str| {
        relation == left_alias
            || relation == right_alias
            || steps.iter().any(|step| step.alias == relation)
    };
    if let Some(column) = &grouped_count_column {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    if let Some((column, _, _)) = &on_right_like {
        if !column.relation.is_empty() && column.relation != right_alias {
            return Err(ParseError("JOIN ON LIKE must use the right input".into()));
        }
    }
    if let Some(JoinAggregate::CountColumn(column) | JoinAggregate::Numeric { column, .. }) =
        &aggregate
    {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    for item in &scalar_aggregates {
        if let JoinAggregate::CountColumn(column) | JoinAggregate::Numeric { column, .. } =
            &item.aggregate
        {
            if !column.relation.is_empty() && !known_qualifier(&column.relation) {
                return Err(ParseError(format!(
                    "unknown JOIN qualifier: {}",
                    column.relation
                )));
            }
        }
    }
    if let Some(columns) = &columns {
        for column in columns {
            if !column.relation.is_empty() && !known_qualifier(&column.relation) {
                return Err(ParseError(format!(
                    "unknown JOIN qualifier: {}",
                    column.relation
                )));
            }
        }
    }
    if let Some((column, _, _)) = &where_comparison {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    for (column, _, _) in &where_comparisons_extra {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    if let Some(column) = &grouped_distinct {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    if let Some(column) = &grouped_min {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    for (column, _) in grouped_min_metrics.iter().chain(&grouped_max_metrics) {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    if let Some(GroupedSum::Column(column)) = &grouped_sum {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    for (left, _, right) in &where_column_comparisons {
        for column in [left, right] {
            if !column.relation.is_empty() && !known_qualifier(&column.relation) {
                return Err(ParseError(format!(
                    "unknown JOIN qualifier: {}",
                    column.relation
                )));
            }
        }
    }
    for (column, _) in &where_in_filters {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    for (column, _, _, _) in &where_like_filters {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    if let Some((column, _)) = &where_null {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    if let Some(columns) = &group_by {
        for column in columns {
            if !column.relation.is_empty() && !known_qualifier(&column.relation) {
                return Err(ParseError(format!(
                    "unknown JOIN qualifier: {}",
                    column.relation
                )));
            }
        }
    }
    for (column, _, _) in &order_by {
        if !column.is_count_order()
            && !column.relation.is_empty()
            && !known_qualifier(&column.relation)
        {
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
    if !where_substring_comparisons.is_empty()
        && (steps.is_empty()
            || kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || aggregate.is_some()
            || !scalar_aggregates.is_empty()
            || scalar_sum.is_some()
            || scalar_formula.is_some()
            || grouped_min.is_some()
            || !grouped_max_metrics.is_empty()
            || grouped_formula.is_some()
            || grouped_distinct.is_some()
            || !grouped_case_counts.is_empty()
            || (!order_by.is_empty() && !grouped_count_projection && grouped_sum.is_none()))
    {
        return Err(ParseError(
            "SUBSTRING comparisons require a streaming INNER join chain".into(),
        ));
    }
    // A chained scalar SUM uses the existing bounded streaming accumulator.
    let mut chained_scalar_sum = false;
    if !steps.is_empty()
        && kind == JoinType::Inner
        && steps.iter().all(|step| step.kind == JoinType::Inner)
        && scalar_sum.is_none()
        && scalar_formula.is_none()
    {
        if let Some(JoinAggregate::Numeric {
            function: crate::AggregateFunction::Sum,
            column,
        }) = &aggregate
        {
            scalar_sum = Some(crate::ScalarExpr::Column(column.name.clone()));
            scalar_sum_qualifier = (!column.relation.is_empty()).then(|| column.relation.clone());
            aggregate = None;
            chained_scalar_sum = true;
        }
    }
    if (scalar_sum.is_some() || scalar_formula.is_some())
        && (kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || (!order_by.is_empty()
                && !(scalar_sum.is_some()
                    && order_by
                        .iter()
                        .all(|(column, _, _)| column.is_count_order())))
            || (!chained_scalar_sum
                && (!where_in_filters.is_empty() || !where_like_filters.is_empty())))
    {
        return Err(ParseError(
            "JOIN expression SUM requires INNER joins and comparison filters".into(),
        ));
    }
    if !scalar_aggregates.is_empty()
        && (kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || group_by.is_some()
            || order_by
                .iter()
                .any(|(column, _, _)| !column.is_count_order())
            || offset > 0)
    {
        return Err(ParseError(
            "multiple scalar JOIN aggregates require an unordered INNER join chain".into(),
        ));
    }
    let chained_comparison_filters = (!steps.is_empty()
        || grouped_sum.is_some()
        || grouped_min.is_some()
        || !grouped_max_metrics.is_empty()
        || grouped_formula.is_some())
        && kind == JoinType::Inner
        && steps.iter().all(|step| step.kind == JoinType::Inner)
        && matches!(
            aggregate.as_ref(),
            None | Some(JoinAggregate::CountStar | JoinAggregate::CountColumn(_))
        )
        && (scalar_sum.is_some() || (where_in_filters.is_empty() && where_like_filters.is_empty()));
    let derived_ordered_comparison_filters = allow_ordered_comparison_filters
        && kind == JoinType::Inner
        && steps.iter().all(|step| step.kind == JoinType::Inner)
        && !grouped_count_projection
        && aggregate.is_none()
        && group_by.is_none()
        && scalar_where.is_none()
        && where_comparison.is_some()
        && where_column_comparisons.is_empty()
        && where_in_filters.is_empty()
        && where_like_filters.is_empty();
    if grouped_case_counts.is_empty()
        && (where_comparison.is_some()
            || !where_column_comparisons.is_empty()
            || !where_in_filters.is_empty()
            || !where_like_filters.is_empty())
        && !chained_comparison_filters
        && !derived_ordered_comparison_filters
        && (kind != JoinType::Inner
            || !steps.is_empty()
            || (grouped_count_projection
                && (columns.as_ref().is_none_or(|keys| keys.len() != 1)
                    || where_comparison.as_ref().is_some_and(|(column, _, _)| {
                        !column.relation.is_empty() && column.relation != left_alias
                    })))
            || (!grouped_count_projection
                && (!order_by.is_empty()
                    || !matches!(aggregate.as_ref(), None | Some(JoinAggregate::CountStar))
                    || group_by.is_some())))
    {
        return Err(ParseError(
            "JOIN comparison WHERE requires an unordered two-table INNER JOIN, or grouped COUNT(*) with a fact-side filter"
                .into(),
        ));
    }
    if grouped_count_projection
        && grouped_case_counts.is_empty()
        && !chained_comparison_filters
        && (!where_comparisons_extra.is_empty()
            || !where_column_comparisons.is_empty()
            || !where_in_filters.is_empty()
            || !where_like_filters.is_empty())
    {
        return Err(ParseError(
            "grouped JOIN supports one comparison filter".into(),
        ));
    }
    if grouped_case_counts
        .iter()
        .any(|(_, value, _)| !matches!(value, GroupedCaseValue::Count))
        && steps.is_empty()
        && kind != JoinType::Inner
    {
        return Err(ParseError(
            "conditional numeric SUM requires an INNER join chain".into(),
        ));
    }
    let global_case_aggregate = grouped_count_projection
        && !grouped_case_counts.is_empty()
        && columns.as_ref().is_some_and(Vec::is_empty)
        && group_by.is_none();
    if !grouped_case_counts.is_empty()
        && !global_case_aggregate
        && (!matches!(kind, JoinType::Inner | JoinType::Left)
            || steps
                .iter()
                .any(|step| !matches!(step.kind, JoinType::Inner | JoinType::Left))
            || where_null.is_some()
            || columns.as_ref().is_none_or(Vec::is_empty))
    {
        return Err(ParseError(
            "JOIN grouped CASE requires group keys over INNER or LEFT inputs".into(),
        ));
    }
    if !grouped_avg_metrics.is_empty()
        && ((!allow_two_table_grouped_avg && steps.is_empty())
            || kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || grouped_sum.is_some()
            || grouped_formula.is_some()
            || grouped_distinct.is_some()
            || grouped_count_column.is_some()
            || !grouped_case_counts.is_empty()
            || order_by
                .iter()
                .any(|(column, _, _)| column.is_count_order()))
    {
        return Err(ParseError(
            "multiple grouped AVGs require an INNER join chain with key ordering".into(),
        ));
    }
    if grouped_distinct.is_some()
        && (kind != JoinType::Inner || steps.iter().any(|step| step.kind != JoinType::Inner))
    {
        return Err(ParseError(
            "grouped COUNT DISTINCT requires INNER joins".into(),
        ));
    }
    if grouped_count_column.is_some()
        && (!steps.is_empty()
            || !matches!(kind, JoinType::Inner | JoinType::Left)
            || columns.as_ref().is_none_or(|columns| columns.len() != 1))
    {
        return Err(ParseError(
            "grouped COUNT(column) requires one key and two INNER or LEFT inputs".into(),
        ));
    }
    if on_right_like.is_some()
        && (kind != JoinType::Left || grouped_count_column.is_none() || !steps.is_empty())
    {
        return Err(ParseError(
            "JOIN ON LIKE requires grouped LEFT JOIN COUNT(column)".into(),
        ));
    }
    let grouped_sum_outer_chain = grouped_sum.is_some()
        && grouped_count_projection
        && columns.as_ref().is_some_and(|keys| !keys.is_empty())
        && group_by.is_some()
        && matches!(kind, JoinType::Inner | JoinType::Left)
        && steps
            .iter()
            .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left));
    let grouped_case_inner_chain = !grouped_case_counts.is_empty()
        && grouped_count_projection
        && columns.as_ref().is_some_and(|keys| !keys.is_empty())
        && group_by.is_some()
        && kind == JoinType::Inner
        && steps.iter().all(|step| step.kind == JoinType::Inner);
    if (grouped_min.is_some()
        || !grouped_max_metrics.is_empty()
        || grouped_formula.is_some()
        || (grouped_sum.is_some() && !grouped_sum_outer_chain))
        && (kind != JoinType::Inner || steps.iter().any(|step| step.kind != JoinType::Inner))
    {
        return Err(ParseError(
            "grouped SUM requires a keyed INNER or LEFT join chain; grouped MIN/MAX requires INNER joins"
                .into(),
        ));
    }
    if !grouped_key_aliases.is_empty() {
        let simple_projection = !grouped_count_projection
            && aggregate.is_none()
            && scalar_aggregates.is_empty()
            && scalar_sum.is_none()
            && scalar_formula.is_none()
            && grouped_sum.is_none()
            && grouped_min.is_none()
            && !grouped_keys_only
            && group_by.is_none();
        let simple_projection_outer_chain = simple_projection
            && matches!(kind, JoinType::Inner | JoinType::Left)
            && steps
                .iter()
                .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left));
        if !(grouped_count_projection || simple_projection)
            || (!(grouped_sum_outer_chain || simple_projection_outer_chain)
                && (kind != JoinType::Inner
                    || steps.iter().any(|step| step.kind != JoinType::Inner)))
            || grouped_count_column.is_some()
            || (!grouped_case_counts.is_empty() && !grouped_case_inner_chain)
        {
            return Err(ParseError(
                "JOIN key aliases require a simple or grouped INNER join".into(),
            ));
        }
        for (index, (_, alias)) in grouped_key_aliases.iter().enumerate() {
            if grouped_key_aliases[..index]
                .iter()
                .any(|(_, other)| other.eq_ignore_ascii_case(alias))
                || grouped_year
                    .as_ref()
                    .is_some_and(|(_, other)| other.eq_ignore_ascii_case(alias))
                || (grouped_count_projection
                    && aggregate_alias
                        .as_deref()
                        .unwrap_or("count")
                        .eq_ignore_ascii_case(alias))
            {
                return Err(ParseError("ambiguous grouped JOIN output alias".into()));
            }
        }
    }
    if let Some((position, alias)) = &grouped_year {
        let keys = columns.as_ref().expect("year projection");
        if !grouped_count_projection
            || kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || keys.iter().enumerate().any(|(index, key)| {
                index != *position
                    && (key == &keys[*position] || key.name.eq_ignore_ascii_case(alias))
            })
            || aggregate_alias
                .as_deref()
                .unwrap_or("count")
                .eq_ignore_ascii_case(alias)
            || !grouped_case_counts.is_empty()
            || grouped_count_column.is_some()
        {
            return Err(ParseError(
                "EXTRACT YEAR requires an unambiguous grouped INNER join".into(),
            ));
        }
    }
    if let Some(ratio) = &grouped_window_ratio {
        let valid_multiplier = ratio.multiplier.parse::<f64>().is_ok_and(f64::is_finite);
        let Some(GroupedSum::Column(_)) = grouped_sum.as_ref() else {
            return Err(ParseError(
                "grouped window ratio requires a selected column SUM".into(),
            ));
        };
        let Some(keys) = columns.as_ref() else {
            return Err(ParseError(
                "grouped window ratio requires projected group keys".into(),
            ));
        };
        if !valid_multiplier
            || !grouped_count_projection
            || steps.is_empty()
            || kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || !keys.contains(&ratio.partition)
            || !group_by
                .as_ref()
                .is_some_and(|groups| groups.contains(&ratio.partition))
            || aggregate_alias
                .as_ref()
                .is_some_and(|alias| alias.eq_ignore_ascii_case(&ratio.alias))
            || keys
                .iter()
                .any(|key| key.name.eq_ignore_ascii_case(&ratio.alias))
            || grouped_key_aliases
                .iter()
                .any(|(_, alias)| alias.eq_ignore_ascii_case(&ratio.alias))
            || (!ratio.partition.relation.is_empty() && !known_qualifier(&ratio.partition.relation))
            || grouped_sum_position.is_none()
            || !grouped_case_counts.is_empty()
            || !grouped_sum_extras.is_empty()
            || grouped_formula.is_some()
            || !grouped_avg_metrics.is_empty()
        {
            return Err(ParseError(
                "grouped window ratio requires a grouped INNER SUM and a projected partition key"
                    .into(),
            ));
        }
    }
    if let Some(window_average) = &grouped_window_average {
        let Some(GroupedSum::Column(sum_column)) = grouped_sum.as_ref() else {
            return Err(ParseError(
                "grouped window AVG requires a selected column SUM".into(),
            ));
        };
        let Some(groups) = group_by.as_ref() else {
            return Err(ParseError("grouped window AVG requires GROUP BY".into()));
        };
        let Some(keys) = columns.as_ref() else {
            return Err(ParseError(
                "grouped window AVG requires projected group keys".into(),
            ));
        };
        if grouped_window_ratio.is_some()
            || grouped_sum_extras.len() > 0
            || !grouped_case_counts.is_empty()
            || !grouped_min_metrics.is_empty()
            || !grouped_max_metrics.is_empty()
            || !grouped_avg_metrics.is_empty()
            || grouped_formula.is_some()
            || grouped_rollup
            || steps.is_empty()
            || kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || &window_average.value != sum_column
            || window_average.partition.is_empty()
            || window_average
                .partition
                .iter()
                .any(|key| !groups.contains(key) || !keys.contains(key))
            || window_average
                .partition
                .iter()
                .enumerate()
                .any(|(index, key)| window_average.partition[..index].contains(key))
            || keys
                .iter()
                .any(|key| key.name.eq_ignore_ascii_case(&window_average.alias))
            || grouped_key_aliases
                .iter()
                .any(|(_, alias)| alias.eq_ignore_ascii_case(&window_average.alias))
            || aggregate_alias
                .as_ref()
                .is_some_and(|alias| alias.eq_ignore_ascii_case(&window_average.alias))
        {
            return Err(ParseError(
                "grouped window AVG requires a grouped INNER SUM and projected partition key"
                    .into(),
            ));
        }
    }
    let groups_include_unprojected_keys = grouped_sum.is_some()
        && !steps.is_empty()
        && group_by.as_ref().is_some_and(|groups| {
            columns
                .as_ref()
                .is_some_and(|projected| groups.iter().any(|key| !projected.contains(key)))
        });
    let grouped_execution_keys =
        if grouped_window_average.is_some() || groups_include_unprojected_keys {
            group_by.clone().unwrap_or_default()
        } else {
            Vec::new()
        };
    let grouped_count = if grouped_count_projection {
        let projected_keys = columns
            .as_ref()
            .ok_or_else(|| ParseError("grouped JOIN requires projected keys".into()))?;
        let matching_groups = global_case_aggregate
            || if !steps.is_empty() {
                group_by.as_ref().is_some_and(|groups| {
                    groups == projected_keys
                        || (groups.len() == projected_keys.len()
                            && groups.iter().all(|key| projected_keys.contains(key))
                            && groups
                                .iter()
                                .enumerate()
                                .all(|(index, key)| !groups[..index].contains(key))
                            && projected_keys
                                .iter()
                                .enumerate()
                                .all(|(index, key)| !projected_keys[..index].contains(key)))
                })
            } else {
                group_by.as_ref() == Some(projected_keys)
            };
        let grouped_sum_can_collapse_unprojected_keys = !steps.is_empty()
            && grouped_sum.is_some()
            && group_by
                .as_ref()
                .is_some_and(|groups| projected_keys.iter().all(|key| groups.contains(key)));
        if !matching_groups && !grouped_sum_can_collapse_unprojected_keys {
            return Err(ParseError(
                "GROUP BY must match the selected JOIN keys".into(),
            ));
        }
        if grouped_rollup
            && (grouped_avg_metrics.is_empty()
                || grouped_sum.is_some()
                || !grouped_sum_extras.is_empty()
                || grouped_min.is_some()
                || !grouped_max_metrics.is_empty()
                || grouped_formula.is_some()
                || grouped_window_ratio.is_some()
                || !grouped_case_counts.is_empty()
                || grouped_sum_having.is_some()
                || grouped_count_having.is_some())
        {
            return Err(ParseError(
                "JOIN ROLLUP currently requires grouped AVG metrics without HAVING".into(),
            ));
        }
        let grouped_case_outer_join = !grouped_case_counts.is_empty()
            && ((matches!(kind, JoinType::Inner | JoinType::Left)
                && steps
                    .iter()
                    .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left)))
                || (global_case_aggregate && kind == JoinType::Full && steps.is_empty()));
        if (!matches!(kind, JoinType::Inner | JoinType::Left | JoinType::Full)
            || (kind == JoinType::Full && !grouped_case_outer_join))
            || (!(grouped_case_outer_join || grouped_sum_outer_chain)
                && ((projected_keys.len() > 1 && kind != JoinType::Inner)
                    || (!steps.is_empty()
                        && (kind != JoinType::Inner
                            || steps.iter().any(|step| step.kind != JoinType::Inner)))))
        {
            return Err(ParseError(
                "chained grouped JOIN COUNT requires INNER joins, except for grouped CASE with LEFT joins".into(),
            ));
        }
        if order_by.iter().any(|(column, _, _)| {
            (column.is_count_order() && grouped_keys_only)
                || (!column.is_count_order()
                    && !column.is_window_ratio_order()
                    && column.case_order_index().is_none()
                    && column.avg_order_index().is_none()
                    && column.grouping_order_index().is_none()
                    && !projected_keys.contains(column))
        }) {
            return Err(ParseError(
                "grouped JOIN ORDER BY must use a projected group key or aggregate".into(),
            ));
        }
        if global_case_aggregate {
            Some(Vec::new())
        } else if steps.is_empty() {
            group_by
        } else {
            Some(projected_keys.clone())
        }
    } else {
        if group_by.is_some() {
            return Err(ParseError(
                "JOIN GROUP BY requires projected COUNT(*)".into(),
            ));
        }
        None
    };
    if let Some(subquery) = &in_subquery {
        let supported = if subquery.negated {
            grouped_distinct.is_some() && scalar_where.is_some()
        } else {
            !steps.is_empty()
                && grouped_sum.is_some()
                && scalar_where.is_none()
                && grouped_count_projection
        };
        if !supported
            || kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || where_null.is_some()
        {
            return Err(ParseError(
                "JOIN subquery requires a supported grouped INNER join".into(),
            ));
        }
    }
    // COUNT(column) can use bounded materialization for an all-INNER chain.
    if matches!(aggregate.as_ref(), Some(JoinAggregate::CountColumn(_)))
        && (kind != JoinType::Inner || steps.iter().any(|step| step.kind != JoinType::Inner))
    {
        return Err(ParseError("JOIN COUNT(column) requires INNER joins".into()));
    }
    // Chained numeric/column aggregates and OUTER or filtered chained COUNT
    // lack a matching streaming semantics implementation. Reject them.
    // COUNT(*) can also use the bounded materializing executor for OUTER
    // chains. Only the all-INNER case uses the multiplicity-only fast path.
    if !steps.is_empty()
        && aggregate.is_some()
        && !matches!(
            aggregate.as_ref(),
            Some(JoinAggregate::CountStar | JoinAggregate::CountColumn(_))
        )
    {
        return Err(ParseError(
            "chained JOIN supports COUNT as its only scalar aggregate".into(),
        ));
    }
    let scalar_count_order = aggregate == Some(JoinAggregate::CountStar)
        && order_by
            .iter()
            .all(|(column, _, _)| column.is_count_order());
    if aggregate.is_some()
        && ((!order_by.is_empty() && !scalar_count_order)
            || (steps.is_empty()
                && kind != JoinType::Inner
                && !matches!(aggregate.as_ref(), Some(JoinAggregate::CountStar))))
    {
        return Err(ParseError(
            "JOIN aggregates require no ORDER BY; two-table OUTER JOIN supports COUNT(*) only"
                .into(),
        ));
    }
    if !scalar_projections.is_empty() {
        let projected_columns = columns
            .as_ref()
            .ok_or_else(|| ParseError("JOIN scalar projections require row output".into()))?;
        let grouped_case_literals = grouped_case_inner_chain
            && scalar_projections.iter().all(|projection| {
                matches!(projection.expression, crate::ScalarExpr::TextLiteral(_))
            });
        if (grouped_count_projection && !grouped_case_literals)
            || aggregate.is_some()
            || !scalar_aggregates.is_empty()
            || scalar_sum.is_some()
            || scalar_formula.is_some()
            || projection_order.len() != projected_columns.len() + scalar_projections.len()
        {
            return Err(ParseError(
                "JOIN scalar projections require a simple row-level SELECT".into(),
            ));
        }
    }
    Ok(Some(JoinSelect {
        left,
        right,
        left_alias,
        right_alias,
        kind,
        any,
        columns,
        scalar_projections,
        projection_order,
        aggregate,
        scalar_aggregates,
        aggregate_alias,
        grouped_count,
        grouped_execution_keys,
        grouped_output,
        grouped_keys_only,
        grouped_count_column,
        on_right_like,
        grouped_distinct,
        grouped_case_counts,
        grouped_sum,
        grouped_sum_extras,
        grouped_min,
        grouped_min_metrics,
        grouped_max_metrics,
        grouped_avg_metrics,
        grouped_window_average,
        grouped_rollup,
        grouped_formula,
        grouped_window_ratio,
        grouped_year,
        grouped_key_aliases,
        grouped_sum_position,
        grouped_sum_having,
        grouped_count_having,
        scalar_sum,
        scalar_sum_qualifier,
        scalar_result_typed: false,
        scalar_formula,
        scalar_where,
        in_subquery,
        keys,
        using_keys,
        where_null,
        where_comparison,
        where_comparisons_extra,
        where_column_comparisons,
        where_substring_comparisons,
        where_in_filters,
        where_like_filters,
        order_by,
        limit,
        offset,
        steps,
        virtual_columns: Vec::new(),
    }))
}
