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
    Add(Box<ScalarExpr>, Box<ScalarExpr>),
    Subtract(Box<ScalarExpr>, Box<ScalarExpr>),
    Multiply(Box<ScalarExpr>, Box<ScalarExpr>),
    Divide(Box<ScalarExpr>, Box<ScalarExpr>),
    Compare(Box<ScalarExpr>, Operator, Box<ScalarExpr>),
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
            Self::Integer(_) => {}
            Self::Add(left, right)
            | Self::Subtract(left, right)
            | Self::Multiply(left, right)
            | Self::Divide(left, right)
            | Self::Compare(left, _, right) => {
                left.collect_referenced_columns(out, seen);
                right.collect_referenced_columns(out, seen);
            }
        }
    }
}

const MAX_EXPRESSION_DEPTH: usize = 64;

fn parse_arithmetic_tokens(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    let mut expr = product(parser, depth)?;
    loop {
        let addition = if parser.take(Token::Plus) {
            true
        } else if parser.take(Token::Minus) {
            false
        } else {
            break;
        };
        let right = product(parser, depth)?;
        expr = if addition {
            ScalarExpr::Add(Box::new(expr), Box::new(right))
        } else {
            ScalarExpr::Subtract(Box::new(expr), Box::new(right))
        };
    }
    Ok(expr)
}

/// Comparison has lower precedence than addition and multiplication.
pub(crate) fn parse_expression_tokens(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    parse_expression_tokens_at_depth(parser, 0)
}

fn parse_expression_tokens_at_depth(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    let left = parse_arithmetic_tokens(parser, depth)?;
    if let Some(Token::Operator(operator)) = parser.tokens.get(parser.index) {
        let operator = *operator;
        if !matches!(operator, Operator::Eq | Operator::Ne | Operator::Lt
            | Operator::Le | Operator::Gt | Operator::Ge) {
            return Err(ParseError("invalid operator in numeric expression".into()));
        }
        parser.index += 1;
        let right = parse_arithmetic_tokens(parser, depth)?;
        Ok(ScalarExpr::Compare(Box::new(left), operator, Box::new(right)))
    } else {
        Ok(left)
    }
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
    fn rejects_excessive_parentheses_and_unary_sign_nesting() {
        for expression in [
            format!("{}1{}", "(".repeat(1000), ")".repeat(1000)),
            format!("{}1", "-".repeat(1000)),
            format!("{}1", "+".repeat(1000)),
        ] {
            assert!(parse_scalar_expression(&expression).is_err());
        }
        assert!(parse_scalar_expression(&format!("{}1{}", "(".repeat(16), ")".repeat(16))).is_ok());
    }

    #[test]
    fn rejects_invalid_expressions() {
        for value in ["width +", "width,", "(width + 1", "9223372036854775808"] {
            assert!(parse_scalar_expression(value).is_err(), "{value}");
        }
    }
}
