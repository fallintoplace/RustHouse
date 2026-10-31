use crate::expression::parse_expression_tokens;
use crate::lexer::Token;
use crate::{AggregateFunction, ParseError, Parser, ScalarExpr};
use std::fmt::Write as _;

const MAX_AGGREGATE_ITEMS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateItem {
    pub function: Option<AggregateFunction>,
    pub argument: Option<ScalarExpr>,
    pub distinct: bool,
    pub alias: Option<String>,
}

impl AggregateItem {
    pub fn header(&self) -> String {
        if let Some(alias) = &self.alias {
            return alias.clone();
        }
        let function = self.function.map_or("count", AggregateFunction::name);
        let argument = self.argument.as_ref().map_or("*".into(), render_expr);
        if self.distinct {
            format!("count(DISTINCT {argument})")
        } else {
            format!("{function}({argument})")
        }
    }
}

fn render_expr(expr: &ScalarExpr) -> String {
    let mut output = String::new();
    render_expr_into(expr, &mut output);
    output
}

// Append directly to the final header instead of formatting a new String
// at every interior AST node. Long, bounded expressions are linear in output
// size rather than repeatedly copying their accumulated prefixes.
fn render_expr_into(expr: &ScalarExpr, output: &mut String) {
    match expr {
        ScalarExpr::Column(name) | ScalarExpr::Decimal(name) => output.push_str(name),
        ScalarExpr::TextLength(name) => {
            output.push_str("length(");
            output.push_str(name);
            output.push(')');
        }
        ScalarExpr::Integer(number) => {
            write!(output, "{number}").expect("writing to String cannot fail");
        }
        ScalarExpr::Case { branches, else_expr } => {
            output.push_str("CASE");
            for (condition, value) in branches {
                output.push_str(" WHEN ");
                render_expr_into(condition, output);
                output.push_str(" THEN ");
                render_expr_into(value, output);
            }
            if let Some(value) = else_expr {
                output.push_str(" ELSE ");
                render_expr_into(value, output);
            }
            output.push_str(" END");
        }
        ScalarExpr::Add(left, right) => {
            render_expr_into(left, output);
            output.push_str(" + ");
            render_expr_into(right, output);
        }
        ScalarExpr::Subtract(left, right) => {
            render_expr_into(left, output);
            output.push_str(" - ");
            let wrap = matches!(right.as_ref(), ScalarExpr::Add(..) | ScalarExpr::Subtract(..));
            if wrap { output.push('('); }
            render_expr_into(right, output);
            if wrap { output.push(')'); }
        }
        ScalarExpr::Divide(left, right) | ScalarExpr::Modulo(left, right) => {
            output.push('(');
            render_expr_into(left, output);
            output.push_str(if matches!(expr, ScalarExpr::Divide(..)) { ") / (" } else { ") % (" });
            render_expr_into(right, output);
            output.push(')');
        }
        ScalarExpr::Compare(left, operator, right) => {
            render_expr_into(left, output);
            let symbol = match operator {
                crate::Operator::Eq => "=",
                crate::Operator::Ne => "!=",
                crate::Operator::Lt => "<",
                crate::Operator::Le => "<=",
                crate::Operator::Gt => ">",
                crate::Operator::Ge => ">=",
                _ => unreachable!("comparisons only accept comparison operators"),
            };
            write!(output, " {symbol} ").expect("writing to String cannot fail");
            render_expr_into(right, output);
        }
        ScalarExpr::Multiply(left, right) => {
            for (index, operand) in [left.as_ref(), right.as_ref()].into_iter().enumerate() {
                if index != 0 { output.push_str(" * "); }
                let wrap = matches!(
                    operand,
                    ScalarExpr::Add(..) | ScalarExpr::Subtract(..) | ScalarExpr::Compare(..)
                );
                if wrap { output.push('('); }
                render_expr_into(operand, output);
                if wrap { output.push(')'); }
            }
        }
    }
}

fn aggregate_function(name: &str) -> Option<Option<AggregateFunction>> {
    match name.to_ascii_uppercase().as_str() {
        "COUNT" => Some(None),
        "SUM" => Some(Some(AggregateFunction::Sum)),
        "AVG" => Some(Some(AggregateFunction::Avg)),
        "MIN" => Some(Some(AggregateFunction::Min)),
        "MAX" => Some(Some(AggregateFunction::Max)),
        "COUNTIF" => Some(Some(AggregateFunction::CountIf)),
        "PRODUCT" => Some(Some(AggregateFunction::Product)),
        "SUMSQUARES" => Some(Some(AggregateFunction::SumSquares)),
        "RANGE" => Some(Some(AggregateFunction::Range)),
        "RMS" => Some(Some(AggregateFunction::Rms)),
        "VARPOP" => Some(Some(AggregateFunction::VarPop)),
        "VARSAMP" => Some(Some(AggregateFunction::VarSamp)),
        "STDDEVPOP" => Some(Some(AggregateFunction::StddevPop)),
        "STDDEVSAMP" => Some(Some(AggregateFunction::StddevSamp)),
        "MEDIAN" => Some(Some(AggregateFunction::Median)),
        "ANY" => Some(Some(AggregateFunction::Any)),
        "ANYLAST" => Some(Some(AggregateFunction::AnyLast)),
        "GROUPBITAND" => Some(Some(AggregateFunction::GroupBitAnd)),
        "GROUPBITOR" => Some(Some(AggregateFunction::GroupBitOr)),
        "GROUPBITXOR" => Some(Some(AggregateFunction::GroupBitXor)),
        _ => None,
    }
}

pub(crate) fn parse_multiple(
    parser: &mut Parser,
) -> Result<Option<Vec<AggregateItem>>, ParseError> {
    let start = parser.index;
    let mut items = Vec::new();
    loop {
        let Some(Token::Word(name)) = parser.tokens.get(parser.index) else {
            parser.index = start;
            return Ok(None);
        };
        let Some(function) = aggregate_function(name) else {
            if items.is_empty() {
                parser.index = start;
                return Ok(None);
            }
            return Err(ParseError("expected aggregate function after comma".into()));
        };
        if parser.tokens.get(parser.index + 1) != Some(&Token::LParen) {
            parser.index = start;
            return Ok(None);
        }
        parser.index += 2;
        let distinct = parser.keyword("DISTINCT");
        if distinct && function.is_some() {
            return Err(ParseError("DISTINCT is only supported for COUNT".into()));
        }
        let (argument, already_closed) = if parser.take(Token::Star) {
            (None, false)
        } else if parser.take(Token::RParen) {
            (None, true)
        } else {
            (Some(parse_expression_tokens(parser)?), false)
        };
        if function.is_some() && argument.is_none() {
            return Err(ParseError("numeric aggregate needs a column".into()));
        }
        if already_closed && distinct {
            return Err(ParseError("COUNT(DISTINCT) requires a column".into()));
        }
        if !already_closed && !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after aggregate argument".into()));
        }
        let alias = if parser.keyword("AS") {
            Some(parser.word()?)
        } else {
            None
        };
        if distinct && argument.is_none() {
            return Err(ParseError("COUNT(DISTINCT) requires a column".into()));
        }
        items.push(AggregateItem {
            function,
            argument,
            distinct,
            alias,
        });
        if items.len() >= MAX_AGGREGATE_ITEMS
            && parser.tokens.get(parser.index) == Some(&Token::Comma)
        {
            return Err(ParseError("SELECT supports at most 128 aggregates".into()));
        }
        if !parser.take(Token::Comma) {
            break;
        }
    }
    // Preserve the single-column parser, including rejection of COUNT(1).
    // Only a calculation referencing an input column needs this executor.
    let is_computed = items.len() == 1
        && items[0].argument.as_ref().is_some_and(|argument| {
            let mut columns = Vec::new();
            argument.referenced_columns(&mut columns);
            !columns.is_empty() && !matches!(argument, crate::ScalarExpr::Column(_))
        });
    if items.len() < 2 && !is_computed {
        parser.index = start;
        return Ok(None);
    }
    Ok(Some(items))
}

#[cfg(test)]
mod tests {
    use crate::{Projection, parse_select};

    #[test]
    fn count_empty_parentheses_cannot_bypass_aggregate_list_limit() {
        let within = format!("SELECT {} FROM numbers(1)",
            vec!["count()"; 128].join(", "));
        assert!(parse_select(&within).is_ok());
        let oversized = format!("SELECT {} FROM numbers(1)",
            vec!["count()"; 129].join(", "));
        assert!(parse_select(&oversized).is_err());
        let mixed = format!("SELECT {} FROM numbers(1)",
            [vec!["count()"; 128], vec!["sum(number)"]].concat().join(", "));
        assert!(parse_select(&mixed).is_err());
    }

    #[test]
    fn parses_mixed_aggregates_and_calculated_arguments() {
        let query = parse_select("SELECT SUM(width + 1), COUNT(*), AVG(width) FROM part").unwrap();
        let Projection::MultipleAggregates(items) = query.projection else {
            panic!("expected multiple aggregates");
        };
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].header(), "sum(width + 1)");
        assert_eq!(items[1].header(), "count(*)");
    }

    #[test]
    fn renders_nested_subtraction_without_losing_parentheses() {
        let query = parse_select("SELECT sum(a - (b - c)) FROM part").unwrap();
        let Projection::MultipleAggregates(items) = query.projection else {
            panic!("expected computed aggregate");
        };
        assert_eq!(items[0].header(), "sum(a - (b - c))");
    }

    #[test]
    fn parses_empty_and_star_counts_with_shared_argument_path() {
        let query = parse_select("SELECT count(), count(*), sum(width) FROM part").unwrap();
        let Projection::MultipleAggregates(items) = query.projection else {
            panic!("expected multiple aggregates");
        };
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].header(), "count(*)");
        assert_eq!(items[1].header(), "count(*)");
        assert_eq!(items[2].header(), "sum(width)");
    }

    #[test]
    fn parses_distinct_counts_with_other_aggregates() {
        let query =
            parse_select("SELECT count(DISTINCT customer), count(*), sum(amount) FROM part")
                .unwrap();
        let Projection::MultipleAggregates(items) = query.projection else {
            panic!("expected multiple aggregates");
        };
        assert!(items[0].distinct);
        assert!(!items[1].distinct);
        assert_eq!(items[0].header(), "count(DISTINCT customer)");
        assert!(parse_select("SELECT sum(DISTINCT amount), count(*) FROM part").is_err());
    }

    #[test]
    fn tpch_decimal_literals_in_computed_aggregates() {
        let query = parse_select(
            "SELECT sum(l_extendedprice * (1 - l_discount)), sum(l_quantity * 0.05) FROM lineitem"
        ).unwrap();
        let Projection::MultipleAggregates(items) = query.projection else {
            panic!("expected computed aggregates");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].header(), "sum(l_quantity * 0.05)");
        assert!(parse_select("SELECT sum(l_quantity * 0.05) FROM lineitem").is_ok());
    }

    #[test]
    fn parses_long_multi_aggregate_statements() {
        let expressions = (0..90)
            .map(|i| format!("sum(width + {i})"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT {expressions} FROM part");
        let Projection::MultipleAggregates(items) = parse_select(&sql).unwrap().projection else {
            panic!("expected multiple aggregates");
        };
        assert_eq!(items.len(), 90);
    }
}

#[cfg(test)]
mod aggregate_header_rendering_tests {
    use super::*;

    #[test]
    fn preserves_arithmetic_parentheses_and_case_format() {
        let plus = ScalarExpr::Add(
            Box::new(ScalarExpr::Column("x".into())),
            Box::new(ScalarExpr::Integer(2)),
        );
        let negative = ScalarExpr::Subtract(
            Box::new(ScalarExpr::Integer(10)),
            Box::new(plus.clone()),
        );
        assert_eq!(render_expr(&negative), "10 - (x + 2)");
        let product = ScalarExpr::Multiply(Box::new(plus), Box::new(negative));
        assert_eq!(render_expr(&product), "(x + 2) * (10 - (x + 2))");
        let case = ScalarExpr::Case {
            branches: vec![(ScalarExpr::Integer(1), product)],
            else_expr: Some(Box::new(ScalarExpr::Decimal("3.50".into()))),
        };
        assert_eq!(
            render_expr(&case),
            "CASE WHEN 1 THEN (x + 2) * (10 - (x + 2)) ELSE 3.50 END",
        );
    }

    #[test]
    fn long_left_deep_header_preserves_all_terms() {
        let mut expr = ScalarExpr::Integer(0);
        for _ in 0..120 {
            expr = ScalarExpr::Add(Box::new(expr), Box::new(ScalarExpr::Column("x".into())));
        }
        let text = render_expr(&expr);
        assert_eq!(text.matches(" + x").count(), 120);
        assert!(text.starts_with("0 + x"));
    }
}
