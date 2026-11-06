//! Separate predicate atom parsing from bounded AND/OR expression expansion.

use super::*;

pub(super) fn parse_atom(
    parser: &mut Parser,
    conditions: &mut usize,
    depth: usize,
    allow_expression: bool,
) -> Result<Branches, ParseError> {
    let expression_starts_here = matches!(
        parser.tokens.get(parser.index),
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("CASE")
    ) || (parser.tokens.get(parser.index) == Some(&Token::LParen)
        && matches!(parser.tokens.get(parser.index + 1), Some(Token::Word(word)) if word.eq_ignore_ascii_case("CASE")))
        || (matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if ["COALESCE", "ROUND"].iter().any(|name| word.eq_ignore_ascii_case(name)))
            && parser.tokens.get(parser.index + 1) == Some(&Token::LParen));
    let mut expression_index = parser.index;
    let mut expression_depth = 0usize;
    let mut arithmetic_starts_here = false;
    let mut case_starts_here = false;
    while let Some(token) = parser.tokens.get(expression_index) {
        if expression_depth == 0
            && matches!(token, Token::Word(word) if ["AND", "OR", "BETWEEN", "GROUP", "ORDER", "LIMIT", "OFFSET"].iter().any(|name| word.eq_ignore_ascii_case(name)))
        {
            break;
        }
        match token {
            Token::LParen => expression_depth += 1,
            Token::RParen if expression_depth == 0 => break,
            Token::RParen => expression_depth -= 1,
            Token::Word(word) if word.eq_ignore_ascii_case("CASE") => {
                case_starts_here = true;
            }
            Token::Plus | Token::Minus | Token::Star | Token::Slash | Token::Percent
                if expression_depth == 0 =>
            {
                arithmetic_starts_here = true;
            }
            _ => {}
        }
        expression_index += 1;
    }
    if allow_expression && (expression_starts_here || case_starts_here || arithmetic_starts_here) {
        let expression = crate::expression::parse_comparison_tokens(parser)?;
        if !matches!(
            expression,
            crate::ScalarExpr::Compare(..)
                | crate::ScalarExpr::TextCompare { .. }
                | crate::ScalarExpr::TextLike { .. }
        ) {
            return Err(ParseError(
                "JOIN CASE filter requires a comparison expression".into(),
            ));
        }
        *conditions = conditions.checked_add(1).ok_or_else(bad_expansion)?;
        if *conditions > MAX_CONDITIONS {
            return Err(ParseError("WHERE supports at most 16 conditions".into()));
        }
        return Ok(vec![vec![Predicate::Expression(expression)]]);
    }
    // Parenthesized negations are different from NOT applied to one predicate:
    // invert every AND/OR branch using De Morgan's laws before execution.
    if parser.keyword("NOT") {
        if parser.take(Token::LParen) {
            check_parenthesis_depth(depth)?;
            let expression = parse_or(parser, conditions, depth + 1, allow_expression)?;
            if !parser.take(Token::RParen) {
                return Err(ParseError("NOT group requires closing ')'".into()));
            }
            return negate(expression);
        }
        // Reuse existing NOT IN, NOT BETWEEN, NOT LIKE and comparison parsing.
        parser.index -= 1;
    }
    if parser.take(Token::LParen) {
        check_parenthesis_depth(depth)?;
        let branches = parse_or(parser, conditions, depth + 1, allow_expression)?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("WHERE requires closing ')'".into()));
        }
        return Ok(branches);
    }
    let mut predicates = Vec::new();
    let prefix_not = parser.keyword("NOT");
    if parser.keyword("EXISTS") {
        return Err(ParseError(
            "EXISTS and NOT EXISTS subqueries are not yet supported".into(),
        ));
    }
    if parser.keyword("SUBSTRING") || parser.keyword("SUBSTR") {
        if !parser.take(Token::LParen) {
            return Err(ParseError("SUBSTRING requires '('".into()));
        }
        let column = parser.word()?;
        if !parser.take(Token::Comma) {
            return Err(ParseError(
                "SUBSTRING requires comma-separated bounds".into(),
            ));
        }
        let start = parser
            .number()?
            .parse::<usize>()
            .map_err(|_| ParseError("SUBSTRING start must be a positive integer".into()))?;
        if !parser.take(Token::Comma) {
            return Err(ParseError("SUBSTRING requires a length".into()));
        }
        let length = parser
            .number()?
            .parse::<usize>()
            .map_err(|_| ParseError("SUBSTRING length must be a positive integer".into()))?;
        if !(1..=64).contains(&start) || !(1..=64).contains(&length) || !parser.take(Token::RParen)
        {
            return Err(ParseError(
                "SUBSTRING bounds must be in 1..64 with closing ')'".into(),
            ));
        }
        let postfix_not = parser.keyword("NOT");
        if !parser.keyword("IN") || !parser.take(Token::LParen) {
            return Err(ParseError("SUBSTRING filter requires IN or NOT IN".into()));
        }
        let mut values = Vec::new();
        let mut has_null = false;
        let mut item_count = 0usize;
        loop {
            if item_count == MAX_IN_LIST_VALUES {
                return Err(ParseError("IN supports at most 4096 values".into()));
            }
            item_count += 1;
            if parser.keyword("NULL") {
                has_null = true;
            } else {
                values.push(parser.literal()?);
            }
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after IN list".into()));
        }
        predicates.push(Predicate::SubstringIn {
            column,
            start,
            length,
            values,
            has_null,
            negated: prefix_not ^ postfix_not,
        });
        *conditions = conditions.checked_add(1).ok_or_else(bad_expansion)?;
        if *conditions > MAX_CONDITIONS {
            return Err(ParseError("WHERE supports at most 16 conditions".into()));
        }
        return Ok(vec![predicates]);
    }
    let column = parser.word()?;
    let postfix_not = parser.keyword("NOT");
    let negate_in = prefix_not ^ postfix_not;
    if parser.keyword("IS") {
        if postfix_not {
            return Err(ParseError("unexpected NOT before IS".into()));
        }
        let negated = parser.keyword("NOT") ^ prefix_not;
        let predicate = if parser.keyword("DISTINCT") {
            if !parser.keyword("FROM") {
                return Err(ParseError("IS DISTINCT requires FROM".into()));
            }
            if parser.keyword("NULL") {
                Predicate::IsNull {
                    column,
                    negated: !negated,
                }
            } else {
                Predicate::DistinctFrom {
                    column,
                    value: parser.literal()?,
                    negated,
                }
            }
        } else if parser.keyword("NULL") || parser.keyword("UNKNOWN") {
            Predicate::IsNull { column, negated }
        } else {
            let expected_true = if parser.keyword("TRUE") {
                true
            } else if parser.keyword("FALSE") {
                false
            } else {
                return Err(ParseError(
                    "expected TRUE, FALSE, NULL, UNKNOWN or DISTINCT FROM after IS".into(),
                ));
            };
            // IS NOT TRUE and IS NOT FALSE include NULL, unlike ordinary
            // comparisons. Express them in the common DNF predicate model.
            let operator = if expected_true ^ negated {
                Operator::Ne
            } else {
                Operator::Eq
            };
            let compare = Predicate::Comparison {
                column: column.clone(),
                operator,
                value: "0".into(),
            };
            if negated {
                *conditions = conditions.checked_add(2).ok_or_else(bad_expansion)?;
                if *conditions > MAX_CONDITIONS {
                    return Err(bad_expansion());
                }
                return Ok(vec![
                    vec![compare],
                    vec![Predicate::IsNull {
                        column,
                        negated: false,
                    }],
                ]);
            }
            // IS TRUE/FALSE are two-valued. Retaining the non-NULL guard
            // makes NOT (column IS TRUE/FALSE) include NULL after De Morgan.
            predicates.push(Predicate::IsNull {
                column,
                negated: true,
            });
            compare
        };
        predicates.push(predicate);
    } else if let Some(case_insensitive) = parser.like_keyword() {
        let pattern = match parser.tokens.get(parser.index) {
            Some(Token::String(value)) => value.clone(),
            _ => return Err(ParseError("LIKE requires a string literal".into())),
        };
        let trailing_escapes = pattern
            .as_bytes()
            .iter()
            .rev()
            .take_while(|&&byte| byte == b'\\')
            .count();
        if trailing_escapes % 2 != 0 {
            return Err(ParseError("LIKE pattern has a trailing escape".into()));
        }
        parser.index += 1;
        predicates.push(Predicate::Like {
            column,
            pattern,
            case_insensitive,
            negated: negate_in,
        });
    } else if parser.keyword("BETWEEN") {
        // ASYMMETRIC is the SQL default and uses the existing ordered bounds.
        parser.keyword("ASYMMETRIC");
        let lower = numeric_or_literal(parser)?;
        if !parser.keyword("AND") {
            return Err(ParseError("BETWEEN requires AND".into()));
        }
        let upper = numeric_or_literal(parser)?;
        if negate_in {
            predicates.push(Predicate::NotBetween {
                column,
                lower,
                upper,
            });
        } else {
            predicates.push(Predicate::Comparison {
                column: column.clone(),
                operator: Operator::Ge,
                value: lower,
            });
            predicates.push(Predicate::Comparison {
                column,
                operator: Operator::Le,
                value: upper,
            });
        }
    } else if parser.keyword("IN") {
        if !parser.take(Token::LParen) {
            return Err(ParseError("expected '(' after IN".into()));
        }
        if matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT"))
        {
            return Err(ParseError(
                "IN (SELECT ...) and NOT IN (SELECT ...) are not yet supported".into(),
            ));
        }
        let mut values = Vec::new();
        loop {
            if values.len() == MAX_IN_LIST_VALUES {
                return Err(ParseError("IN supports at most 4096 values".into()));
            }
            values.push(numeric_or_literal(parser)?);
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after IN list".into()));
        }
        predicates.push(if negate_in {
            Predicate::NotIn { column, values }
        } else {
            Predicate::In { column, values }
        });
    } else {
        if postfix_not {
            return Err(ParseError(
                "NOT after a column requires IN or BETWEEN".into(),
            ));
        }
        let mut operator = if parser.take(Token::Percent) {
            let modulus = parser.number()?;
            let modulus = modulus
                .parse::<u64>()
                .map_err(|_| ParseError("modulus must be a positive UInt64".into()))?;
            if modulus == 0 {
                return Err(ParseError("modulus must be positive".into()));
            }
            match parser.operator()? {
                Operator::Eq => Operator::Modulo(modulus),
                Operator::Ne => Operator::ModuloNe(modulus),
                _ => return Err(ParseError("modulo predicates require = or !=".into())),
            }
        } else {
            parser.operator()?
        };
        if prefix_not {
            if matches!(operator, Operator::Modulo(_) | Operator::ModuloNe(_)) {
                return Err(ParseError("NOT modulo predicates are unsupported".into()));
            }
            operator = inverse_comparison_operator(&operator);
        }
        if parser.tokens.get(parser.index) == Some(&Token::LParen)
            && matches!(parser.tokens.get(parser.index + 1), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT"))
        {
            return Err(ParseError(
                "scalar WHERE subqueries are not yet supported".into(),
            ));
        }
        // A bare identifier after a relational operator is another column,
        // not a string literal. CAST and typed DATE/TIMESTAMP literals retain
        // their existing parser paths.
        let other = match parser.tokens.get(parser.index) {
            Some(Token::Word(word))
                if !matches!(
                    word.to_ascii_uppercase().as_str(),
                    "CAST" | "DATE" | "TIMESTAMP" | "NULL" | "TRUE" | "FALSE" | "INTERVAL"
                ) =>
            {
                Some(parser.word()?)
            }
            Some(Token::QuotedWord(_)) => Some(parser.word()?),
            _ => None,
        };
        predicates.push(if let Some(other) = other {
            if matches!(operator, Operator::Modulo(_) | Operator::ModuloNe(_)) {
                return Err(ParseError(
                    "column comparisons do not support modulo".into(),
                ));
            }
            Predicate::ColumnComparison {
                column,
                operator,
                other,
            }
        } else {
            Predicate::Comparison {
                column,
                operator,
                value: parser.literal()?,
            }
        });
    }
    *conditions = conditions
        .checked_add(predicates.len())
        .ok_or_else(bad_expansion)?;
    if *conditions > MAX_CONDITIONS {
        return Err(ParseError("WHERE supports at most 16 conditions".into()));
    }
    Ok(vec![predicates])
}
