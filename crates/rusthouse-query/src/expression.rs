mod parser;

use self::parser::*;

use crate::lexer::{Token, tokenize};
use crate::{Operator, ParseError, Parser};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalarExpr {
    Column(String),
    TextLength(String),
    Integer(i64),
    Null,
    IsNull(Box<ScalarExpr>, bool),
    Not(Box<ScalarExpr>),
    Between(Box<ScalarExpr>, Box<ScalarExpr>, Box<ScalarExpr>, bool),
    In(Box<ScalarExpr>, Vec<ScalarExpr>, bool),
    Coalesce(Vec<ScalarExpr>),
    NullIf(Box<ScalarExpr>, Box<ScalarExpr>),
    Abs(Box<ScalarExpr>),
    Extremum(Vec<ScalarExpr>, bool),
    Clamp(Box<ScalarExpr>, Box<ScalarExpr>, Box<ScalarExpr>),
    Add(Box<ScalarExpr>, Box<ScalarExpr>),
    Subtract(Box<ScalarExpr>, Box<ScalarExpr>),
    Multiply(Box<ScalarExpr>, Box<ScalarExpr>),
    Divide(Box<ScalarExpr>, Box<ScalarExpr>),
    Modulo(Box<ScalarExpr>, Box<ScalarExpr>),
    And(Box<ScalarExpr>, Box<ScalarExpr>),
    Or(Box<ScalarExpr>, Box<ScalarExpr>),
    Compare(Box<ScalarExpr>, Operator, Box<ScalarExpr>),
    CaseWhen(Box<ScalarExpr>, Box<ScalarExpr>, Box<ScalarExpr>),
}

impl ScalarExpr {
    pub fn referenced_columns(&self, out: &mut Vec<String>) {
        let mut seen = out.iter().cloned().collect::<HashSet<_>>();
        self.collect_referenced_columns(out, &mut seen);
    }

    fn collect_referenced_columns(&self, out: &mut Vec<String>, seen: &mut HashSet<String>) {
        match self {
            Self::Column(name) | Self::TextLength(name) => {
                if seen.insert(name.clone()) {
                    out.push(name.clone());
                }
            }
            Self::Integer(_) | Self::Null => {}
            Self::IsNull(value, _) | Self::Not(value) | Self::Abs(value) =>
                value.collect_referenced_columns(out, seen),
            Self::Coalesce(values) | Self::Extremum(values, _) => {
                for value in values {
                    value.collect_referenced_columns(out, seen);
                }
            }
            Self::In(value, candidates, _) => {
                value.collect_referenced_columns(out, seen);
                for candidate in candidates {
                    candidate.collect_referenced_columns(out, seen);
                }
            }
            Self::Between(value, low, high, _) | Self::Clamp(value, low, high) => {
                value.collect_referenced_columns(out, seen);
                low.collect_referenced_columns(out, seen);
                high.collect_referenced_columns(out, seen);
            }
            Self::CaseWhen(condition, yes, no) => {
                condition.collect_referenced_columns(out, seen);
                yes.collect_referenced_columns(out, seen);
                no.collect_referenced_columns(out, seen);
            }
            Self::NullIf(left, right)
            | Self::Add(left, right)
            | Self::Subtract(left, right)
            | Self::Multiply(left, right)
            | Self::Divide(left, right)
            | Self::Modulo(left, right)
            | Self::And(left, right)
            | Self::Or(left, right)
            | Self::Compare(left, _, right) => {
                left.collect_referenced_columns(out, seen);
                right.collect_referenced_columns(out, seen);
            }
        }
    }
}

fn parse_arithmetic_tokens(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    let mut expr = product(parser)?;
    loop {
        let addition = if parser.take(Token::Plus) {
            true
        } else if parser.take(Token::Minus) {
            false
        } else {
            break;
        };
        let right = product(parser)?;
        expr = if addition {
            ScalarExpr::Add(Box::new(expr), Box::new(right))
        } else {
            ScalarExpr::Subtract(Box::new(expr), Box::new(right))
        };
    }
    Ok(expr)
}

/// Arithmetic binds before comparisons, comparisons before AND, and
/// AND before OR. CASE WHEN conditions use the same expression parser.
fn parse_comparison_tokens(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    let mut negations = 0usize;
    while parser.keyword("NOT") {
        negations += 1;
        if negations > 16 {
            return Err(ParseError("NOT nesting exceeds 16 levels".into()));
        }
    }
    let result = parse_comparison_core(parser)?;
    Ok(if negations % 2 == 0 { result } else { ScalarExpr::Not(Box::new(result)) })
}

fn parse_comparison_core(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    let left = parse_arithmetic_tokens(parser)?;
    let in_negated = matches!(
        (parser.tokens.get(parser.index), parser.tokens.get(parser.index + 1)),
        (Some(Token::Word(not)), Some(Token::Word(keyword)))
            if not.eq_ignore_ascii_case("NOT") && keyword.eq_ignore_ascii_case("IN")
    );
    if in_negated {
        parser.index += 1;
    }
    if parser.keyword("IN") {
        if !parser.take(Token::LParen) {
            return Err(ParseError("IN requires '('".into()));
        }
        let mut candidates = Vec::new();
        loop {
            if candidates.len() == 64 {
                return Err(ParseError("aggregate IN lists support at most 64 values".into()));
            }
            candidates.push(parse_arithmetic_tokens(parser)?);
            if !parser.take(Token::Comma) { break; }
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("IN requires ')'".into()));
        }
        return Ok(ScalarExpr::In(Box::new(left), candidates, in_negated));
    }
    let between_negated = parser.keyword("NOT");
    if between_negated || parser.keyword("BETWEEN") {
        if between_negated && !parser.keyword("BETWEEN") {
            return Err(ParseError("NOT after a value requires BETWEEN".into()));
        }
        let lower = parse_arithmetic_tokens(parser)?;
        if !parser.keyword("AND") {
            return Err(ParseError("BETWEEN requires AND".into()));
        }
        let upper = parse_arithmetic_tokens(parser)?;
        return Ok(ScalarExpr::Between(
            Box::new(left), Box::new(lower), Box::new(upper), between_negated,
        ));
    }
    if parser.keyword("IS") {
        let negated = parser.keyword("NOT");
        if !parser.keyword("NULL") {
            return Err(ParseError("IS requires NULL or NOT NULL".into()));
        }
        return Ok(ScalarExpr::IsNull(Box::new(left), negated));
    }
    if let Some(Token::Operator(operator)) = parser.tokens.get(parser.index) {
        let operator = *operator;
        if !matches!(operator, Operator::Eq | Operator::Ne | Operator::Lt
            | Operator::Le | Operator::Gt | Operator::Ge) {
            return Err(ParseError("invalid operator in numeric expression".into()));
        }
        parser.index += 1;
        let right = parse_arithmetic_tokens(parser)?;
        Ok(ScalarExpr::Compare(Box::new(left), operator, Box::new(right)))
    } else {
        Ok(left)
    }
}

fn parse_and_tokens(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    let mut left = parse_comparison_tokens(parser)?;
    while parser.keyword("AND") {
        let right = parse_comparison_tokens(parser)?;
        left = ScalarExpr::And(Box::new(left), Box::new(right));
    }
    Ok(left)
}

pub(crate) fn parse_expression_tokens(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    let mut left = parse_and_tokens(parser)?;
    while parser.keyword("OR") {
        let right = parse_and_tokens(parser)?;
        left = ScalarExpr::Or(Box::new(left), Box::new(right));
    }
    Ok(left)
}

pub fn parse_scalar_expression(sql: &str) -> Result<ScalarExpr, ParseError> {
    let mut parser = Parser {
        tokens: tokenize(sql)?,
        index: 0,
    };
    let expr = parse_expression_tokens(&mut parser)?;
    if parser.index != parser.tokens.len() {
        return Err(ParseError(
            "unexpected tokens after scalar expression".into(),
        ));
    }
    Ok(expr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_column_addition_and_subtraction() {
        let expr = parse_scalar_expression("ResolutionWidth + 89").unwrap();
        assert_eq!(
            expr,
            ScalarExpr::Add(
                Box::new(ScalarExpr::Column("ResolutionWidth".into())),
                Box::new(ScalarExpr::Integer(89)),
            )
        );
        for sql in ["ClientIP - 3", "ClientIP-3", "ClientIP -3"] {
            assert!(
                matches!(
                    parse_scalar_expression(sql).unwrap(),
                    ScalarExpr::Subtract(..)
                ),
                "{sql}"
            );
        }
    }

    #[test]
    fn parses_nested_expressions_and_collects_unique_columns() {
        let expr = parse_scalar_expression("(width + 2) - width").unwrap();
        let mut columns = Vec::new();
        expr.referenced_columns(&mut columns);
        assert_eq!(columns, ["width"]);
    }

    #[test]
    fn accepts_string_length_inside_numeric_expressions() {
        let expr = parse_scalar_expression("length(URL) + 1").unwrap();
        let mut columns = Vec::new();
        expr.referenced_columns(&mut columns);
        assert_eq!(columns, ["URL"]);
        assert!(matches!(expr, ScalarExpr::Add(..)));
        let repeated = parse_scalar_expression("length(URL) + URL").unwrap();
        let mut columns = Vec::new();
        repeated.referenced_columns(&mut columns);
        assert_eq!(columns, ["URL"]);
        assert!(parse_scalar_expression("length(URL, Other)").is_err());
    }

    #[test]
    fn multiplication_precedes_addition() {
        assert_eq!(
            parse_scalar_expression("width + 2 * 3").unwrap(),
            ScalarExpr::Add(
                Box::new(ScalarExpr::Column("width".into())),
                Box::new(ScalarExpr::Multiply(
                    Box::new(ScalarExpr::Integer(2)),
                    Box::new(ScalarExpr::Integer(3)),
                )),
            )
        );
        assert!(matches!(
            parse_scalar_expression("(width + 2) * 3").unwrap(),
            ScalarExpr::Multiply(..)
        ));
    }

    #[test]
    fn parses_unary_signs_on_columns_and_parentheses() {
        assert_eq!(
            parse_scalar_expression("-amount").unwrap(),
            ScalarExpr::Subtract(
                Box::new(ScalarExpr::Integer(0)),
                Box::new(ScalarExpr::Column("amount".into())),
            )
        );
        assert_eq!(
            parse_scalar_expression("+(amount * 2)").unwrap(),
            ScalarExpr::Multiply(
                Box::new(ScalarExpr::Column("amount".into())),
                Box::new(ScalarExpr::Integer(2)),
            )
        );
        assert!(parse_scalar_expression("-").is_err());
    }

    #[test]
    fn parses_division_with_multiplication_precedence() {
        assert_eq!(
            parse_scalar_expression("total / count + 1").unwrap(),
            ScalarExpr::Add(
                Box::new(ScalarExpr::Divide(
                    Box::new(ScalarExpr::Column("total".into())),
                    Box::new(ScalarExpr::Column("count".into())),
                )),
                Box::new(ScalarExpr::Integer(1)),
            )
        );
        assert!(parse_scalar_expression("total /").is_err());
    }

    #[test]
    fn comparison_precedence_and_nullable_column_references() {
        let expr = parse_scalar_expression("amount + 2 > threshold * 3").unwrap();
        let mut columns = Vec::new();
        expr.referenced_columns(&mut columns);
        assert_eq!(columns, ["amount", "threshold"]);
        assert!(matches!(expr, ScalarExpr::Compare(..)));
        assert!(parse_scalar_expression("amount >").is_err());
        assert!(parse_scalar_expression("amount > 1 > 2").is_err());
    }

    #[test]
    fn parses_null_checks_inside_computed_aggregate_conditions() {
        for (sql, negated) in [("amount IS NULL", false), ("amount IS NOT NULL", true)] {
            let value = parse_scalar_expression(sql).unwrap();
            assert!(matches!(value, ScalarExpr::IsNull(_, flag) if flag == negated));
        }
        assert!(parse_scalar_expression("amount IS EMPTY").is_err());
    }

    #[test]
    fn parses_not_after_comparisons_with_boolean_precedence() {
        assert!(matches!(parse_scalar_expression("NOT amount > 3").unwrap(),
            ScalarExpr::Not(value) if matches!(*value, ScalarExpr::Compare(..))));
        assert!(matches!(parse_scalar_expression("NOT (a = 1 OR b = 2)").unwrap(),
            ScalarExpr::Not(value) if matches!(*value, ScalarExpr::Or(..))));
        assert!(matches!(parse_scalar_expression("NOT NOT a IS NULL").unwrap(),
            ScalarExpr::IsNull(..)));
        assert!(parse_scalar_expression(&format!("{}a = 1", "NOT ".repeat(17))).is_err());
    }

    #[test]
    fn parses_between_and_not_between_with_boolean_precedence() {
        assert!(matches!(parse_scalar_expression("amount BETWEEN 2 AND 5").unwrap(),
            ScalarExpr::Between(_, _, _, false)));
        assert!(matches!(parse_scalar_expression("amount NOT BETWEEN 2 AND 5").unwrap(),
            ScalarExpr::Between(_, _, _, true)));
        assert!(matches!(parse_scalar_expression("NOT amount BETWEEN 2 AND 5").unwrap(),
            ScalarExpr::Not(..)));
        assert!(matches!(parse_scalar_expression("amount BETWEEN 1 AND 2 OR other = 1").unwrap(),
            ScalarExpr::Or(..)));
        assert!(parse_scalar_expression("amount BETWEEN 1").is_err());
    }

    #[test]
    fn parses_in_membership_with_boolean_precedence_and_budgets() {
        assert!(matches!(parse_scalar_expression("amount IN (1, 3, 5)").unwrap(),
            ScalarExpr::In(_, members, false) if members.len() == 3));
        assert!(matches!(parse_scalar_expression("amount NOT IN (1, 3)").unwrap(),
            ScalarExpr::In(_, _, true)));
        assert!(matches!(parse_scalar_expression("a IN (1, 2) AND b > 4").unwrap(),
            ScalarExpr::And(..)));
        assert!(parse_scalar_expression("a IN ()").is_err());
        assert!(parse_scalar_expression(&format!("a IN ({})",
            vec!["1"; 65].join(", "))).is_err());
    }

    #[test]
    fn explicit_null_literals_work_in_case_and_in_expressions() {
        assert_eq!(parse_scalar_expression("NULL").unwrap(), ScalarExpr::Null);
        assert!(matches!(parse_scalar_expression("a IN (NULL, 1)").unwrap(),
            ScalarExpr::In(_, members, false) if members[0] == ScalarExpr::Null));
        assert!(matches!(parse_scalar_expression("CASE WHEN a = 1 THEN NULL ELSE 2 END").unwrap(),
            ScalarExpr::CaseWhen(_, yes, _) if *yes == ScalarExpr::Null));
    }

    #[test]
    fn coalesce_expression_requires_two_to_sixteen_arguments() {
        assert!(matches!(parse_scalar_expression("coalesce(NULL, amount, 0)").unwrap(),
            ScalarExpr::Coalesce(values) if values.len() == 3));
        assert!(parse_scalar_expression("coalesce(amount)").is_err());
        assert!(parse_scalar_expression(&format!("coalesce({})",
            vec!["1"; 17].join(", "))).is_err());
    }

    #[test]
    fn nullif_is_a_nullable_numeric_expression() {
        assert!(matches!(parse_scalar_expression("nullIf(amount, 0)").unwrap(),
            ScalarExpr::NullIf(..)));
        assert!(parse_scalar_expression("nullIf(amount)").is_err());
        assert!(parse_scalar_expression("nullIf(amount, 0, 1)").is_err());
    }

    #[test]
    fn if_function_reuses_case_when_short_circuit_expression() {
        assert!(matches!(parse_scalar_expression("if(a > 0, a, 0)").unwrap(),
            ScalarExpr::CaseWhen(..)));
        for sql in ["if(a)", "if(a, 1)", "if(a, 1, 2, 3)", "if()"] {
            assert!(parse_scalar_expression(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn absolute_value_is_available_in_computed_aggregates() {
        assert!(matches!(parse_scalar_expression("abs(amount - 3)").unwrap(),
            ScalarExpr::Abs(..)));
        assert!(parse_scalar_expression("abs()").is_err());
        assert!(parse_scalar_expression("abs(a, b)").is_err());
    }

    #[test]
    fn parses_greatest_and_least_over_bounded_numeric_expressions() {
        assert!(matches!(parse_scalar_expression("greatest(a, b + 1, 0)").unwrap(),
            ScalarExpr::Extremum(values, true) if values.len() == 3));
        assert!(matches!(parse_scalar_expression("least(a, b)").unwrap(),
            ScalarExpr::Extremum(_, false)));
        assert!(parse_scalar_expression("greatest(a)").is_err());
        assert!(parse_scalar_expression(&format!("least({})",
            vec!["1"; 17].join(", "))).is_err());
    }

    #[test]
    fn modulo_has_multiplicative_precedence_in_aggregate_inputs() {
        assert!(matches!(parse_scalar_expression("a + b % 3").unwrap(),
            ScalarExpr::Add(_, right) if matches!(*right, ScalarExpr::Modulo(..))));
        assert!(matches!(parse_scalar_expression("a * b % 3").unwrap(),
            ScalarExpr::Modulo(..)));
        assert!(parse_scalar_expression("a %").is_err());
    }

    #[test]
    fn rejects_invalid_expressions() {
        for value in ["width +", "width,", "(width + 1", "9223372036854775808"] {
            assert!(parse_scalar_expression(value).is_err(), "{value}");
        }
    }
}
