use super::*;

/// Reject extremely long left-associative chains before execution or planning
/// recursively visits the AST. Parentheses depth alone does not bound tree depth.
pub(super) fn check_expression_budget(root: &ScalarExpr) -> Result<(), ParseError> {
    let mut pending = vec![(root, 0usize)];
    let mut total = 0usize;
    while let Some((expr, depth)) = pending.pop() {
        total += 1;
        if depth >= 256 || total > 4096 {
            return Err(ParseError(
                "numeric expression exceeds complexity budget".into(),
            ));
        }
        match expr {
            ScalarExpr::Add(left, right)
            | ScalarExpr::Subtract(left, right)
            | ScalarExpr::Multiply(left, right)
            | ScalarExpr::Divide(left, right)
            | ScalarExpr::Modulo(left, right)
            | ScalarExpr::Compare(left, _, right)
            | ScalarExpr::And(left, right)
            | ScalarExpr::Or(left, right) => {
                pending.push((right, depth + 1));
                pending.push((left, depth + 1));
            }
            ScalarExpr::Case {
                branches,
                else_expr,
            } => {
                if let Some(value) = else_expr {
                    pending.push((value, depth + 1));
                }
                for (condition, value) in branches {
                    pending.push((condition, depth + 1));
                    pending.push((value, depth + 1));
                }
            }
            ScalarExpr::Column(_)
            | ScalarExpr::TextCompare { .. }
            | ScalarExpr::TextLike { .. }
            | ScalarExpr::IsNull { .. }
            | ScalarExpr::TextLength(_)
            | ScalarExpr::TextLiteral(_)
            | ScalarExpr::Integer(_)
            | ScalarExpr::Decimal(_) => {}
            ScalarExpr::TextConcat(arguments) => {
                for argument in arguments {
                    pending.push((argument, depth + 1));
                }
            }
            ScalarExpr::TextSubstring { value, .. } | ScalarExpr::Round { value, .. } => {
                pending.push((value, depth + 1));
            }
        }
    }
    Ok(())
}

pub(super) fn parse_comparison_tokens_at_depth(
    parser: &mut Parser,
    depth: usize,
) -> Result<ScalarExpr, ParseError> {
    let left = parse_arithmetic_tokens(parser, depth)?;
    let negated = parser.keyword("NOT");
    let like = parser.keyword("LIKE");
    let ilike = !like && parser.keyword("ILIKE");
    if like || ilike {
        let ScalarExpr::Column(column) = left else {
            return Err(ParseError(
                "LIKE in scalar expressions requires a String column".into(),
            ));
        };
        let Some(Token::String(pattern)) = parser.tokens.get(parser.index) else {
            return Err(ParseError(
                "LIKE in scalar expressions requires a string literal".into(),
            ));
        };
        let pattern = pattern.clone();
        parser.index += 1;
        return Ok(ScalarExpr::TextLike {
            column,
            pattern,
            negated,
            case_insensitive: ilike,
        });
    }
    if negated {
        return Err(ParseError("expected LIKE or ILIKE after NOT".into()));
    }
    if parser.keyword("IS") {
        let negated = parser.keyword("NOT");
        if !parser.keyword("NULL") {
            return Err(ParseError("IS expression requires NULL".into()));
        }
        let ScalarExpr::Column(column) = left else {
            return Err(ParseError("IS NULL requires a column".into()));
        };
        return Ok(ScalarExpr::IsNull { column, negated });
    }
    if let Some(Token::Operator(operator)) = parser.tokens.get(parser.index) {
        let operator = *operator;
        if !matches!(
            operator,
            Operator::Eq | Operator::Ne | Operator::Lt | Operator::Le | Operator::Gt | Operator::Ge
        ) {
            return Err(ParseError("invalid operator in numeric expression".into()));
        }
        parser.index += 1;
        if let Some(Token::String(value)) = parser.tokens.get(parser.index) {
            let ScalarExpr::Column(column) = left else {
                return Err(ParseError("text comparison requires a column".into()));
            };
            let value = value.clone();
            parser.index += 1;
            return Ok(ScalarExpr::TextCompare {
                column,
                operator,
                value,
            });
        }
        let right = parse_arithmetic_tokens(parser, depth)?;
        Ok(ScalarExpr::Compare(
            Box::new(left),
            operator,
            Box::new(right),
        ))
    } else {
        Ok(left)
    }
}

pub(super) fn parse_and_tokens_at_depth(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    let mut result = parse_comparison_tokens_at_depth(parser, depth)?;
    let mut count = 0;
    while parser.keyword("AND") {
        count += 1;
        if count > 128 {
            return Err(ParseError(
                "AND expression exceeds complexity budget".into(),
            ));
        }
        let right = parse_comparison_tokens_at_depth(parser, depth)?;
        result = ScalarExpr::And(Box::new(result), Box::new(right));
    }
    Ok(result)
}

pub(super) fn parse_expression_tokens_at_depth(
    parser: &mut Parser,
    depth: usize,
) -> Result<ScalarExpr, ParseError> {
    let mut result = parse_and_tokens_at_depth(parser, depth)?;
    let mut count = 0;
    while parser.keyword("OR") {
        count += 1;
        if count > 128 {
            return Err(ParseError("OR expression exceeds complexity budget".into()));
        }
        let right = parse_and_tokens_at_depth(parser, depth)?;
        result = ScalarExpr::Or(Box::new(result), Box::new(right));
    }
    Ok(result)
}

