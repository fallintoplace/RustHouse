//! Parse AND/OR predicate groups with SQL precedence, preserving a bounded DNF.
//! The executor consumes one conjunctive branch plus optional OR branches.

use crate::{Operator, ParseError, Parser, Predicate};
use crate::lexer::Token;

type Branches = Vec<Vec<Predicate>>;
const MAX_CONDITIONS: usize = 16;
const MAX_BRANCHES: usize = 64;
const MAX_EXPANDED_CONDITIONS: usize = 256;
const MAX_DEPTH: usize = 16;

fn bad_expansion() -> ParseError {
    ParseError("WHERE boolean expression expands beyond bounded predicate limits".into())
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

fn combine_and(left: Branches, right: Branches) -> Result<Branches, ParseError> {
    if left.len().checked_mul(right.len()).is_none_or(|n| n > MAX_BRANCHES) {
        return Err(bad_expansion());
    }
    let mut result = Vec::with_capacity(left.len() * right.len());
    for lhs in &left {
        for rhs in &right {
            let mut conjuncts = Vec::with_capacity(lhs.len() + rhs.len());
            conjuncts.extend(lhs.iter().cloned());
            conjuncts.extend(rhs.iter().cloned());
            result.push(conjuncts);
        }
    }
    check_expansion(&result)?;
    Ok(result)
}

fn parse_atom(parser: &mut Parser, conditions: &mut usize, depth: usize) -> Result<Branches, ParseError> {
    if parser.take(Token::LParen) {
        if depth >= MAX_DEPTH {
            return Err(ParseError("WHERE has too many nested parentheses".into()));
        }
        let branches = parse_or(parser, conditions, depth + 1)?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("WHERE requires closing ')'".into()));
        }
        return Ok(branches);
    }
    let mut predicates = Vec::new();
    let prefix_not = parser.keyword("NOT");
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
                Predicate::IsNull { column, negated: !negated }
            } else {
                Predicate::DistinctFrom {
                    column, value: parser.literal()?, negated,
                }
            }
        } else if parser.keyword("NULL") {
            Predicate::IsNull { column, negated }
        } else {
            return Err(ParseError("expected NULL or DISTINCT FROM after IS".into()));
        };
        predicates.push(predicate);
    } else if let Some(case_insensitive) = parser.like_keyword() {
        let pattern = match parser.tokens.get(parser.index) {
            Some(Token::String(value)) => value.clone(),
            _ => return Err(ParseError("LIKE requires a string literal".into())),
        };
        let trailing_escapes = pattern.as_bytes().iter().rev()
            .take_while(|&&byte| byte == b'\\').count();
        if trailing_escapes % 2 != 0 {
            return Err(ParseError("LIKE pattern has a trailing escape".into()));
        }
        parser.index += 1;
        predicates.push(Predicate::Like {
            column, pattern, case_insensitive, negated: negate_in,
        });
    } else if parser.keyword("BETWEEN") {
        let lower = parser.literal()?;
        if !parser.keyword("AND") {
            return Err(ParseError("BETWEEN requires AND".into()));
        }
        let upper = parser.literal()?;
        if negate_in {
            predicates.push(Predicate::NotBetween { column, lower, upper });
        } else {
            predicates.push(Predicate::Comparison {
                column: column.clone(), operator: Operator::Ge, value: lower,
            });
            predicates.push(Predicate::Comparison {
                column, operator: Operator::Le, value: upper,
            });
        }
    } else if parser.keyword("IN") {
        if !parser.take(Token::LParen) {
            return Err(ParseError("expected '(' after IN".into()));
        }
        let mut values = Vec::new();
        loop {
            if values.len() == 32 {
                return Err(ParseError("IN supports at most 32 values".into()));
            }
            values.push(parser.literal()?);
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
            return Err(ParseError("NOT after a column requires IN or BETWEEN".into()));
        }
        let mut operator = if parser.take(Token::Percent) {
            let modulus = parser.number()?;
            let modulus = modulus.parse::<u64>()
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
            operator = match operator {
                Operator::Eq => Operator::Ne,
                Operator::Ne => Operator::Eq,
                Operator::Lt => Operator::Ge,
                Operator::Le => Operator::Gt,
                Operator::Gt => Operator::Le,
                Operator::Ge => Operator::Lt,
                Operator::Modulo(_) | Operator::ModuloNe(_) =>
                    return Err(ParseError("NOT modulo predicates are unsupported".into())),
            };
        }
        predicates.push(Predicate::Comparison {
            column, operator, value: parser.literal()?,
        });
    }
    *conditions = conditions.checked_add(predicates.len()).ok_or_else(bad_expansion)?;
    if *conditions > MAX_CONDITIONS {
        return Err(ParseError("WHERE supports at most 16 conditions".into()));
    }
    Ok(vec![predicates])
}

fn parse_and(parser: &mut Parser, conditions: &mut usize, depth: usize) -> Result<Branches, ParseError> {
    let mut branches = parse_atom(parser, conditions, depth)?;
    while parser.keyword("AND") {
        branches = combine_and(branches, parse_atom(parser, conditions, depth)?)?;
    }
    Ok(branches)
}

fn parse_or(parser: &mut Parser, conditions: &mut usize, depth: usize) -> Result<Branches, ParseError> {
    let mut branches = parse_and(parser, conditions, depth)?;
    while parser.keyword("OR") {
        let next = parse_and(parser, conditions, depth)?;
        if branches.len().checked_add(next.len()).is_none_or(|n| n > MAX_BRANCHES) {
            return Err(bad_expansion());
        }
        branches.extend(next);
        check_expansion(&branches)?;
    }
    Ok(branches)
}

/// Parse a WHERE clause already past the WHERE keyword.
pub(crate) fn parse_where(parser: &mut Parser) -> Result<Branches, ParseError> {
    let mut conditions = 0;
    parse_or(parser, &mut conditions, 0)
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
    fn nested_boolean_groups_respect_and_precedence() {
        assert_eq!(
            run("SELECT number FROM numbers(8) WHERE (number = 1 OR number = 2) AND number >= 2 ORDER BY number"),
            "number\n2\n"
        );
        assert_eq!(
            run("SELECT number FROM numbers(8) WHERE number = 1 OR number = 2 AND number > 2 ORDER BY number"),
            "number\n1\n"
        );
    }

    #[test]
    fn distributes_and_over_or_without_changing_between() {
        assert_eq!(
            run("SELECT number FROM numbers(8) WHERE number BETWEEN 1 AND 5 AND (number = 2 OR number = 4) ORDER BY number"),
            "number\n2\n4\n"
        );
    }

    #[test]
    fn rejects_bad_brackets_and_exponential_expansion() {
        for sql in [
            "SELECT number FROM numbers(8) WHERE (number = 1",
            "SELECT number FROM numbers(8) WHERE number = 1)",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
        let groups = (0..7).map(|_| "(number = 1 OR number = 2)")
            .collect::<Vec<_>>().join(" AND ");
        let sql = format!("SELECT number FROM numbers(8) WHERE {groups}");
        assert!(parse_select(&sql).is_err());
    }
}
