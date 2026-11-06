//! SELECT token stream parsing, separate from the public query API.

use super::*;

pub(super) fn parse_select_tokens(tokens: Vec<Token>) -> Result<Select, ParseError> {
    let mut parser = Parser { tokens, index: 0 };
    if !parser.keyword("SELECT") {
        if matches!(parser.tokens.first(), Some(Token::Word(word)) if word.eq_ignore_ascii_case("WITH"))
        {
            return Err(ParseError(
                "WITH common table expressions are not yet supported".into(),
            ));
        }
        return Err(ParseError("only SELECT statements are supported".into()));
    }
    let mut distinct =
        parser.keyword("DISTINCT") || parser.keyword("DISTINCTROW") || parser.keyword("UNIQUE");
    if !distinct {
        parser.keyword("ALL");
    }
    if matches!(
        parser.tokens.get(parser.index),
        Some(Token::Word(word))
            if ["ALL", "DISTINCT", "DISTINCTROW", "UNIQUE"]
                .iter()
                .any(|modifier| word.eq_ignore_ascii_case(modifier))
    ) {
        return Err(ParseError(
            "SELECT accepts only one DISTINCT or ALL modifier".into(),
        ));
    }
    let top_limit = if parser.keyword("TOP") {
        let parenthesized = parser.take(Token::LParen);
        let count = parser.number()?;
        if !count.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError("TOP must be a non-negative integer".into()));
        }
        let count = count
            .parse::<usize>()
            .map_err(|_| ParseError("TOP is too large".into()))?;
        if parenthesized && !parser.take(Token::RParen) {
            return Err(ParseError("TOP requires closing ')'".into()));
        }
        Some(count)
    } else {
        None
    };
    let mut literal_alias = None;

    let multiple = multi_aggregate::parse_multiple(&mut parser)?;
    let mut grouped_expressions = Vec::new();
    let mut projection = if let Some(items) = multiple {
        Projection::MultipleAggregates(items)
    } else if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("LENGTH"))
        && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
        && matches!(parser.tokens.get(parser.index + 2), Some(Token::Word(name)) if !name.eq_ignore_ascii_case("NULL"))
        && parser.tokens.get(parser.index + 3) == Some(&Token::RParen)
    {
        parser.keyword("LENGTH");
        parser.take(Token::LParen);
        let column = parser.word()?;
        parser.take(Token::RParen);
        Projection::StringLength { column }
    } else if parser.take(Token::Star) {
        Projection::All
    } else if matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
        && parser.keyword("COUNT")
    {
        if !parser.take(Token::LParen) {
            return Err(ParseError("expected COUNT() or COUNT(*)".into()));
        }
        let all = if !matches!(
            parser.tokens.get(parser.index),
            Some(Token::Word(word)) if word.eq_ignore_ascii_case("DISTINCT")
        ) {
            parser.keyword("ALL")
        } else {
            false
        };
        if all && parser.tokens.get(parser.index) == Some(&Token::RParen) {
            return Err(ParseError("COUNT(ALL) requires an argument".into()));
        }
        if parser.keyword("DISTINCT") {
            let column = parser.word()?;
            if !parser.take(Token::RParen) {
                return Err(ParseError(
                    "expected ')' after COUNT(DISTINCT column)".into(),
                ));
            }
            Projection::CountDistinct { column }
        } else if parser.take(Token::RParen) {
            Projection::Count
        } else if parser.take(Token::Star) {
            if !parser.take(Token::RParen) {
                return Err(ParseError("expected ')' after COUNT(*)".into()));
            }
            Projection::Count
        } else if matches!(parser.tokens.get(parser.index), Some(Token::Number(_))) {
            // A non-NULL constant contributes one count per input row.
            parser.index += 1;
            if !parser.take(Token::RParen) {
                return Err(ParseError("expected ')' after COUNT(constant)".into()));
            }
            Projection::Count
        } else {
            let column = parser.word()?;
            if !parser.take(Token::RParen) {
                return Err(ParseError("expected ')' after COUNT(column)".into()));
            }
            Projection::CountColumn { column }
        }
    } else if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("UNIQSTATE"))
        && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
    {
        parser.keyword("UNIQSTATE");
        parser.take(Token::LParen);
        let column = parser.word()?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after uniqState(column)".into()));
        }
        Projection::ApproxDistinctState { column }
    } else if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("UNIQ") || name.eq_ignore_ascii_case("APPROX_COUNT_DISTINCT"))
        && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
    {
        if !parser.keyword("UNIQ") {
            parser.keyword("APPROX_COUNT_DISTINCT");
        }
        parser.take(Token::LParen);
        let column = parser.word()?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after uniq(column)".into()));
        }
        Projection::ApproxDistinct { column }
    } else if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("HISTOGRAM"))
        && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
    {
        parser.keyword("HISTOGRAM");
        parser.take(Token::LParen);
        let bins = parser
            .number()?
            .parse::<usize>()
            .map_err(|_| ParseError("histogram bins must be an integer".into()))?;
        if !(1..=256).contains(&bins) {
            return Err(ParseError(
                "histogram requires between 1 and 256 bins".into(),
            ));
        }
        if !parser.take(Token::RParen) || !parser.take(Token::LParen) {
            return Err(ParseError(
                "histogram requires histogram(bins)(column)".into(),
            ));
        }
        let column = parser.word()?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after histogram column".into()));
        }
        Projection::Histogram { column, bins }
    } else if matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
        && !matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if scalar::is_function(word) || text_scalar::is_function(word))
    {
        let name = parser.word()?;
        let function = match name.to_ascii_uppercase().as_str() {
            "GROUPBITXOR" | "BIT_XOR" => AggregateFunction::GroupBitXor,
            "GROUPBITOR" | "BIT_OR" => AggregateFunction::GroupBitOr,
            "GROUPBITAND" | "BIT_AND" => AggregateFunction::GroupBitAnd,
            "ANYLAST" => AggregateFunction::AnyLast,
            "ANY" | "ANY_VALUE" => AggregateFunction::Any,
            "COUNTIF" => AggregateFunction::CountIf,
            "SUM" => AggregateFunction::Sum,
            "PRODUCT" => AggregateFunction::Product,
            "AVG" => AggregateFunction::Avg,
            "SUMSQUARES" => AggregateFunction::SumSquares,
            "MIN" => AggregateFunction::Min,
            "RANGE" => AggregateFunction::Range,
            "MAX" => AggregateFunction::Max,
            "RMS" => AggregateFunction::Rms,
            "VARPOP" => AggregateFunction::VarPop,
            "STDDEVPOP" => AggregateFunction::StddevPop,
            "VARSAMP" | "VARIANCE" => AggregateFunction::VarSamp,
            "STDDEVSAMP" | "STDDEV" => AggregateFunction::StddevSamp,
            "MEDIAN" => AggregateFunction::Median,
            _ => return Err(ParseError(format!("unsupported aggregate: {name}"))),
        };
        parser.take(Token::LParen);
        let column = parser.word()?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after aggregate column".into()));
        }
        Projection::Aggregate { function, column }
    } else if matches!(
        parser.tokens.get(parser.index),
        Some(Token::Number(_) | Token::String(_) | Token::LParen | Token::Plus | Token::Minus)
    ) || matches!(
        parser.tokens.get(parser.index),
        Some(Token::Word(word))
            if word.eq_ignore_ascii_case("NULL")
                || (parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
                    && (scalar::is_function(word) || text_scalar::is_function(word)))
    ) {
        let mut columns = Vec::new();
        let is_text_scalar = matches!(
            parser.tokens.get(parser.index),
            Some(Token::Word(name)) if text_scalar::is_function(name)
                && (!scalar::is_function(name)
                    || matches!(parser.tokens.get(parser.index + 2), Some(Token::String(_)))
                    || matches!(parser.tokens.get(parser.index + 2),
                        Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL")))
        );
        loop {
            if columns.len() >= 64 {
                return Err(ParseError("SELECT supports at most 64 constants".into()));
            }
            let value = match parser.tokens.get(parser.index) {
                Some(Token::Number(number))
                    if number.contains('.') || number.contains('e') || number.contains('E') =>
                {
                    let number = number.clone();
                    if !number.parse::<f64>().is_ok_and(f64::is_finite) {
                        return Err(ParseError("SELECT decimal literal must be finite".into()));
                    }
                    parser.index += 1;
                    ConstantValue::Decimal(number)
                }
                Some(Token::Number(_) | Token::LParen | Token::Plus | Token::Minus) => {
                    ConstantValue::Integer(scalar::parse(&mut parser)?)
                }
                Some(Token::Word(name))
                    if scalar::is_function(name)
                        && text_scalar::is_function(name)
                        && (matches!(
                            parser.tokens.get(parser.index + 2),
                            Some(Token::String(_))
                        ) || matches!(parser.tokens.get(parser.index + 2),
                                Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL"))) =>
                {
                    text_scalar::parse(&mut parser)?
                }
                Some(Token::Word(name)) if scalar::is_function(name) => {
                    ConstantValue::Integer(scalar::parse(&mut parser)?)
                }
                Some(Token::Word(name)) if text_scalar::is_function(name) => {
                    text_scalar::parse(&mut parser)?
                }
                Some(Token::String(value)) => {
                    let value = value.clone();
                    parser.index += 1;
                    ConstantValue::Text(value)
                }
                Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL") => {
                    parser.index += 1;
                    ConstantValue::Null
                }
                _ => return Err(ParseError("expected a constant after comma".into())),
            };
            let alias = parser.select_output_alias()?;
            columns.push(Constant { value, alias });
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if is_text_scalar {
            Projection::Constants(columns)
        } else if let [
            Constant {
                value: ConstantValue::Integer(value),
                alias,
            },
        ] = columns.as_slice()
        {
            literal_alias = alias.clone();
            Projection::Literal(*value)
        } else {
            Projection::Constants(columns)
        }
    } else {
        let mut columns = vec![parser.word()?];
        let mut grouped_count = false;
        let mut grouped_distinct = None;
        let mut grouped_aggregate = None;
        let mut grouped_aggregates = None;
        while parser.take(Token::Comma) {
            if matches!(parser.tokens.get(parser.index),
                Some(Token::Word(word)) if word.eq_ignore_ascii_case("FROM"))
            {
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("COUNT"))
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
                && matches!(
                    parser.tokens.get(parser.index + 2),
                    Some(Token::Word(word)) if word.eq_ignore_ascii_case("DISTINCT")
                )
                && !distinct_count_has_following_metric(&parser)
            {
                // Keep the specialized COUNT(DISTINCT column) projection.
                if columns.len() > 8 {
                    return Err(ParseError("GROUP BY supports at most 8 keys".into()));
                }
                parser.keyword("COUNT");
                parser.take(Token::LParen);
                parser.keyword("DISTINCT");
                let value_column = parser.word()?;
                if !parser.take(Token::RParen) {
                    return Err(ParseError(
                        "expected ')' after COUNT(DISTINCT column)".into(),
                    ));
                }
                grouped_distinct = Some(value_column);
                break;
            }
            if parser.tokens.get(parser.index + 1) == Some(&Token::LParen) {
                let function = match parser.tokens.get(parser.index) {
                    Some(Token::Word(name)) => match name.to_ascii_uppercase().as_str() {
                        "GROUPBITXOR" | "BIT_XOR" => Some(AggregateFunction::GroupBitXor),
                        "GROUPBITOR" | "BIT_OR" => Some(AggregateFunction::GroupBitOr),
                        "GROUPBITAND" | "BIT_AND" => Some(AggregateFunction::GroupBitAnd),
                        "ANYLAST" => Some(AggregateFunction::AnyLast),
                        "ANY" | "ANY_VALUE" => Some(AggregateFunction::Any),
                        "COUNTIF" => Some(AggregateFunction::CountIf),
                        "COUNT" => Some(AggregateFunction::Count),
                        "SUM" | "SUMIF" => Some(AggregateFunction::Sum),
                        "PRODUCT" => Some(AggregateFunction::Product),
                        "AVG" | "AVGIF" => Some(AggregateFunction::Avg),
                        "SUMSQUARES" => Some(AggregateFunction::SumSquares),
                        "MIN" | "MINIF" => Some(AggregateFunction::Min),
                        "RANGE" => Some(AggregateFunction::Range),
                        "MAX" | "MAXIF" => Some(AggregateFunction::Max),
                        "RMS" => Some(AggregateFunction::Rms),
                        "VARPOP" => Some(AggregateFunction::VarPop),
                        "STDDEVPOP" => Some(AggregateFunction::StddevPop),
                        "VARSAMP" | "VARIANCE" => Some(AggregateFunction::VarSamp),
                        "STDDEVSAMP" | "STDDEV" => Some(AggregateFunction::StddevSamp),
                        "MEDIAN" => Some(AggregateFunction::Median),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(function) = function {
                    let conditional = matches!(parser.tokens.get(parser.index),
                        Some(Token::Word(name)) if matches!(name.to_ascii_uppercase().as_str(),
                            "SUMIF" | "AVGIF" | "MINIF" | "MAXIF"));
                    parser.index += 1;
                    parser.take(Token::LParen);
                    let function =
                        if function == AggregateFunction::Count && parser.keyword("DISTINCT") {
                            AggregateFunction::CountDistinct
                        } else {
                            function
                        };
                    let (value_column, case_filter, expr) =
                        grouped_metric_with_expression(&mut parser, function)?;
                    let function = if function == AggregateFunction::Sum && value_column == "*" {
                        AggregateFunction::Count
                    } else {
                        function
                    };
                    if conditional && case_filter.is_some() {
                        return Err(ParseError("SUMIF cannot nest a CASE filter".into()));
                    }
                    let filter = if conditional {
                        if !parser.take(Token::Comma) {
                            return Err(ParseError(
                                "conditional aggregate requires filter column".into(),
                            ));
                        }
                        Some(grouped_metric_filter(&mut parser)?)
                    } else {
                        case_filter
                    };
                    if !parser.take(Token::RParen) {
                        return Err(ParseError("expected ')' after grouped aggregate".into()));
                    }
                    let mut expressions = vec![expr];
                    let mut items = vec![(function, value_column)];
                    let mut filters = vec![filter];
                    let mut aliases = vec![parser.select_output_alias()?];
                    while parser.take(Token::Comma) {
                        if items.len() >= 64 {
                            return Err(ParseError(
                                "grouped SELECT supports at most 64 numeric aggregates".into(),
                            ));
                        }
                        let name = parser.word()?;
                        let conditional = matches!(
                            name.to_ascii_uppercase().as_str(),
                            "SUMIF" | "AVGIF" | "MINIF" | "MAXIF"
                        );
                        let function = match name.to_ascii_uppercase().as_str() {
                            "MIN" | "MINIF" => AggregateFunction::Min,
                            "MAX" | "MAXIF" => AggregateFunction::Max,
                            "AVG" | "AVGIF" => AggregateFunction::Avg,
                            "SUM" | "SUMIF" => AggregateFunction::Sum,
                            "COUNT" => AggregateFunction::Count,
                            "COUNTIF" => AggregateFunction::CountIf,
                            "PRODUCT" => AggregateFunction::Product,
                            "SUMSQUARES" => AggregateFunction::SumSquares,
                            "RANGE" => AggregateFunction::Range,
                            "RMS" => AggregateFunction::Rms,
                            "VARPOP" => AggregateFunction::VarPop,
                            "VARSAMP" | "VARIANCE" => AggregateFunction::VarSamp,
                            "STDDEVPOP" => AggregateFunction::StddevPop,
                            "STDDEVSAMP" => AggregateFunction::StddevSamp,
                            "MEDIAN" => AggregateFunction::Median,
                            "ANY" | "ANY_VALUE" => AggregateFunction::Any,
                            "ANYLAST" => AggregateFunction::AnyLast,
                            "GROUPBITAND" | "BIT_AND" => AggregateFunction::GroupBitAnd,
                            "GROUPBITOR" | "BIT_OR" => AggregateFunction::GroupBitOr,
                            "GROUPBITXOR" | "BIT_XOR" => AggregateFunction::GroupBitXor,
                            _ => {
                                return Err(ParseError(format!(
                                    "unsupported grouped aggregate: {name}"
                                )));
                            }
                        };
                        if !parser.take(Token::LParen) {
                            return Err(ParseError("expected '(' after aggregate name".into()));
                        }
                        let function =
                            if function == AggregateFunction::Count && parser.keyword("DISTINCT") {
                                AggregateFunction::CountDistinct
                            } else {
                                function
                            };
                        let (value_column, case_filter, expr) =
                            grouped_metric_with_expression(&mut parser, function)?;
                        let function = if function == AggregateFunction::Sum && value_column == "*"
                        {
                            AggregateFunction::Count
                        } else {
                            function
                        };
                        if conditional && case_filter.is_some() {
                            return Err(ParseError("SUMIF cannot nest a CASE filter".into()));
                        }
                        let filter = if conditional {
                            if !parser.take(Token::Comma) {
                                return Err(ParseError(
                                    "conditional aggregate requires filter column".into(),
                                ));
                            }
                            Some(grouped_metric_filter(&mut parser)?)
                        } else {
                            case_filter
                        };
                        if !parser.take(Token::RParen) {
                            return Err(ParseError("expected ')' after grouped aggregate".into()));
                        }
                        expressions.push(expr);
                        items.push((function, value_column));
                        filters.push(filter);
                        aliases.push(parser.select_output_alias()?);
                    }
                    let mut seen = std::collections::HashSet::new();
                    for alias in aliases.iter().flatten() {
                        if columns
                            .iter()
                            .any(|column| column.eq_ignore_ascii_case(alias))
                            || !seen.insert(alias.to_ascii_lowercase())
                        {
                            return Err(ParseError(
                                "duplicate or ambiguous aggregate alias".into(),
                            ));
                        }
                    }
                    if items.len() == 1
                        && items[0].0 == AggregateFunction::Count
                        && items[0].1 == "*"
                        && aliases[0].is_none()
                        && filters[0].is_none()
                    {
                        // Preserve the existing COUNT(*) fast path.
                        grouped_count = true;
                    } else if items.len() == 1
                        && expressions[0].is_none()
                        && columns.len() == 1
                        && aliases[0].is_none()
                        && filters[0].is_none()
                    {
                        grouped_aggregate = items.pop();
                    } else {
                        grouped_expressions = if expressions.iter().any(Option::is_some) {
                            expressions
                        } else {
                            Vec::new()
                        };
                        grouped_aggregates = Some((items, aliases, filters));
                    }
                    break;
                }
            }
            columns.push(parser.word()?);
        }
        if grouped_count {
            if columns.len() > 8 {
                return Err(ParseError("GROUP BY supports at most 8 keys".into()));
            }
            if columns.len() == 1 {
                Projection::GroupedCount {
                    column: columns.remove(0),
                }
            } else {
                Projection::GroupedMultiCount { columns }
            }
        } else if let Some(value_column) = grouped_distinct {
            if columns.len() == 1 {
                Projection::GroupedDistinctCount {
                    column: columns.remove(0),
                    value_column,
                }
            } else {
                Projection::GroupedMultiDistinctCount {
                    columns,
                    value_column,
                }
            }
        } else if let Some((function, value_column)) = grouped_aggregate {
            Projection::GroupedAggregate {
                column: columns.remove(0),
                function,
                value_column,
            }
        } else if let Some((items, aliases, filters)) = grouped_aggregates {
            if columns.len() == 1 {
                Projection::GroupedAggregates {
                    column: columns.remove(0),
                    items,
                    aliases,
                    filters,
                }
            } else {
                if columns.len() > 8 {
                    return Err(ParseError("GROUP BY supports at most 8 keys".into()));
                }
                Projection::GroupedMultiAggregates {
                    columns,
                    items,
                    aliases,
                    filters,
                }
            }
        } else {
            Projection::Columns(columns)
        }
    };

    if distinct
        && !matches!(
            projection,
            Projection::All
                | Projection::Columns(_)
                | Projection::StringLength { .. }
                | Projection::Literal(_)
                | Projection::Constants(_)
        )
    {
        return Err(ParseError(
            "DISTINCT supports only row projections or LENGTH".into(),
        ));
    }
    let rename = if parser.keyword("RENAME") {
        if !matches!(projection, Projection::All) {
            return Err(ParseError("RENAME requires SELECT *".into()));
        }
        let source = parser.word()?;
        if !parser.keyword("AS") {
            return Err(ParseError("RENAME requires AS".into()));
        }
        Some((source, parser.word()?))
    } else {
        None
    };

    let alias = if let Some(alias) = literal_alias {
        Some(alias)
    } else {
        let single_result = matches!(
            &projection,
            Projection::Literal(_)
                | Projection::Count
                | Projection::CountColumn { .. }
                | Projection::CountDistinct { .. }
                | Projection::ApproxDistinct { .. }
                | Projection::Histogram { .. }
                | Projection::ApproxDistinctState { .. }
                | Projection::Aggregate { .. }
                | Projection::GroupedCount { .. }
                | Projection::GroupedDistinctCount { .. }
                | Projection::GroupedMultiCount { .. }
                | Projection::GroupedMultiDistinctCount { .. }
                | Projection::StringLength { .. }
        ) || matches!(&projection, Projection::Columns(columns) if columns.len() == 1);
        if single_result {
            parser.select_output_alias()?
        } else if parser.keyword("AS") {
            return Err(ParseError("AS supports only a single result column".into()));
        } else {
            None
        }
    };

    let source = if parser.keyword("FROM") {
        if parser.tokens.get(parser.index) == Some(&Token::LParen) {
            return Err(ParseError(
                "derived tables in FROM are not yet supported".into(),
            ));
        }
        let name = parser.word()?;
        if name.eq_ignore_ascii_case("DUAL")
            && matches!(
                projection,
                Projection::Literal(_) | Projection::Constants(_)
            )
        {
            None
        } else if name.eq_ignore_ascii_case("numbers") {
            if !parser.take(Token::LParen) {
                return Err(ParseError("numbers requires a row count".into()));
            }
            let count = parser.number()?;
            if !count.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(ParseError("numbers row count must be non-negative".into()));
            }
            let first = count
                .parse::<u64>()
                .map_err(|_| ParseError("numbers argument is too large".into()))?;
            let source = if parser.take(Token::Comma) {
                let count = parser.number()?;
                if !count.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(ParseError("numbers row count must be non-negative".into()));
                }
                let count = count
                    .parse::<u64>()
                    .map_err(|_| ParseError("numbers row count is too large".into()))?;
                if count > 1_000_000_000 {
                    return Err(ParseError(
                        "numbers supports at most 1000000000 rows".into(),
                    ));
                }
                if parser.take(Token::Comma) {
                    let step = parser
                        .number()?
                        .parse::<u64>()
                        .map_err(|_| ParseError("numbers step must be a positive UInt64".into()))?;
                    if step == 0 {
                        return Err(ParseError("numbers step must be positive".into()));
                    }
                    Source::NumbersStepped {
                        start: first,
                        length: count,
                        step,
                    }
                } else {
                    Source::NumbersRange {
                        start: first,
                        count,
                    }
                }
            } else {
                if first > 1_000_000_000 {
                    return Err(ParseError(
                        "numbers supports at most 1000000000 rows".into(),
                    ));
                }
                Source::Numbers(first)
            };
            let last_offset = match &source {
                Source::NumbersRange { count, .. } if *count > 0 => Some(*count - 1),
                Source::NumbersStepped { length, step, .. } if *length > 0 => {
                    Some(((*length - 1) / *step) * *step)
                }
                _ => None,
            };
            if last_offset.is_some_and(|offset| first.checked_add(offset).is_none()) {
                return Err(ParseError("numbers range overflows UInt64".into()));
            }
            if !parser.take(Token::RParen) {
                return Err(ParseError("expected ')' after numbers row count".into()));
            }
            Some(source)
        } else if name.eq_ignore_ascii_case("generate_series") {
            if !parser.take(Token::LParen) {
                return Err(ParseError(
                    "generate_series requires (start, stop[, step])".into(),
                ));
            }
            let parse_argument = |parser: &mut Parser, label: &str| -> Result<u64, ParseError> {
                let value = parser.number()?;
                if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(ParseError(format!(
                        "generate_series {label} must be a non-negative UInt64"
                    )));
                }
                value
                    .parse::<u64>()
                    .map_err(|_| ParseError(format!("generate_series {label} is too large")))
            };
            let start = parse_argument(&mut parser, "start")?;
            if !parser.take(Token::Comma) {
                return Err(ParseError(
                    "generate_series requires a stop argument".into(),
                ));
            }
            let stop = parse_argument(&mut parser, "stop")?;
            let step = if parser.take(Token::Comma) {
                parse_argument(&mut parser, "step")?
            } else {
                1
            };
            if step == 0 {
                return Err(ParseError("generate_series step must be positive".into()));
            }
            if !parser.take(Token::RParen) {
                return Err(ParseError(
                    "expected ')' after generate_series arguments".into(),
                ));
            }
            let count = if stop < start {
                0
            } else {
                ((stop - start) / step).checked_add(1).ok_or_else(|| {
                    ParseError("generate_series supports at most 10000000 rows".into())
                })?
            };
            if count > 10_000_000 {
                return Err(ParseError(
                    "generate_series supports at most 10000000 rows".into(),
                ));
            }
            Some(Source::GenerateSeries { start, count, step })
        } else {
            Some(Source::Table(name))
        }
    } else {
        None
    };
    // The single-source SELECT executor addresses unqualified columns by
    // their physical names; an AS alias does not change that lookup.
    let source_alias = if parser.keyword("AS") {
        true
    } else {
        matches!(parser.tokens.get(parser.index), Some(Token::QuotedWord(_)))
            || matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if ![
                "PREWHERE", "WHERE", "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET",
                "FETCH", "FORMAT", "SETTINGS", "JOIN", "INNER", "LEFT", "RIGHT",
                "FULL", "CROSS", "ON", "USING", "UNION", "INTERSECT", "EXCEPT",
                "DISTINCT", "DISTINCTROW", "UNIQUE", "ALL",
            ].iter().any(|keyword| word.eq_ignore_ascii_case(keyword)))
    };
    if source_alias {
        if source.is_none() {
            return Err(ParseError("source alias requires FROM".into()));
        }
        parser.bounded_word()?;
    }
    if source.is_some() && parser.tokens.get(parser.index) == Some(&Token::Comma) {
        return Err(ParseError(
            "comma-separated FROM tables are not yet supported; use explicit JOIN".into(),
        ));
    }
    if source.is_none()
        && matches!(
            projection,
            Projection::Aggregate { .. } | Projection::MultipleAggregates(_)
        )
        && matches!(
            parser.tokens.get(parser.index),
            Some(Token::Plus | Token::Minus | Token::Star | Token::Slash | Token::Percent)
        )
    {
        return Err(ParseError(
            "arithmetic on aggregate results is not yet supported".into(),
        ));
    }
    if source.is_none()
        && matches!(projection, Projection::Constants(_))
        && matches!(
            parser.tokens.get(parser.index),
            Some(Token::Plus | Token::Minus | Token::Star | Token::Slash | Token::Percent)
        )
    {
        return Err(ParseError(
            "arithmetic after SELECT decimal constants is not yet supported".into(),
        ));
    }
    if source.is_none()
        && !matches!(
            projection,
            Projection::Literal(_) | Projection::Constants(_)
        )
    {
        return Err(ParseError("a table source is required".into()));
    }
    // ClickHouse evaluates PREWHERE as an additional filtering stage. Lowering
    // PREWHERE AND WHERE into the shared bounded DNF preserves OR precedence
    // and uses the same typed predicate executor for all table sources.
    let prewhere = if parser.keyword("PREWHERE") {
        if source.is_none() {
            return Err(ParseError("PREWHERE requires a table source".into()));
        }
        Some(where_parser::parse_where(&mut parser)?)
    } else {
        None
    };
    let where_clause = if parser.keyword("WHERE") {
        if source.is_none() {
            return Err(ParseError("WHERE requires a table source".into()));
        }
        Some(where_parser::parse_where(&mut parser)?)
    } else {
        None
    };
    let branches = match (prewhere, where_clause) {
        (Some(prewhere), Some(where_clause)) => where_parser::combine_and(prewhere, where_clause)?,
        (Some(branches), None) | (None, Some(branches)) => branches,
        (None, None) => Vec::new(),
    };
    let mut branches = branches.into_iter();
    let predicates = branches.next().unwrap_or_default();
    let or_predicates = branches.collect();

    let mut hidden_aggregate = false;
    if parser.keyword("GROUP") {
        if !parser.keyword("BY") {
            return Err(ParseError("expected BY after GROUP".into()));
        }
        // ClickHouse GROUP BY ALL groups by every non-aggregate output
        // expression. In this bounded SELECT dialect those expressions are
        // either the explicit grouping keys or all plain projected columns.
        if parser.keyword("ALL") {
            match &projection {
                Projection::GroupedCount { .. }
                | Projection::GroupedMultiCount { .. }
                | Projection::GroupedDistinctCount { .. }
                | Projection::GroupedMultiDistinctCount { .. }
                | Projection::GroupedAggregate { .. }
                | Projection::GroupedAggregates { .. }
                | Projection::GroupedMultiAggregates { .. } => {}
                Projection::All | Projection::Columns(_) => distinct = true,
                Projection::Count
                | Projection::CountColumn { .. }
                | Projection::CountDistinct { .. }
                | Projection::ApproxDistinct { .. }
                | Projection::ApproxDistinctState { .. }
                | Projection::Histogram { .. }
                | Projection::Aggregate { .. }
                | Projection::MultipleAggregates(_) => {}
                _ => {
                    return Err(ParseError(
                        "GROUP BY ALL requires columns or aggregate projections".into(),
                    ));
                }
            }
        } else {
            let mut group_columns = Vec::new();
            loop {
                if group_columns.len() >= 8 {
                    return Err(ParseError("GROUP BY supports at most 8 keys".into()));
                }
                let column = if matches!(parser.tokens.get(parser.index), Some(Token::Number(_))) {
                    let position = parser
                        .number()?
                        .parse::<usize>()
                        .map_err(|_| ParseError("GROUP BY position is too large".into()))?;
                    match &projection {
                        Projection::GroupedCount { column }
                        | Projection::GroupedDistinctCount { column, .. }
                        | Projection::GroupedAggregate { column, .. }
                        | Projection::GroupedAggregates { column, .. }
                            if position == 1 =>
                        {
                            column.clone()
                        }
                        Projection::GroupedMultiCount { columns }
                        | Projection::GroupedMultiAggregates { columns, .. }
                        | Projection::GroupedMultiDistinctCount { columns, .. } => position
                            .checked_sub(1)
                            .and_then(|index| columns.get(index))
                            .cloned()
                            .ok_or_else(|| {
                                ParseError("GROUP BY position is out of range".into())
                            })?,
                        _ => return Err(ParseError("GROUP BY position is out of range".into())),
                    }
                } else {
                    parser.word()?
                };
                group_columns.push(column);
                if !parser.take(Token::Comma) {
                    break;
                }
                let trailing = parser.tokens.get(parser.index).is_none_or(|token| {
                    matches!(token, Token::Semicolon)
                        || matches!(token, Token::Word(word)
                            if ["HAVING", "ORDER", "LIMIT", "OFFSET", "FETCH"]
                                .iter()
                                .any(|keyword| word.eq_ignore_ascii_case(keyword)))
                });
                if trailing {
                    break;
                }
            }
            // A grouped key may be selected without projecting the SUM used by HAVING.
            if let Projection::Columns(columns) = &projection {
                let key_matches = columns.len() == 1
                    && group_columns.len() == 1
                    && columns[0].eq_ignore_ascii_case(&group_columns[0]);
                let tokens = &parser.tokens[parser.index..];
                if key_matches
                    && matches!(tokens, [Token::Word(having), Token::Word(sum), Token::LParen, Token::Word(_), Token::RParen, ..] if having.eq_ignore_ascii_case("HAVING") && sum.eq_ignore_ascii_case("SUM"))
                {
                    if !matches!(source, Some(Source::Table(_))) {
                        return Err(ParseError(
                            "unprojected HAVING SUM requires a stored table".into(),
                        ));
                    }
                    let value_column = match &tokens[3] {
                        Token::Word(name) => name.clone(),
                        _ => unreachable!(),
                    };
                    projection = Projection::GroupedAggregate {
                        column: columns[0].clone(),
                        function: AggregateFunction::Sum,
                        value_column,
                    };
                    hidden_aggregate = true;
                }
            }
            match &projection {
                Projection::GroupedMultiCount { columns }
                    if columns.len() == group_columns.len()
                        && columns
                            .iter()
                            .zip(&group_columns)
                            .all(|(a, b)| a.eq_ignore_ascii_case(b)) => {}
                Projection::GroupedMultiAggregates { columns, .. }
                | Projection::GroupedMultiDistinctCount { columns, .. }
                    if columns.len() == group_columns.len()
                        && columns
                            .iter()
                            .zip(&group_columns)
                            .all(|(a, b)| a.eq_ignore_ascii_case(b)) => {}
                Projection::GroupedCount { column }
                | Projection::GroupedDistinctCount { column, .. }
                | Projection::GroupedAggregate { column, .. }
                | Projection::GroupedAggregates { column, .. }
                    if group_columns.len() == 1
                        && column.eq_ignore_ascii_case(&group_columns[0]) => {}
                Projection::GroupedCount { .. }
                | Projection::GroupedDistinctCount { .. }
                | Projection::GroupedMultiDistinctCount { .. }
                | Projection::GroupedMultiCount { .. }
                | Projection::GroupedAggregate { .. }
                | Projection::GroupedAggregates { .. }
                | Projection::GroupedMultiAggregates { .. } => {
                    return Err(ParseError(
                        "GROUP BY must match selected grouping keys".into(),
                    ));
                }
                _ => {
                    return Err(ParseError(
                        "GROUP BY requires SELECT key, aggregate(column)".into(),
                    ));
                }
            }
        }
    } else if matches!(
        projection,
        Projection::GroupedCount { .. }
            | Projection::GroupedDistinctCount { .. }
            | Projection::GroupedMultiDistinctCount { .. }
            | Projection::GroupedMultiCount { .. }
            | Projection::GroupedAggregate { .. }
            | Projection::GroupedAggregates { .. }
            | Projection::GroupedMultiAggregates { .. }
    ) {
        return Err(ParseError("grouped aggregates require GROUP BY".into()));
    }

    let having = if parser.keyword("HAVING") {
        let mut aggregate_index = None;
        if matches!(
            projection,
            Projection::GroupedCount { .. } | Projection::GroupedMultiCount { .. }
        ) {
            let using_alias = alias.as_ref().is_some_and(|value| {
                matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if word.eq_ignore_ascii_case(value))
            });
            if using_alias {
                parser.index += 1;
            } else {
                if !parser.keyword("COUNT") || !parser.take(Token::LParen) {
                    return Err(ParseError("HAVING requires COUNT(*)".into()));
                }
                if !(parser.take(Token::RParen)
                    || (parser.take(Token::Star) && parser.take(Token::RParen)))
                {
                    return Err(ParseError("HAVING requires COUNT() or COUNT(*)".into()));
                }
            }
        } else if let Projection::GroupedAggregate {
            function,
            value_column,
            ..
        } = &projection
        {
            // The grouped state can evaluate HAVING on all numeric metrics.
            // Invalid NULL results are removed by SQL HAVING semantics.
            if !matches!(
                function,
                AggregateFunction::Sum
                    | AggregateFunction::Min
                    | AggregateFunction::Max
                    | AggregateFunction::Avg
                    | AggregateFunction::Count
                    | AggregateFunction::CountIf
                    | AggregateFunction::Product
                    | AggregateFunction::SumSquares
                    | AggregateFunction::Range
                    | AggregateFunction::Rms
                    | AggregateFunction::VarPop
                    | AggregateFunction::VarSamp
                    | AggregateFunction::StddevPop
                    | AggregateFunction::StddevSamp
                    | AggregateFunction::Median
                    | AggregateFunction::Any
                    | AggregateFunction::AnyLast
                    | AggregateFunction::GroupBitAnd
                    | AggregateFunction::GroupBitOr
                    | AggregateFunction::GroupBitXor
            ) {
                return Err(ParseError("HAVING does not support this aggregate".into()));
            }
            let name = parser.word()?;
            if !name.eq_ignore_ascii_case(function.name())
                || !parser.take(Token::LParen)
                || parser.word()? != *value_column
                || !parser.take(Token::RParen)
            {
                return Err(ParseError(
                    "HAVING must match the selected aggregate".into(),
                ));
            }
        } else if let Projection::GroupedAggregates { items, aliases, .. }
        | Projection::GroupedMultiAggregates { items, aliases, .. } = &projection
        {
            let name = parser.word()?;
            // HAVING uses the same aggregate alias resolution as ORDER BY.
            // Only a projected metric can be filtered, so an alias never
            // selects a different or previously uncomputed aggregate.
            let selected = if parser.take(Token::LParen) {
                let column = if parser.take(Token::Star)
                    || parser.tokens.get(parser.index) == Some(&Token::RParen)
                {
                    "*".to_string()
                } else {
                    parser.word()?
                };
                if !parser.take(Token::RParen) {
                    return Err(ParseError("expected ')' after HAVING metric".into()));
                }
                items.iter().position(|(function, argument)| {
                    function.name().eq_ignore_ascii_case(&name) && argument == &column
                })
            } else {
                aliases.iter().position(|alias| {
                    alias
                        .as_ref()
                        .is_some_and(|alias| alias.eq_ignore_ascii_case(&name))
                })
            };
            aggregate_index = Some(
                selected
                    .filter(|&index| {
                        matches!(
                            items[index].0,
                            AggregateFunction::Sum
                                | AggregateFunction::Avg
                                | AggregateFunction::Min
                                | AggregateFunction::Max
                                | AggregateFunction::Count
                        )
                    })
                    .ok_or_else(|| ParseError("HAVING metric must match SELECT".into()))?,
            );
        } else if let Projection::GroupedDistinctCount { value_column, .. }
        | Projection::GroupedMultiDistinctCount { value_column, .. } = &projection
        {
            if !parser.keyword("COUNT")
                || !parser.take(Token::LParen)
                || !parser.keyword("DISTINCT")
                || parser.word()? != *value_column
                || !parser.take(Token::RParen)
            {
                return Err(ParseError(
                    "HAVING must match the selected COUNT(DISTINCT column)".into(),
                ));
            }
        } else {
            return Err(ParseError("HAVING requires a grouped aggregate".into()));
        }
        let operator = parser.operator()?;
        let value = parser
            .number()?
            .parse::<u64>()
            .map_err(|_| ParseError("HAVING threshold must be a UInt64".into()))?;
        Some(Having {
            operator,
            value,
            aggregate_index,
        })
    } else {
        None
    };

    let mut order_by_extra = Vec::new();
    let mut order_by = if parser.keyword("ORDER") {
        if !parser.keyword("BY") {
            return Err(ParseError("expected BY after ORDER".into()));
        }
        if parser.keyword("ALL") {
            let columns = if hidden_aggregate {
                match &projection {
                    Projection::GroupedAggregate { column, .. } => vec![column.clone()],
                    _ => unreachable!(),
                }
            } else {
                order_by_all_columns(&projection)?
            };
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
                    return Err(ParseError("NULLS requires FIRST or LAST".into()));
                }
            } else {
                None
            };
            let mut terms = columns.into_iter().map(|column| OrderBy {
                column,
                descending,
                nulls_first,
            });
            let first = terms.next().expect("ORDER BY ALL has columns");
            order_by_extra.extend(terms);
            Some(first)
        } else {
            let first = parser.order_term(&projection, hidden_aggregate)?;
            while parser.take(Token::Comma) {
                let trailing = parser.tokens.get(parser.index).is_none_or(|token| {
                    matches!(token, Token::Semicolon)
                        || matches!(token, Token::Word(word)
                            if ["LIMIT", "OFFSET", "FETCH"]
                                .iter()
                                .any(|keyword| word.eq_ignore_ascii_case(keyword)))
                });
                if trailing {
                    break;
                }
                if order_by_extra.len() >= 7 {
                    return Err(ParseError("ORDER BY supports at most 8 columns".into()));
                }
                order_by_extra.push(parser.order_term(&projection, hidden_aggregate)?);
            }
            Some(first)
        }
    } else {
        None
    };
    if matches!(
        projection,
        Projection::GroupedCount { .. }
            | Projection::GroupedDistinctCount { .. }
            | Projection::GroupedMultiDistinctCount { .. }
            | Projection::GroupedMultiCount { .. }
    ) {
        if let Some(count_alias) = alias.as_deref() {
            for term in order_by.iter_mut().chain(order_by_extra.iter_mut()) {
                if term.column == count_alias {
                    term.column = "count".into();
                }
            }
        }
    }
    if order_by.is_some()
        && matches!(
            projection,
            Projection::Count
                | Projection::CountColumn { .. }
                | Projection::CountDistinct { .. }
                | Projection::ApproxDistinct { .. }
                | Projection::Histogram { .. }
                | Projection::ApproxDistinctState { .. }
                | Projection::Aggregate { .. }
                | Projection::MultipleAggregates(_)
                | Projection::StringLength { .. }
                | Projection::Literal(_)
        )
    {
        return Err(ParseError("ORDER BY requires column projections".into()));
    }

    if let Projection::GroupedCount { column } | Projection::GroupedDistinctCount { column, .. } =
        &projection
    {
        if order_by
            .iter()
            .chain(order_by_extra.iter())
            .any(|order| &order.column != column && order.column != "count")
        {
            return Err(ParseError("grouped ORDER BY must use the group key".into()));
        }
    }
    if let Projection::GroupedMultiCount { columns }
    | Projection::GroupedMultiDistinctCount { columns, .. } = &projection
    {
        if order_by
            .iter()
            .chain(order_by_extra.iter())
            .any(|order| !columns.contains(&order.column) && order.column != "count")
        {
            return Err(ParseError(
                "grouped ORDER BY must use the group keys or count".into(),
            ));
        }
    }
    if let Projection::GroupedAggregate { column, .. } = &projection {
        if hidden_aggregate {
            if let Some(alias) = alias.as_deref() {
                for term in order_by.iter_mut().chain(order_by_extra.iter_mut()) {
                    if term.column.eq_ignore_ascii_case(alias) {
                        term.column = column.clone();
                    }
                }
            }
        }
        if order_by
            .iter()
            .chain(order_by_extra.iter())
            .any(|order| &order.column != column && order.column != "#aggregate")
        {
            return Err(ParseError(
                "grouped numeric ORDER BY must use the group key or aggregate".into(),
            ));
        }
    }
    if let Projection::GroupedAggregates { column, items, .. } = &projection {
        let metric = |name: &str| {
            name.strip_prefix("#aggregate")
                .and_then(|number| number.parse::<usize>().ok())
                .is_some_and(|index| index < items.len())
        };
        if order_by
            .iter()
            .chain(order_by_extra.iter())
            .any(|order| &order.column != column && !metric(&order.column))
        {
            return Err(ParseError(
                "grouped numeric ORDER BY must use the group key or a selected metric".into(),
            ));
        }
    }

    if let Projection::GroupedMultiAggregates { columns, items, .. } = &projection {
        let metric = |name: &str| {
            name.strip_prefix("#aggregate")
                .and_then(|number| number.parse::<usize>().ok())
                .is_some_and(|index| index < items.len())
        };
        if order_by
            .iter()
            .chain(order_by_extra.iter())
            .any(|order| !columns.contains(&order.column) && !metric(&order.column))
        {
            return Err(ParseError(
                "grouped numeric ORDER BY must use the group keys or selected metrics".into(),
            ));
        }
    }

    let mut comma_offset = None;
    let mut limit = if parser.keyword("LIMIT") {
        if parser.keyword("ALL") {
            // PostgreSQL treats LIMIT ALL as no limit, including with OFFSET.
            None
        } else {
            let parenthesized = parser.take(Token::LParen);
            let value = parser.number()?;
            if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(ParseError("LIMIT must be a non-negative integer".into()));
            }
            let first = value
                .parse::<usize>()
                .map_err(|_| ParseError("LIMIT is too large".into()))?;
            if parenthesized && !parser.take(Token::RParen) {
                return Err(ParseError("LIMIT requires closing ')'".into()));
            }
            if parser.take(Token::Comma) {
                let value = parser.number()?;
                if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(ParseError("LIMIT must be a non-negative integer".into()));
                }
                comma_offset = Some(first);
                Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| ParseError("LIMIT is too large".into()))?,
                )
            } else {
                Some(first)
            }
        }
    } else {
        None
    };

    // Accept both ClickHouse LIMIT pagination and SQL-standard OFFSET/FETCH.
    let offset = if parser.keyword("OFFSET") {
        if comma_offset.is_some() {
            return Err(ParseError(
                "cannot combine LIMIT comma syntax with OFFSET".into(),
            ));
        }
        let parenthesized = parser.take(Token::LParen);
        let value = parser.number()?;
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError("OFFSET must be a non-negative integer".into()));
        }
        let offset = value
            .parse::<usize>()
            .map_err(|_| ParseError("OFFSET is too large".into()))?;
        if parenthesized && !parser.take(Token::RParen) {
            return Err(ParseError("OFFSET requires closing ')'".into()));
        }
        if !parser.keyword("ROWS") {
            parser.keyword("ROW");
        }
        Some(offset)
    } else {
        comma_offset
    };

    if parser.keyword("FETCH") {
        if limit.is_some() {
            return Err(ParseError("cannot combine FETCH with LIMIT".into()));
        }
        if !parser.keyword("FIRST") && !parser.keyword("NEXT") {
            return Err(ParseError("FETCH requires FIRST or NEXT".into()));
        }
        // SQL permits FETCH FIRST ROW ONLY without a numeric count.
        let count = if matches!(parser.tokens.get(parser.index), Some(Token::Word(word))
            if word.eq_ignore_ascii_case("ROW") || word.eq_ignore_ascii_case("ROWS"))
        {
            1
        } else {
            let parenthesized = parser.take(Token::LParen);
            let value = parser.number()?;
            if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(ParseError(
                    "FETCH count must be a non-negative integer".into(),
                ));
            }
            let count = value
                .parse::<usize>()
                .map_err(|_| ParseError("FETCH count is too large".into()))?;
            if parenthesized && !parser.take(Token::RParen) {
                return Err(ParseError("FETCH count requires closing ')'".into()));
            }
            count
        };
        if !parser.keyword("ROWS") && !parser.keyword("ROW") {
            return Err(ParseError("FETCH requires ROW or ROWS".into()));
        }
        if !parser.keyword("ONLY") {
            return Err(ParseError("FETCH requires ONLY".into()));
        }
        limit = Some(count);
    }
    if top_limit.is_some() && (limit.is_some() || offset.is_some()) {
        return Err(ParseError(
            "TOP cannot be combined with LIMIT, OFFSET or FETCH".into(),
        ));
    }
    limit = top_limit.or(limit);
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError(
            "unexpected tokens after SELECT statement".into(),
        ));
    }
    Ok(Select {
        projection,
        grouped_literal: None,
        grouped_affine: Vec::new(),
        grouped_minute: None,
        grouped_extract_minute: None,
        grouped_case_key: None,
        grouped_regex: None,
        grouped_expressions,
        hidden_aggregate,
        alias,
        rename,
        distinct,
        source,
        predicates,
        or_predicates,
        order_by,
        order_by_extra,
        having,
        limit,
        offset,
    })
}
