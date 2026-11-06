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
            && matches!(token, Token::Word(word) if ["AND", "OR", "GROUP", "ORDER", "LIMIT", "OFFSET"].iter().any(|name| word.eq_ignore_ascii_case(name)))
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
mod tests {
    use crate::{execute, parse_select};

    fn run(sql: &str) -> String {
        let query = parse_select(sql).unwrap();
        let mut output = Vec::new();
        execute(&query, None, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn prefix_not_comparisons_share_the_parenthesized_inversion() {
        assert_eq!(
            run("SELECT number FROM numbers(5) WHERE NOT number > 3 ORDER BY number"),
            "number\n0\n1\n2\n3\n"
        );
    }

    #[test]
    fn nested_boolean_groups_respect_and_precedence() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(8) WHERE (number = 1 OR number = 2) AND number >= 2 ORDER BY number"
            ),
            "number\n2\n"
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(8) WHERE number = 1 OR number = 2 AND number > 2 ORDER BY number"
            ),
            "number\n1\n"
        );
    }

    #[test]
    fn integer_arithmetic_is_folded_in_in_lists() {
        assert_eq!(
            run("SELECT number FROM numbers(4) WHERE number IN (1, 1 + 1) ORDER BY number"),
            "number\n1\n2\n"
        );
    }

    #[test]
    fn integer_arithmetic_without_spaces_is_folded_in_in_lists() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(2002) WHERE number IN (1999,1999+1,1999+2) ORDER BY number"
            ),
            "number\n1999\n2000\n2001\n"
        );
    }

    #[test]
    fn integer_bound_stops_before_a_set_operator() {
        let mut parser = crate::Parser {
            tokens: crate::tokenize("1200 + 11 INTERSECT SELECT").unwrap(),
            index: 0,
        };
        assert_eq!(super::numeric_or_literal(&mut parser).unwrap(), "1211");
        assert!(matches!(
            parser.tokens.get(parser.index),
            Some(crate::Token::Word(word)) if word.eq_ignore_ascii_case("INTERSECT")
        ));
    }

    #[test]
    fn integer_arithmetic_is_folded_in_between_bounds() {
        assert_eq!(
            run("SELECT number FROM numbers(5) WHERE number BETWEEN 1 AND 1 + 2 ORDER BY number"),
            "number\n1\n2\n3\n"
        );
    }

    #[test]
    fn distributes_and_over_or_without_changing_between() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(8) WHERE number BETWEEN 1 AND 5 AND (number = 2 OR number = 4) ORDER BY number"
            ),
            "number\n2\n4\n"
        );
    }

    #[test]
    fn not_group_distributes_over_or_and_between_predicates() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(7) WHERE NOT (number = 1 OR number BETWEEN 3 AND 5) ORDER BY number"
            ),
            "number\n0\n2\n6\n",
        );
        assert_eq!(
            run(
                "SELECT number FROM numbers(7) WHERE NOT (number = 1 AND number < 2) ORDER BY number"
            ),
            "number\n0\n2\n3\n4\n5\n6\n",
        );
    }

    #[test]
    fn not_group_handles_nested_groups_and_modulo() {
        assert_eq!(
            run(
                "SELECT number FROM numbers(8) WHERE NOT (number % 2 = 0 OR (number > 5 AND number < 7)) ORDER BY number"
            ),
            "number\n1\n3\n5\n7\n",
        );
        assert!(parse_select("SELECT number FROM numbers(8) WHERE NOT (number = 1").is_err());
    }

    #[test]
    fn repeated_eight_atom_filters_simplify_before_branch_budget() {
        // Four independent binary disjunctions produce 16 DNF branches
        // from just eight source predicates. Repeating the group uses the
        // 16-condition parser budget but creates 256 temporary pairs.
        let clause = "(number > 0 OR number = 0) AND (number < 5 OR number > 8) AND (number % 2 = 0 OR number = 3) AND (number < 4 OR number > 1)";
        let base = format!("SELECT number FROM numbers(11) WHERE {clause} ORDER BY number");
        let repeated = format!(
            "SELECT number FROM numbers(11) WHERE ({clause}) AND ({clause}) ORDER BY number"
        );
        let query = crate::parse_select(&repeated).unwrap();
        assert_eq!(query.or_predicates.len(), 15);
        assert_eq!(run(&base), "number\n0\n2\n3\n4\n10\n");
        assert_eq!(run(&repeated), run(&base));
    }

    #[test]
    fn rejects_bad_brackets_and_exponential_expansion() {
        for sql in [
            "SELECT number FROM numbers(8) WHERE (number = 1",
            "SELECT number FROM numbers(8) WHERE number = 1)",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
        // Independent branches still trigger the bounded DNF guard.
        let groups = (0..7)
            .map(|index| format!("(number > {index} OR number < {index})"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let sql = format!("SELECT number FROM numbers(8) WHERE {groups}");
        assert!(parse_select(&sql).is_err());
    }
}

#[cfg(test)]
mod duplicate_boolean_terms_tests {
    use crate::parse_select;

    #[test]
    fn identical_or_branches_and_and_predicates_share_one_plan() {
        let sql = "SELECT number FROM numbers(10) WHERE             (number = 1 AND number = 1) OR (number = 1) OR             (number = 2 AND number >= 0 AND number = 2)";
        let query = parse_select(sql).unwrap();
        assert_eq!(query.predicates.len(), 1);
        assert_eq!(query.or_predicates.len(), 1);
        assert_eq!(query.or_predicates[0].len(), 2);
    }

    #[test]
    fn equivalent_conjunctions_deduplicate_regardless_of_order() {
        let query = parse_select(
            "SELECT number FROM numbers(10) WHERE              (number > 1 AND number < 7) OR (number < 7 AND number > 1)",
        ).unwrap();
        assert_eq!(query.predicates.len(), 2);
        assert!(query.or_predicates.is_empty());
    }
}

#[cfg(test)]
mod boolean_is_predicate_tests {
    use super::*;
    use crate::{execute, parse_select};

    fn run(sql: &str) -> String {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn boolean_is_forms_have_stable_numeric_truthiness() {
        assert_eq!(
            run("SELECT number FROM numbers(4) WHERE number IS TRUE ORDER BY number"),
            "number\n1\n2\n3\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(4) WHERE number IS FALSE"),
            "number\n0\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(4) WHERE number IS NOT TRUE"),
            "number\n0\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(4) WHERE number IS NOT FALSE ORDER BY number"),
            "number\n1\n2\n3\n"
        );
    }

    #[test]
    fn negated_is_predicates_preserve_sql_null_alternative() {
        let query = parse_select("SELECT number FROM numbers(3) WHERE number IS NOT TRUE").unwrap();
        assert_eq!(query.or_predicates.len(), 1);
        assert!(matches!(
            query.or_predicates[0].as_slice(),
            [Predicate::IsNull { negated: false, .. }]
        ));
        assert_eq!(
            run("SELECT number FROM numbers(3) WHERE number IS NOT UNKNOWN ORDER BY number"),
            "number\n0\n1\n2\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(3) WHERE number IS UNKNOWN"),
            "number\n"
        );
        assert!(parse_select("SELECT number FROM numbers(3) WHERE number IS MAYBE").is_err());
    }
}

#[cfg(test)]
mod boolean_absorption_tests {
    use super::*;
    use crate::{execute, parse_select};

    fn rows(where_clause: &str) -> Vec<u8> {
        let sql = format!("SELECT number FROM numbers(20) WHERE {where_clause} ORDER BY number");
        let query = parse_select(&sql).unwrap();
        let mut output = Vec::new();
        execute(&query, None, &mut output).unwrap();
        output
    }

    #[test]
    fn redundant_disjunctions_and_conjunctions_keep_exact_rows() {
        for base in ["number = 2", "number >= 3", "number % 2 = 0"] {
            assert_eq!(
                rows(base),
                rows(&format!("({base}) OR (({base}) AND number < 7)")),
            );
            assert_eq!(
                rows(base),
                rows(&format!("(({base}) AND number < 7) OR ({base})")),
            );
            assert_eq!(
                rows(&format!("NOT ({base})")),
                rows(&format!("NOT (({base}) OR (({base}) AND number < 7))")),
            );
        }
    }

    #[test]
    fn repeated_factors_no_longer_explode_the_dnf_limit() {
        let groups = (0..7)
            .map(|_| "(number = 1 OR number = 2)")
            .collect::<Vec<_>>()
            .join(" AND ");
        let sql = format!("SELECT number FROM numbers(8) WHERE {groups} ORDER BY number");
        let select = parse_select(&sql).unwrap();
        assert!(select.or_predicates.len() <= 1);
        let mut output = Vec::new();
        execute(&select, None, &mut output).unwrap();
        assert_eq!(output, b"number\n1\n2\n");
    }

    #[test]
    fn empty_and_distinct_branches_preserve_boolean_identities() {
        let eq_one = Predicate::Comparison {
            column: "number".into(),
            operator: Operator::Eq,
            value: "1".into(),
        };
        let eq_two = Predicate::Comparison {
            column: "number".into(),
            operator: Operator::Eq,
            value: "2".into(),
        };
        assert_eq!(
            unique_branches(vec![
                vec![eq_one.clone(), eq_two.clone()],
                vec![eq_one.clone()],
            ]),
            vec![vec![eq_one]]
        );
        assert_eq!(
            unique_branches(vec![vec![eq_two], vec![],]),
            vec![Vec::<Predicate>::new()]
        );
    }
}

#[cfg(test)]
mod streaming_dnf_absorption_tests {
    use super::*;

    fn equals(column: &str, number: &str) -> Predicate {
        Predicate::Comparison {
            column: column.into(),
            operator: Operator::Eq,
            value: number.into(),
        }
    }

    #[test]
    fn identical_large_in_lists_do_not_need_cartesian_literal_clones() {
        let values = (0..4096).map(|n| n.to_string()).collect::<Vec<_>>();
        let heavy = Predicate::In {
            column: "number".into(),
            values,
        };
        let lhs = vec![vec![heavy.clone()]; 64];
        let rhs = vec![vec![heavy]; 64];
        let product = combine_and(lhs, rhs).unwrap();
        assert_eq!(product.len(), 1);
        assert_eq!(product[0].len(), 1);
    }

    #[test]
    fn later_subsumption_removes_stronger_products_without_reordering() {
        let a = equals("a", "1");
        let b = equals("b", "2");
        let c = equals("c", "3");
        let d = equals("d", "4");
        let product = combine_and(
            vec![vec![a.clone(), b], vec![a.clone()]],
            vec![vec![c.clone()], vec![c.clone(), d]],
        )
        .unwrap();
        assert_eq!(product, vec![vec![a, c]]);
    }

    #[test]
    fn unrelated_cross_products_still_enforce_final_branch_limit() {
        let lhs = (0..9).map(|n| vec![equals("a", &n.to_string())]).collect();
        let rhs = (0..9).map(|n| vec![equals("b", &n.to_string())]).collect();
        assert!(combine_and(lhs, rhs).is_err());
    }
}

#[cfg(test)]
mod tpcds_large_dnf_tests {
    use crate::parse_select;

    fn arm(key: usize, prefix: &str) -> String {
        let categories = (0..4)
            .map(|category| {
                format!(
                    "category = '{prefix}{category}' AND (color = 'a' OR color = 'b') AND (units = 'x' OR units = 'y') AND (size = 'm' OR size = 'l')"
                )
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        format!("(key = {key} AND ({categories}))")
    }

    #[test]
    fn accepts_64_nested_branches_with_320_expanded_conditions() {
        let sql = format!(
            "SELECT count(*) FROM items WHERE {} OR {}",
            arm(1, "a"),
            arm(2, "b")
        );
        let select = parse_select(&sql).unwrap();
        assert_eq!(select.or_predicates.len() + 1, 64);
        assert_eq!(select.predicates.len(), 5);
        assert!(select.or_predicates.iter().all(|branch| branch.len() == 5));
    }
}

#[cfg(test)]
mod asymmetric_between_tests {
    use crate::parse_select;

    #[test]
    fn explicit_asymmetric_uses_the_default_between_semantics() {
        for modifier in ["", "NOT "] {
            let plain = parse_select(&format!(
                "SELECT * FROM numbers(10) WHERE number {modifier}BETWEEN 2 AND 5"
            ))
            .unwrap();
            let explicit = parse_select(&format!(
                "SELECT * FROM numbers(10) WHERE number {modifier}BETWEEN ASYMMETRIC 2 AND 5"
            ))
            .unwrap();
            assert_eq!(plain.predicates, explicit.predicates);
            assert_eq!(plain.or_predicates, explicit.or_predicates);
        }
        assert!(
            parse_select("SELECT * FROM numbers(10) WHERE number BETWEEN ASYMMETRIC 1").is_err()
        );
    }
}

#[cfg(test)]
mod tpch_where_subquery_diagnostic_10 {
    use crate::parse_select;

    #[test]
    fn unsupported_subquery_has_specific_diagnostic() {
        let err = parse_select("SELECT count(*) FROM partsupp WHERE ps_supplycost = (SELECT min(ps_supplycost) FROM partsupp)").unwrap_err();
        assert!(err.0.contains("scalar WHERE subqueries"), "{err}");
    }
}

#[cfg(test)]
mod tpch_where_subquery_diagnostic_9 {
    use crate::parse_select;

    #[test]
    fn unsupported_subquery_has_specific_diagnostic() {
        let err = parse_select(
            "SELECT count(*) FROM orders WHERE o_orderkey IN (SELECT l_orderkey FROM lineitem)",
        )
        .unwrap_err();
        assert!(err.0.contains("IN (SELECT ...) and NOT IN"), "{err}");
    }
}

#[cfg(test)]
mod tpch_where_subquery_diagnostic_8 {
    use crate::parse_select;

    #[test]
    fn unsupported_subquery_has_specific_diagnostic() {
        let err = parse_select("SELECT count(*) FROM orders WHERE EXISTS (SELECT * FROM lineitem WHERE l_orderkey = o_orderkey)").unwrap_err();
        assert!(err.0.contains("EXISTS and NOT EXISTS subqueries"), "{err}");
    }
}
