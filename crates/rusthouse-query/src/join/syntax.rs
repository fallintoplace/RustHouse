use super::*;

fn parse_relation(parser: &mut Parser) -> Result<String, ParseError> {
    let relation = parser.word()?;
    if !parser.take(Token::Dot) {
        return Err(ParseError(
            "JOIN columns must be qualified (alias.column)".into(),
        ));
    }
    Ok(relation)
}

pub(super) fn qualified(parser: &mut Parser) -> Result<JoinColumn, ParseError> {
    let relation = parse_relation(parser)?;
    let name = parser.word()?;
    Ok(JoinColumn { relation, name })
}

pub(super) fn projected(parser: &mut Parser) -> Result<JoinColumn, ParseError> {
    let relation = parse_relation(parser)?;
    let name = if parser.take(Token::Star) {
        "*".to_owned()
    } else {
        parser.word()?
    };
    Ok(JoinColumn { relation, name })
}

pub(super) fn alias(parser: &mut Parser, table: &str) -> Result<String, ParseError> {
    if parser.keyword("AS") {
        return parser.word();
    }
    match parser.tokens.get(parser.index) {
        Some(Token::Word(word))
            if ![
                "INNER", "LEFT", "RIGHT", "FULL", "CROSS", "OUTER", "JOIN", "ON", "USING", "WHERE",
                "GROUP", "ORDER", "LIMIT", "OFFSET", "ANY", "ALL",
            ]
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword)) =>
        {
            parser.word()
        }
        _ => Ok(table.to_owned()),
    }
}

pub(super) fn parse_join_kind(parser: &mut Parser) -> JoinType {
    let kind = if parser.keyword("LEFT") {
        JoinType::Left
    } else if parser.keyword("RIGHT") {
        JoinType::Right
    } else if parser.keyword("FULL") {
        JoinType::Full
    } else {
        parser.keyword("INNER");
        JoinType::Inner
    };
    if !matches!(kind, JoinType::Inner) {
        parser.keyword("OUTER");
    }
    kind
}

pub(super) fn equality_group(
    parser: &mut Parser,
    keys: &mut Vec<(JoinColumn, JoinColumn)>,
    depth: usize,
) -> Result<(), ParseError> {
    if depth >= 16 {
        return Err(ParseError("JOIN ON has too many nested parentheses".into()));
    }
    loop {
        if parser.take(Token::LParen) {
            equality_group(parser, keys, depth + 1)?;
            if !parser.take(Token::RParen) {
                return Err(ParseError("JOIN ON requires closing ')'".into()));
            }
        } else {
            // Repeated equalities count against the raw limit, but not the distinct-key limit.
            if keys.len() == 32 {
                return Err(ParseError("JOIN supports at most 32 ON equalities".into()));
            }
            let first = qualified(parser)?;
            if !parser.take(Token::Operator(super::super::Operator::Eq)) {
                return Err(ParseError(
                    "JOIN ON supports AND-connected equality keys".into(),
                ));
            }
            let second = qualified(parser)?;
            keys.push((first, second));
        }
        if !parser.keyword("AND") {
            break;
        }
    }
    Ok(())
}

pub(super) fn bound(parser: &mut Parser, label: &str) -> Result<usize, ParseError> {
    let number = parser.number()?;
    if !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ParseError(format!(
            "{label} must be a non-negative integer"
        )));
    }
    number
        .parse::<usize>()
        .map_err(|_| ParseError(format!("{label} exceeds address space")))
}
