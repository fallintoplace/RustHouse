use super::*;

pub(super) fn render_set_token(token: &Token) -> String {
    match token {
        Token::Word(value) | Token::Number(value) => value.clone(),
        Token::QuotedWord(value) | Token::QuotedAlias(value) => {
            format!("\"{}\"", value.replace('"', "\"\""))
        }
        Token::String(value) => format!("'{}'", value.replace('\'', "''")),
        Token::Star => "*".into(),
        Token::Slash => "/".into(),
        Token::Percent => "%".into(),
        Token::Plus => "+".into(),
        Token::Minus => "-".into(),
        Token::Comma => ",".into(),
        Token::Dot => ".".into(),
        Token::LParen => "(".into(),
        Token::RParen => ")".into(),
        Token::Semicolon => ";".into(),
        Token::Operator(operator) => match operator {
            crate::Operator::Eq => "=".into(),
            crate::Operator::Ne => "!=".into(),
            crate::Operator::Lt => "<".into(),
            crate::Operator::Le => "<=".into(),
            crate::Operator::Gt => ">".into(),
            crate::Operator::Ge => ">=".into(),
            crate::Operator::Modulo(value) => format!("%{value}"),
            crate::Operator::ModuloNe(value) => format!("%{value} !="),
        },
    }
}

pub(super) fn render_set_tokens(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(render_set_token)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn enclosing_parentheses(tokens: &[Token]) -> Option<&[Token]> {
    if tokens.first() != Some(&Token::LParen) || tokens.last() != Some(&Token::RParen) {
        return None;
    }
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth = depth.checked_sub(1)?;
                if depth == 0 && index + 1 != tokens.len() {
                    return None;
                }
            }
            _ => {}
        }
    }
    (depth == 0).then_some(&tokens[1..tokens.len() - 1])
}

pub(super) fn split_derived_set(
    mut tokens: &[Token],
) -> Result<Option<(DerivedSetOperation, Vec<Vec<Token>>)>, ParseError> {
    while let Some(inner) = enclosing_parentheses(tokens) {
        tokens = inner;
    }
    let mut branches = Vec::new();
    let mut operation = None;
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word)
                if depth == 0
                    && (word.eq_ignore_ascii_case("EXCEPT")
                        || word.eq_ignore_ascii_case("INTERSECT")) =>
            {
                let current = if word.eq_ignore_ascii_case("EXCEPT") {
                    DerivedSetOperation::Except
                } else {
                    DerivedSetOperation::Intersect
                };
                if operation.is_some_and(|previous| previous != current) {
                    return Err(ParseError(
                        "derived set count cannot mix EXCEPT and INTERSECT".into(),
                    ));
                }
                operation = Some(current);
                if branches.len() == 2 {
                    return Err(ParseError(
                        "derived set count supports at most 3 SELECTs".into(),
                    ));
                }
                branches.push(tokens[start..index].to_vec());
                start = index + 1;
                if matches!(tokens.get(start), Some(Token::Word(next)) if next.eq_ignore_ascii_case("ALL"))
                {
                    return Err(ParseError("derived set count does not support ALL".into()));
                }
                if matches!(tokens.get(start), Some(Token::Word(next)) if next.eq_ignore_ascii_case("DISTINCT"))
                {
                    start += 1;
                }
            }
            _ => {}
        }
    }
    let Some(operation) = operation else {
        return Ok(None);
    };
    branches.push(tokens[start..].to_vec());
    if branches.len() < 2 || branches.iter().any(Vec::is_empty) {
        return Err(ParseError(
            "derived set count requires SELECT operands".into(),
        ));
    }
    for branch in &mut branches {
        if let Some(inner) = enclosing_parentheses(branch) {
            *branch = inner.to_vec();
        }
        let distinct_index = usize::from(
            matches!(branch.first(), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT")),
        );
        if matches!(branch.get(distinct_index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("DISTINCT"))
        {
            branch.remove(distinct_index);
        }
    }
    Ok(Some((operation, branches)))
}

pub(super) fn join_sources(query: &JoinSelect) -> Vec<(&str, &str)> {
    std::iter::once((query.left.as_str(), query.left_alias.as_str()))
        .chain(std::iter::once((
            query.right.as_str(),
            query.right_alias.as_str(),
        )))
        .chain(
            query
                .steps
                .iter()
                .map(|step| (step.table.as_str(), step.alias.as_str())),
        )
        .collect()
}

pub(super) fn projection_types(
    query: &JoinSelect,
    resolve: &mut impl FnMut(&str) -> Result<Vec<DerivedColumn>, ParseError>,
) -> Result<Option<Vec<String>>, ParseError> {
    let Some(columns) = query.columns.as_ref() else {
        return Ok(None);
    };
    if columns.is_empty() || columns.iter().any(|column| column.name == "*") {
        return Ok(None);
    }
    let sources = join_sources(query);
    let mut output = Vec::with_capacity(columns.len());
    for column in columns {
        let mut found = None;
        for (table, alias) in &sources {
            if !column.relation.is_empty()
                && !column.relation.eq_ignore_ascii_case(alias)
                && !column.relation.eq_ignore_ascii_case(table)
            {
                continue;
            }
            if let Some(metadata) = resolve(table)?
                .into_iter()
                .find(|item| item.name.eq_ignore_ascii_case(&column.name))
            {
                if found.replace(metadata.data_type).is_some() {
                    return Ok(None);
                }
            }
        }
        let Some(data_type) = found else {
            return Ok(None);
        };
        output.push(data_type);
    }
    Ok(Some(output))
}
