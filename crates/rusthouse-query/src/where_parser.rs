//! Parse AND/OR predicate groups with SQL precedence, preserving a bounded DNF.
//! The executor consumes one conjunctive branch plus optional OR branches.

use crate::lexer::Token;
use crate::{Operator, ParseError, Parser, Predicate, ScalarExpr};

type Branches = Vec<Vec<Predicate>>;
// The branch and expanded-predicate budgets keep DNF work bounded, including
// the larger nested filter groups used by TPC-DS.
const MAX_CONDITIONS: usize = 64;
const MAX_BRANCHES: usize = 64;
const MAX_EXPANDED_CONDITIONS: usize = 512;
const MAX_DEPTH: usize = 16;
const MAX_IN_LIST_VALUES: usize = 4_096;

fn bad_expansion() -> ParseError {
    ParseError("WHERE boolean expression expands beyond bounded predicate limits".into())
}

fn integer_expression_value(expression: &ScalarExpr) -> Option<i64> {
    match expression {
        ScalarExpr::Integer(value) => Some(*value),
        ScalarExpr::Add(left, right) => {
            integer_expression_value(left)?.checked_add(integer_expression_value(right)?)
        }
        ScalarExpr::Subtract(left, right) => {
            integer_expression_value(left)?.checked_sub(integer_expression_value(right)?)
        }
        ScalarExpr::Multiply(left, right) => {
            integer_expression_value(left)?.checked_mul(integer_expression_value(right)?)
        }
        ScalarExpr::Divide(left, right) => {
            integer_expression_value(left)?.checked_div(integer_expression_value(right)?)
        }
        ScalarExpr::Modulo(left, right) => {
            integer_expression_value(left)?.checked_rem(integer_expression_value(right)?)
        }
        _ => None,
    }
}

fn numeric_or_literal(parser: &mut Parser) -> Result<String, ParseError> {
    let arithmetic_follows = matches!(parser.tokens.get(parser.index), Some(Token::Number(_)))
        && matches!(
            parser.tokens.get(parser.index + 1),
            Some(Token::Plus | Token::Minus | Token::Star | Token::Slash | Token::Percent)
        );
    if arithmetic_follows {
        // Expression parsing also understands boolean AND/OR. Bound this
        // constant expression at the current predicate delimiter so a later
        // WHERE conjunction is not consumed as part of a BETWEEN bound.
        let start = parser.index;
        let mut end = start;
        let mut depth = 0usize;
        while let Some(token) = parser.tokens.get(end) {
            if depth == 0
                && (matches!(token, Token::Comma | Token::RParen)
                    || matches!(token, Token::Word(word) if ["AND", "OR", "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET", "UNION", "INTERSECT", "EXCEPT"].iter().any(|keyword| word.eq_ignore_ascii_case(keyword))))
            {
                break;
            }
            match token {
                Token::LParen => depth += 1,
                Token::RParen => depth = depth.saturating_sub(1),
                _ => {}
            }
            end += 1;
        }
        let mut expression_parser = Parser {
            tokens: parser.tokens[start..end].to_vec(),
            index: 0,
        };
        let expression = crate::expression::parse_expression_tokens(&mut expression_parser)?;
        if expression_parser.index != expression_parser.tokens.len() {
            return Err(ParseError(
                "integer expression must be a bounded constant".into(),
            ));
        }
        parser.index = end;
        return integer_expression_value(&expression)
            .map(|value| value.to_string())
            .ok_or_else(|| ParseError("IN arithmetic must be a bounded integer constant".into()));
    }
    parser.literal()
}

/// Normalize structurally identical predicates before execution. SQL filters
/// have no side effects, so exact duplicate AND/OR terms never change rows.
fn branch_contains_all(superset: &[Predicate], subset: &[Predicate]) -> bool {
    subset.len() <= superset.len() && subset.iter().all(|predicate| superset.contains(predicate))
}

// Boolean absorption: A OR (A AND B) is A. All supported predicates are
// pure filters, so removing a stronger redundant branch preserves SQL NULL
// behavior as well as results. Retain the order of surviving branches.
fn unique_branches(mut branches: Branches) -> Branches {
    let mut distinct: Branches = Vec::with_capacity(branches.len());
    for mut branch in branches.drain(..) {
        let mut unique = Vec::with_capacity(branch.len());
        for predicate in branch.drain(..) {
            if !unique.contains(&predicate) {
                unique.push(predicate);
            }
        }
        // An existing less-restrictive branch already covers this one.
        if distinct
            .iter()
            .any(|existing| branch_contains_all(&unique, existing))
        {
            continue;
        }
        // This branch may subsume earlier stronger branches.
        distinct.retain(|existing| !branch_contains_all(existing, &unique));
        distinct.push(unique);
    }
    distinct
}

fn check_expansion(branches: &Branches) -> Result<(), ParseError> {
    if branches.len() > MAX_BRANCHES
        || branches.iter().any(|branch| branch.len() > MAX_CONDITIONS)
        || branches.iter().map(Vec::len).sum::<usize>() > MAX_EXPANDED_CONDITIONS
    {
        return Err(bad_expansion());
    }
    Ok(())
}

pub(super) fn combine_and(left: Branches, right: Branches) -> Result<Branches, ParseError> {
    // A temporary cross-product can be larger than the final DNF when
    // absorption removes duplicate and stronger branches. Keep the work
    // bounded, but enforce the public branch cap only after normalization.
    let combinations = left
        .len()
        .checked_mul(right.len())
        .filter(|&count| count <= MAX_BRANCHES * MAX_BRANCHES)
        .ok_or_else(bad_expansion)?;
    let mut result: Branches = Vec::with_capacity(combinations.min(MAX_BRANCHES));
    for lhs in &left {
        for rhs in &right {
            // Test absorption against borrowed terms before cloning literal
            // lists (IN may hold up to 4096 strings). Repeated OR factors
            // otherwise allocate the entire Cartesian product unnecessarily.
            if result.iter().any(|existing| {
                existing
                    .iter()
                    .all(|predicate| lhs.contains(predicate) || rhs.contains(predicate))
            }) {
                continue;
            }
            let mut conjuncts = Vec::with_capacity(lhs.len() + rhs.len());
            for predicate in lhs.iter().chain(rhs) {
                if !conjuncts.contains(predicate) {
                    conjuncts.push(predicate.clone());
                }
            }
            // A later less-restrictive conjunction can replace earlier
            // branches. Do not impose the final DNF cap before all inputs
            // have had the chance to be absorbed.
            result.retain(|existing| !branch_contains_all(existing, &conjuncts));
            result.push(conjuncts);
        }
    }
    check_expansion(&result)?;
    Ok(result)
}

fn inverse_comparison_operator(operator: &Operator) -> Operator {
    match operator {
        Operator::Eq => Operator::Ne,
        Operator::Ne => Operator::Eq,
        Operator::Lt => Operator::Ge,
        Operator::Le => Operator::Gt,
        Operator::Gt => Operator::Le,
        Operator::Ge => Operator::Lt,
        Operator::Modulo(n) => Operator::ModuloNe(*n),
        Operator::ModuloNe(n) => Operator::Modulo(*n),
    }
}

fn inverse(predicate: &Predicate) -> Branches {
    let opposite = match predicate {
        Predicate::ColumnComparison {
            column,
            operator,
            other,
        } => Predicate::ColumnComparison {
            column: column.clone(),
            operator: inverse_comparison_operator(operator),
            other: other.clone(),
        },
        Predicate::Comparison {
            column,
            operator,
            value,
        } => {
            let operator = inverse_comparison_operator(operator);
            Predicate::Comparison {
                column: column.clone(),
                operator,
                value: value.clone(),
            }
        }
        Predicate::In { column, values } => Predicate::NotIn {
            column: column.clone(),
            values: values.clone(),
        },
        Predicate::NotIn { column, values } => Predicate::In {
            column: column.clone(),
            values: values.clone(),
        },
        Predicate::SubstringIn {
            column,
            start,
            length,
            values,
            has_null,
            negated,
        } => Predicate::SubstringIn {
            column: column.clone(),
            start: *start,
            length: *length,
            values: values.clone(),
            has_null: *has_null,
            negated: !*negated,
        },
        Predicate::IsNull { column, negated } => Predicate::IsNull {
            column: column.clone(),
            negated: !*negated,
        },
        Predicate::DistinctFrom {
            column,
            value,
            negated,
        } => Predicate::DistinctFrom {
            column: column.clone(),
            value: value.clone(),
            negated: !*negated,
        },
        Predicate::Like {
            column,
            pattern,
            case_insensitive,
            negated,
        } => Predicate::Like {
            column: column.clone(),
            pattern: pattern.clone(),
            case_insensitive: *case_insensitive,
            negated: !*negated,
        },
        Predicate::Expression(ScalarExpr::Compare(left, operator, right)) => {
            Predicate::Expression(ScalarExpr::Compare(
                left.clone(),
                inverse_comparison_operator(operator),
                right.clone(),
            ))
        }
        Predicate::Expression(ScalarExpr::TextCompare {
            column,
            operator,
            value,
        }) => Predicate::Expression(ScalarExpr::TextCompare {
            column: column.clone(),
            operator: inverse_comparison_operator(operator),
            value: value.clone(),
        }),
        Predicate::Expression(ScalarExpr::TextLike {
            column,
            pattern,
            negated,
            case_insensitive,
        }) => Predicate::Expression(ScalarExpr::TextLike {
            column: column.clone(),
            pattern: pattern.clone(),
            negated: !negated,
            case_insensitive: *case_insensitive,
        }),
        Predicate::Expression(_) => return Vec::new(),
        Predicate::NotBetween {
            column,
            lower,
            upper,
        } => {
            return vec![vec![
                Predicate::Comparison {
                    column: column.clone(),
                    operator: Operator::Ge,
                    value: lower.clone(),
                },
                Predicate::Comparison {
                    column: column.clone(),
                    operator: Operator::Le,
                    value: upper.clone(),
                },
            ]];
        }
    };
    vec![vec![opposite]]
}

/// Apply De Morgan's laws to the existing bounded OR-of-AND representation.
/// No unbounded AST or exponential predicate expansion is allowed.
fn negate(branches: Branches) -> Result<Branches, ParseError> {
    let mut inverted = vec![vec![]];
    for conjunction in branches {
        let mut alternatives = Vec::new();
        for atom in conjunction {
            alternatives.extend(inverse(&atom));
        }
        if alternatives.is_empty() {
            // NOT(TRUE) is FALSE, which cannot match any row.
            return Ok(Vec::new());
        }
        inverted = combine_and(inverted, alternatives)?;
    }
    check_expansion(&inverted)?;
    Ok(inverted)
}

fn check_parenthesis_depth(depth: usize) -> Result<(), ParseError> {
    if depth >= MAX_DEPTH {
        return Err(ParseError("WHERE has too many nested parentheses".into()));
    }
    Ok(())
}

fn has_arithmetic_operator(tokens: &[Token]) -> bool {
    tokens.iter().enumerate().any(|(index, token)| match token {
        Token::Star | Token::Slash | Token::Percent => true,
        Token::Plus | Token::Minus => index > 0,
        _ => false,
    })
}

fn expression_between_bounds(
    parser: &mut Parser,
) -> Result<Option<(ScalarExpr, ScalarExpr)>, ParseError> {
    fn boundary(tokens: &[Token], start: usize, between_delimiter: bool) -> Option<usize> {
        let mut depth = 0usize;
        let mut case_depth = 0usize;
        for (index, token) in tokens.iter().enumerate().skip(start) {
            match token {
                Token::LParen => depth += 1,
                Token::RParen if depth == 0 => return Some(index),
                Token::RParen => depth -= 1,
                Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("CASE") => {
                    case_depth += 1;
                }
                Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("END") => {
                    case_depth = case_depth.saturating_sub(1);
                }
                Token::Word(word)
                    if depth == 0
                        && case_depth == 0
                        && (word.eq_ignore_ascii_case("AND")
                            || (!between_delimiter
                                && [
                                    "OR", "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET", "FORMAT",
                                ]
                                .iter()
                                .any(|keyword| word.eq_ignore_ascii_case(keyword)))) =>
                {
                    return Some(index);
                }
                Token::Semicolon if depth == 0 && case_depth == 0 => return Some(index),
                _ => {}
            }
        }
        None
    }

    fn parse_bound(tokens: &[Token]) -> Result<ScalarExpr, ParseError> {
        let mut parser = Parser {
            tokens: tokens.to_vec(),
            index: 0,
        };
        let expression = crate::expression::parse_expression_tokens(&mut parser)?;
        if parser.index != parser.tokens.len() {
            return Err(ParseError("invalid expression in BETWEEN bound".into()));
        }
        Ok(expression)
    }

    let start = parser.index;
    let Some(and) = boundary(&parser.tokens, start, true) else {
        return Ok(None);
    };
    let upper_start = and + 1;
    let upper_end = boundary(&parser.tokens, upper_start, false).unwrap_or(parser.tokens.len());
    let lower_tokens = &parser.tokens[start..and];
    let upper_tokens = &parser.tokens[upper_start..upper_end];
    if !has_arithmetic_operator(lower_tokens) && !has_arithmetic_operator(upper_tokens) {
        return Ok(None);
    }
    let (Ok(lower), Ok(upper)) = (parse_bound(lower_tokens), parse_bound(upper_tokens)) else {
        return Ok(None);
    };
    let mut referenced_columns = Vec::new();
    lower.referenced_columns(&mut referenced_columns);
    upper.referenced_columns(&mut referenced_columns);
    if referenced_columns.is_empty() {
        return Ok(None);
    }
    parser.index = upper_end;
    Ok(Some((lower, upper)))
}

fn parse_atom(
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
        let expression_bounds = if allow_expression {
            expression_between_bounds(parser)?
        } else {
            None
        };
        if let Some((lower, upper)) = expression_bounds {
            let value = ScalarExpr::Column(column);
            let between = vec![
                Predicate::Expression(ScalarExpr::Compare(
                    Box::new(value.clone()),
                    Operator::Ge,
                    Box::new(lower),
                )),
                Predicate::Expression(ScalarExpr::Compare(
                    Box::new(value),
                    Operator::Le,
                    Box::new(upper),
                )),
            ];
            let branches = if negate_in {
                negate(vec![between])?
            } else {
                vec![between]
            };
            *conditions = conditions
                .checked_add(branches.iter().map(Vec::len).sum::<usize>())
                .ok_or_else(bad_expansion)?;
            if *conditions > MAX_CONDITIONS {
                return Err(ParseError("WHERE supports at most 16 conditions".into()));
            }
            return Ok(branches);
        }
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

fn parse_and(
    parser: &mut Parser,
    conditions: &mut usize,
    depth: usize,
    allow_expression: bool,
) -> Result<Branches, ParseError> {
    let mut branches = parse_atom(parser, conditions, depth, allow_expression)?;
    while parser.keyword("AND") {
        branches = combine_and(
            branches,
            parse_atom(parser, conditions, depth, allow_expression)?,
        )?;
    }
    Ok(branches)
}

fn parse_or(
    parser: &mut Parser,
    conditions: &mut usize,
    depth: usize,
    allow_expression: bool,
) -> Result<Branches, ParseError> {
    let mut branches = parse_and(parser, conditions, depth, allow_expression)?;
    while parser.keyword("OR") {
        let next = parse_and(parser, conditions, depth, allow_expression)?;
        // Both sides were individually bounded. Let equivalent or absorbed
        // branches collapse before rejecting the 64-branch final limit.
        branches.extend(next);
        branches = unique_branches(branches);
        check_expansion(&branches)?;
    }
    Ok(branches)
}

// A NUL cannot occur in a SQL identifier. Encode qualified join references
// without changing the single-table predicate representation.
pub(crate) fn parse_join_where(parser: &mut Parser) -> Result<Branches, ParseError> {
    let mut tokens = Vec::new();
    let mut positions = vec![parser.index];
    let mut index = parser.index;
    while index < parser.tokens.len() {
        fn name(token: &Token) -> Option<&str> {
            match token {
                Token::Word(name) | Token::QuotedWord(name) => Some(name),
                _ => None,
            }
        }
        if let (Some(left), Some(Token::Dot), Some(right)) = (
            parser.tokens.get(index).and_then(name),
            parser.tokens.get(index + 1),
            parser.tokens.get(index + 2).and_then(name),
        ) {
            tokens.push(Token::QuotedWord(format!("{left}\0{right}")));
            index += 3;
        } else {
            tokens.push(parser.tokens[index].clone());
            index += 1;
        }
        positions.push(index);
    }
    let mut local = Parser { tokens, index: 0 };
    let branches = parse_where_with_expression(&mut local, true)?;
    parser.index = positions[local.index];
    Ok(branches)
}

/// Parse a WHERE clause already past the WHERE keyword.
pub(crate) fn parse_where(parser: &mut Parser) -> Result<Branches, ParseError> {
    parse_where_with_expression(parser, false)
}

fn parse_where_with_expression(
    parser: &mut Parser,
    allow_expression: bool,
) -> Result<Branches, ParseError> {
    let mut conditions = 0;
    Ok(unique_branches(parse_or(
        parser,
        &mut conditions,
        0,
        allow_expression,
    )?))
}

#[cfg(test)]
#[path = "where_parser_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "where_parser_tests/duplicate_boolean_terms_tests.rs"]
mod duplicate_boolean_terms_tests;

#[cfg(test)]
#[path = "where_parser_tests/boolean_is_predicate_tests.rs"]
mod boolean_is_predicate_tests;

#[cfg(test)]
#[path = "where_parser_tests/boolean_absorption_tests.rs"]
mod boolean_absorption_tests;

#[cfg(test)]
#[path = "where_parser_tests/streaming_dnf_absorption_tests.rs"]
mod streaming_dnf_absorption_tests;

#[cfg(test)]
#[path = "where_parser_tests/tpcds_large_dnf_tests.rs"]
mod tpcds_large_dnf_tests;

#[cfg(test)]
#[path = "where_parser_tests/asymmetric_between_tests.rs"]
mod asymmetric_between_tests;

#[cfg(test)]
#[path = "where_parser_tests/tpch_where_subquery_diagnostic_10.rs"]
mod tpch_where_subquery_diagnostic_10;

#[cfg(test)]
#[path = "where_parser_tests/tpch_where_subquery_diagnostic_9.rs"]
mod tpch_where_subquery_diagnostic_9;

#[cfg(test)]
#[path = "where_parser_tests/tpch_where_subquery_diagnostic_8.rs"]
mod tpch_where_subquery_diagnostic_8;
