//! Convert a bounded conjunctive comma join into the existing inner join plan.
use super::{ParseError, Parser, Token, alias};
use crate::Operator;

type SchemaResolver<'a> = &'a mut (dyn FnMut(&str) -> Result<Vec<String>, ParseError> + 'a);

fn keyword(token: &Token, name: &str) -> bool {
    matches!(token, Token::Word(w) if w.eq_ignore_ascii_case(name))
}

fn clause_end(token: &Token) -> bool {
    ["GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET", "FORMAT"]
        .iter()
        .any(|name| keyword(token, name))
        || *token == Token::Semicolon
}

fn qualifier(token: &Token) -> Option<&str> {
    match token {
        Token::Word(s) | Token::QuotedWord(s) => Some(s),
        _ => None,
    }
}

fn unqualified_column(token: &Token) -> Option<&str> {
    match token {
        Token::Word(name)
            if [
                "CAST",
                "DATE",
                "TIMESTAMP",
                "NULL",
                "TRUE",
                "FALSE",
                "INTERVAL",
            ]
            .iter()
            .any(|keyword| name.eq_ignore_ascii_case(keyword)) =>
        {
            None
        }
        _ => qualifier(token),
    }
}

fn from_bounds(tokens: &[Token]) -> Result<Option<(usize, usize)>, ParseError> {
    if !matches!(tokens.first(), Some(t) if keyword(t, "SELECT")) {
        return Ok(None);
    }
    let mut depth = 0usize;
    let mut from = None;
    let mut end = None;
    let mut has_comma = false;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && from.is_none() && keyword(token, "FROM") => from = Some(i),
            Token::Comma if depth == 0 && from.is_some() && end.is_none() => has_comma = true,
            _ if depth == 0
                && from.is_some()
                && end.is_none()
                && (keyword(token, "WHERE") || clause_end(token)) =>
            {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    if !has_comma {
        return Ok(None);
    }
    Ok(Some((
        from.expect("comma follows FROM"),
        end.unwrap_or(tokens.len()),
    )))
}

pub(super) fn has_comma(tokens: &[Token]) -> Result<bool, ParseError> {
    Ok(from_bounds(tokens)?.is_some())
}

/// Turn comma sources after an explicit join into inner joins. Only equality
/// keys that connect them to a preserved side of the existing join are moved
/// from WHERE into ON, so a preceding outer join keeps its null behavior.
fn rewrite_mixed_join(
    tokens: &[Token],
    from: usize,
    end: usize,
    comma: usize,
    mut resolve: Option<SchemaResolver<'_>>,
) -> Result<Vec<Token>, ParseError> {
    if end == tokens.len() || !keyword(&tokens[end], "WHERE") {
        return Err(ParseError(
            "mixed comma joins require WHERE equality keys".into(),
        ));
    }

    let prefix = &tokens[from + 1..comma];
    let mut prefix_parser = Parser {
        tokens: prefix.to_vec(),
        index: 0,
    };
    let first_table = prefix_parser.bounded_word()?;
    let first_alias = alias(&mut prefix_parser, &first_table)?;
    let mut relations = vec![(first_table, first_alias)];
    let mut nullable_aliases = Vec::new();
    let mut depth = 0usize;
    for (index, token) in prefix.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && keyword(token, "JOIN") => {
                if index > 0
                    && (keyword(&prefix[index - 1], "RIGHT")
                        || keyword(&prefix[index - 1], "FULL")
                        || (index > 1
                            && keyword(&prefix[index - 2], "RIGHT")
                            && keyword(&prefix[index - 1], "OUTER"))
                        || (index > 1
                            && keyword(&prefix[index - 2], "FULL")
                            && keyword(&prefix[index - 1], "OUTER")))
                {
                    return Err(ParseError(
                        "mixed comma joins do not support RIGHT or FULL JOIN".into(),
                    ));
                }
                let mut table_parser = Parser {
                    tokens: prefix[index + 1..].to_vec(),
                    index: 0,
                };
                let table = table_parser.bounded_word()?;
                let relation_alias = alias(&mut table_parser, &table)?;
                if relations
                    .iter()
                    .any(|(_, existing)| existing.eq_ignore_ascii_case(&relation_alias))
                {
                    return Err(ParseError("joined tables require distinct aliases".into()));
                }
                if index > 0
                    && (keyword(&prefix[index - 1], "LEFT")
                        || (index > 1
                            && keyword(&prefix[index - 2], "LEFT")
                            && keyword(&prefix[index - 1], "OUTER")))
                {
                    nullable_aliases.push(relation_alias.clone());
                }
                relations.push((table, relation_alias));
            }
            _ => {}
        }
    }
    if relations.len() == 1 {
        return Err(ParseError(
            "mixed comma join requires an explicit JOIN".into(),
        ));
    }

    let mut source_parser = Parser {
        tokens: tokens[comma + 1..end].to_vec(),
        index: 0,
    };
    let mut added = Vec::new();
    loop {
        let table = source_parser.bounded_word()?;
        let relation_alias = alias(&mut source_parser, &table)?;
        if relations
            .iter()
            .any(|(_, existing)| existing.eq_ignore_ascii_case(&relation_alias))
        {
            return Err(ParseError("joined tables require distinct aliases".into()));
        }
        relations.push((table, relation_alias));
        added.push(relations.len() - 1);
        if relations.len() > super::MAX_JOIN_TABLES {
            return Err(ParseError(format!(
                "JOIN supports at most {} tables",
                super::MAX_JOIN_TABLES
            )));
        }
        if !source_parser.take(Token::Comma) {
            break;
        }
    }
    if source_parser.index != source_parser.tokens.len() {
        return Err(ParseError(
            "mixed comma joins support named tables only".into(),
        ));
    }

    let mut schemas = Vec::with_capacity(relations.len());
    if let Some(resolver) = resolve.as_mut() {
        for (table, _) in &relations {
            schemas.push(resolver(table)?);
        }
    }

    let mut pred_end = end + 1;
    let mut depth = 0usize;
    while pred_end < tokens.len() {
        match &tokens[pred_end] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            token if depth == 0 && clause_end(token) => break,
            _ => {}
        }
        pred_end += 1;
    }
    let predicates = &tokens[end + 1..pred_end];
    let mut pieces = Vec::<Vec<Token>>::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, token) in predicates.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && keyword(token, "AND") => {
                pieces.push(predicates[start..index].to_vec());
                start = index + 1;
            }
            _ => {}
        }
    }
    pieces.push(predicates[start..].to_vec());
    if pieces.iter().any(Vec::is_empty) {
        return Err(ParseError(
            "mixed comma join requires complete predicates".into(),
        ));
    }

    let column_owner = |column: &[Token]| -> Result<Option<(usize, String)>, ParseError> {
        match column {
            [relation, Token::Dot, name] => {
                let (Some(relation), Some(name)) = (qualifier(relation), qualifier(name)) else {
                    return Ok(None);
                };
                Ok(relations
                    .iter()
                    .position(|(_, alias)| alias.eq_ignore_ascii_case(relation))
                    .map(|index| (index, name.to_owned())))
            }
            [name] => {
                let Some(name) = unqualified_column(name) else {
                    return Ok(None);
                };
                let matches = schemas
                    .iter()
                    .enumerate()
                    .filter(|(_, columns)| {
                        columns
                            .iter()
                            .any(|column| column.eq_ignore_ascii_case(name))
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                if matches.len() > 1 {
                    return Err(ParseError(format!("ambiguous JOIN column: {name}")));
                }
                Ok(matches.first().map(|index| (*index, name.to_owned())))
            }
            _ => Ok(None),
        }
    };

    let mut join_keys = vec![Vec::<Vec<Token>>::new(); relations.len()];
    let mut remainder = Vec::new();
    for piece in pieces {
        let equality = match piece.as_slice() {
            [left, Token::Operator(Operator::Eq), right] => {
                Some((std::slice::from_ref(left), std::slice::from_ref(right)))
            }
            [
                Token::LParen,
                left,
                Token::Operator(Operator::Eq),
                right,
                Token::RParen,
            ] => Some((std::slice::from_ref(left), std::slice::from_ref(right))),
            [
                _,
                Token::Dot,
                _,
                Token::Operator(Operator::Eq),
                _,
                Token::Dot,
                _,
            ] => Some((&piece[..3], &piece[4..])),
            [
                Token::LParen,
                _,
                Token::Dot,
                _,
                Token::Operator(Operator::Eq),
                _,
                Token::Dot,
                _,
                Token::RParen,
            ] => Some((&piece[1..4], &piece[5..8])),
            _ => None,
        };
        let Some((left, right)) = equality else {
            remainder.push(piece);
            continue;
        };
        let (Some((left_index, left_name)), Some((right_index, right_name))) =
            (column_owner(left)?, column_owner(right)?)
        else {
            remainder.push(piece);
            continue;
        };
        let key = added.iter().copied().find_map(|current| {
            let other = if left_index == current {
                Some((right_index, &right_name, &left_name))
            } else if right_index == current {
                Some((left_index, &left_name, &right_name))
            } else {
                None
            }?;
            let (prior, prior_name, current_name) = other;
            let prior_is_safe = prior < current
                && !nullable_aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(&relations[prior].1));
            prior_is_safe.then(|| {
                (
                    current,
                    vec![
                        Token::Word(relations[prior].1.clone()),
                        Token::Dot,
                        Token::Word(prior_name.clone()),
                        Token::Operator(Operator::Eq),
                        Token::Word(relations[current].1.clone()),
                        Token::Dot,
                        Token::Word(current_name.clone()),
                    ],
                )
            })
        });
        if let Some((current, key)) = key {
            join_keys[current].push(key);
        } else {
            remainder.push(piece);
        }
    }

    let word = |value: &str| Token::Word(value.to_owned());
    let mut result = tokens[..comma].to_vec();
    for index in added {
        if join_keys[index].is_empty() {
            return Err(ParseError(
                "mixed comma join requires equality to an earlier preserved table".into(),
            ));
        }
        result.extend([
            word("INNER"),
            word("JOIN"),
            word(&relations[index].0),
            word("AS"),
            word(&relations[index].1),
            word("ON"),
        ]);
        for (key_index, key) in join_keys[index].iter().enumerate() {
            if key_index != 0 {
                result.push(word("AND"));
            }
            result.extend(key.iter().cloned());
        }
    }
    if !remainder.is_empty() {
        result.push(word("WHERE"));
        for (index, piece) in remainder.iter().enumerate() {
            if index != 0 {
                result.push(word("AND"));
            }
            result.extend(piece.iter().cloned());
        }
    }
    result.extend_from_slice(&tokens[pred_end..]);
    Ok(result)
}

// A shared equality in every OR branch can be used as a join key without
// changing which rows the original WHERE expression accepts.
fn rewrite_disjunctive(
    tokens: &[Token],
    from: usize,
    end: usize,
    pred_end: usize,
    tables: &[(String, String)],
    resolver: Option<SchemaResolver<'_>>,
) -> Result<Vec<Token>, ParseError> {
    if tables.len() < 2 {
        return Err(ParseError(
            "OR comma joins require at least two tables".into(),
        ));
    }
    let resolver =
        resolver.ok_or_else(|| ParseError("OR comma joins require schema resolution".into()))?;
    let schemas = tables
        .iter()
        .map(|(table, _)| resolver(table))
        .collect::<Result<Vec<_>, _>>()?;
    let mut parser = Parser {
        tokens: tokens[end + 1..pred_end].to_vec(),
        index: 0,
    };
    let branches = crate::where_parser::parse_where(&mut parser)?;
    if parser.index != parser.tokens.len() || branches.is_empty() {
        return Err(ParseError(
            "OR comma join requires complete predicates".into(),
        ));
    }
    let lookup = |column: &str| -> Result<Option<usize>, ParseError> {
        let found = schemas
            .iter()
            .enumerate()
            .filter_map(|(i, cols)| cols.iter().any(|name| name == column).then_some(i))
            .collect::<Vec<_>>();
        if found.len() > 1 {
            return Err(ParseError(format!("ambiguous JOIN column: {column}")));
        }
        Ok(found.first().copied())
    };
    let mut shared = Vec::<(usize, String, usize, String)>::new();
    for predicate in &branches[0] {
        let crate::Predicate::ColumnComparison {
            column,
            operator: Operator::Eq,
            other,
        } = predicate
        else {
            continue;
        };
        let (Some(left), Some(right)) = (lookup(column)?, lookup(other)?) else {
            continue;
        };
        if left == right {
            continue;
        }
        let edge = if left < right {
            (left, column.clone(), right, other.clone())
        } else {
            (right, other.clone(), left, column.clone())
        };
        let present_in_every_branch = branches.iter().all(|branch| {
            branch.iter().any(|entry| {
                let crate::Predicate::ColumnComparison {
                    column: branch_left,
                    operator: Operator::Eq,
                    other: branch_right,
                } = entry
                else {
                    return false;
                };
                let (Ok(Some(branch_left_table)), Ok(Some(branch_right_table))) =
                    (lookup(branch_left), lookup(branch_right))
                else {
                    return false;
                };
                let candidate = if branch_left_table < branch_right_table {
                    (
                        branch_left_table,
                        branch_left.as_str(),
                        branch_right_table,
                        branch_right.as_str(),
                    )
                } else {
                    (
                        branch_right_table,
                        branch_right.as_str(),
                        branch_left_table,
                        branch_left.as_str(),
                    )
                };
                candidate == (edge.0, edge.1.as_str(), edge.2, edge.3.as_str())
            })
        });
        if present_in_every_branch && !shared.contains(&edge) {
            shared.push(edge);
        }
    }

    let mut edges = Vec::<(usize, usize, Vec<(String, String)>)>::new();
    for (left, left_key, right, right_key) in shared {
        if let Some((_, _, keys)) = edges.iter_mut().find(|(a, b, _)| *a == left && *b == right) {
            keys.push((left_key, right_key));
        } else {
            edges.push((left, right, vec![(left_key, right_key)]));
        }
    }

    let mut order = vec![0usize];
    while order.len() < tables.len() {
        let next = (1..tables.len()).find(|candidate| {
            !order.contains(candidate)
                && edges.iter().any(|(a, b, _)| {
                    (*a == *candidate && order.contains(b))
                        || (*b == *candidate && order.contains(a))
                })
        });
        match next {
            Some(index) => order.push(index),
            None => {
                return Err(ParseError(
                    "OR comma join requires shared equality keys connecting every table".into(),
                ));
            }
        }
    }
    let changed_order = order.iter().enumerate().any(|(i, &index)| i != index);
    if changed_order
        && matches!(
            &tokens[1..from],
            [Token::Star] | [Token::Word(_), Token::Star]
        )
    {
        return Err(ParseError(
            "reordered comma joins require explicit projections".into(),
        ));
    }

    let word = |s: &str| Token::Word(s.to_owned());
    let mut result = tokens[..from + 1].to_vec();
    let (first_table, first_alias) = &tables[order[0]];
    result.extend([word(first_table), word("AS"), word(first_alias)]);
    for (position, &original) in order.iter().enumerate().skip(1) {
        let (table, alias) = &tables[original];
        let connections = edges
            .iter()
            .filter(|(left, right, _)| {
                (*left == original && order[..position].contains(right))
                    || (*right == original && order[..position].contains(left))
            })
            .collect::<Vec<_>>();
        result.extend([
            word("JOIN"),
            word(table),
            word("AS"),
            word(alias),
            word("ON"),
        ]);
        let mut first_key = true;
        for (left, right, keys) in connections {
            for (left_key, right_key) in keys {
                if !first_key {
                    result.push(word("AND"));
                }
                first_key = false;
                result.extend([
                    word(&tables[*left].1),
                    Token::Dot,
                    word(left_key),
                    Token::Operator(Operator::Eq),
                    word(&tables[*right].1),
                    Token::Dot,
                    word(right_key),
                ]);
            }
        }
        debug_assert!(!first_key, "every joined OR table has an equality key");
    }
    result.push(word("WHERE"));
    result.extend_from_slice(&tokens[end + 1..]);
    Ok(result)
}

/// Rewrite AND-connected comma joins into existing inner join plans.
pub(super) fn rewrite(
    tokens: Vec<Token>,
    mut resolve: Option<SchemaResolver<'_>>,
) -> Result<Vec<Token>, ParseError> {
    let Some((from, end)) = from_bounds(&tokens)? else {
        return Ok(tokens);
    };
    let mut depth = 0usize;
    let mut first_comma = None;
    let mut explicit_join_before_comma = false;
    for (index, token) in tokens.iter().enumerate().take(end).skip(from + 1) {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Comma if depth == 0 => {
                first_comma = Some(index);
                break;
            }
            _ if depth == 0 && keyword(token, "JOIN") => explicit_join_before_comma = true,
            _ => {}
        }
    }
    if explicit_join_before_comma {
        return rewrite_mixed_join(
            &tokens,
            from,
            end,
            first_comma.expect("a mixed join has a comma after its explicit join"),
            resolve,
        );
    }
    if end == tokens.len() || !keyword(&tokens[end], "WHERE") {
        return Err(ParseError("comma joins require WHERE equality keys".into()));
    }
    let mut parser = Parser {
        tokens: tokens[from + 1..end].to_vec(),
        index: 0,
    };
    let mut tables = Vec::<(String, String)>::new();
    loop {
        let name = parser.bounded_word()?;
        let alias = alias(&mut parser, &name)?;
        if tables.iter().any(|(_, existing)| existing == &alias) {
            return Err(ParseError("joined tables require distinct aliases".into()));
        }
        tables.push((name, alias));
        if tables.len() > super::MAX_JOIN_TABLES {
            return Err(ParseError(format!(
                "JOIN supports at most {} tables",
                super::MAX_JOIN_TABLES
            )));
        }
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if parser.index != parser.tokens.len() || tables.len() < 2 {
        return Err(ParseError("comma join supports named tables only".into()));
    }
    let mut pred_end = end + 1;
    let mut depth = 0usize;
    while pred_end < tokens.len() {
        match &tokens[pred_end] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            t if depth == 0 && clause_end(t) => break,
            _ => {}
        }
        pred_end += 1;
    }
    let predicates = &tokens[end + 1..pred_end];
    let mut depth = 0usize;
    let mut top_level_or = false;
    for token in predicates {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && keyword(token, "OR") => top_level_or = true,
            _ => {}
        }
    }
    if top_level_or {
        return rewrite_disjunctive(&tokens, from, end, pred_end, &tables, resolve);
    }
    let mut pieces = Vec::<Vec<Token>>::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, token) in predicates.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && keyword(token, "AND") => {
                pieces.push(predicates[start..i].to_vec());
                start = i + 1;
            }
            _ => {}
        }
    }
    pieces.push(predicates[start..].to_vec());
    if pieces.iter().any(Vec::is_empty) {
        return Err(ParseError("comma join requires complete predicates".into()));
    }
    let needs_schema = pieces.iter().any(|piece| match piece.as_slice() {
        [left, Token::Operator(Operator::Eq), right] => {
            unqualified_column(left).is_some() && unqualified_column(right).is_some()
        }
        [left, Token::Operator(Operator::Eq), _, Token::Dot, right]
        | [_, Token::Dot, right, Token::Operator(Operator::Eq), left] => {
            unqualified_column(left).is_some() && qualifier(right).is_some()
        }
        _ => false,
    });
    let schemas = if let (true, Some(resolver)) = (needs_schema, resolve.as_mut()) {
        Some(
            tables
                .iter()
                .map(|(table, _)| resolver(table))
                .collect::<Result<Vec<_>, _>>()?,
        )
    } else {
        None
    };
    let mut edges = Vec::<(usize, usize, Vec<Token>)>::new();
    let mut remainder = Vec::new();
    for mut piece in pieces {
        if let Some(schemas) = &schemas {
            let unqualified = match piece.as_slice() {
                [name, Token::Operator(Operator::Eq), _, Token::Dot, _]
                    if unqualified_column(name).is_some() =>
                {
                    Some(0)
                }
                [_, Token::Dot, _, Token::Operator(Operator::Eq), name]
                    if unqualified_column(name).is_some() =>
                {
                    Some(4)
                }
                _ => None,
            };
            if let Some(index) = unqualified {
                let name = unqualified_column(&piece[index]).expect("unqualified column");
                let found = schemas
                    .iter()
                    .enumerate()
                    .filter(|(_, cols)| cols.iter().any(|column| column == name))
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                if found.len() > 1 {
                    return Err(ParseError(format!("ambiguous JOIN column: {name}")));
                }
                if let Some(&table) = found.first() {
                    piece.splice(
                        index..index,
                        [Token::Word(tables[table].1.clone()), Token::Dot],
                    );
                }
            }
        }
        if let (Some(schemas), [left, Token::Operator(Operator::Eq), right]) =
            (&schemas, piece.as_slice())
        {
            if let (Some(left_name), Some(right_name)) =
                (unqualified_column(left), unqualified_column(right))
            {
                let lookup = |name: &str| -> Result<Option<usize>, ParseError> {
                    let found: Vec<_> = schemas
                        .iter()
                        .enumerate()
                        .filter(|(_, cols)| cols.iter().any(|c| c == name))
                        .map(|(i, _)| i)
                        .collect();
                    if found.len() > 1 {
                        return Err(ParseError(format!("ambiguous JOIN column: {name}")));
                    }
                    Ok(found.first().copied())
                };
                if let (Some(a), Some(b)) = (lookup(left_name)?, lookup(right_name)?) {
                    if a != b {
                        piece = vec![
                            Token::Word(tables[a].1.clone()),
                            Token::Dot,
                            left.clone(),
                            Token::Operator(Operator::Eq),
                            Token::Word(tables[b].1.clone()),
                            Token::Dot,
                            right.clone(),
                        ];
                    }
                }
            }
        }
        if let [
            a,
            Token::Dot,
            left,
            Token::Operator(Operator::Eq),
            b,
            Token::Dot,
            right,
        ] = piece.as_slice()
        {
            if qualifier(left).is_some() && qualifier(right).is_some() {
                let ai = qualifier(a)
                    .and_then(|name| tables.iter().position(|(_, alias)| alias == name));
                let bi = qualifier(b)
                    .and_then(|name| tables.iter().position(|(_, alias)| alias == name));
                if let (Some(ai), Some(bi)) = (ai, bi) {
                    if ai != bi {
                        edges.push((ai, bi, piece));
                        continue;
                    }
                }
            }
        }
        remainder.push(piece);
    }
    // Choose a connected left-deep order. A later table can connect two
    // earlier FROM entries, so the written order is not always executable.
    let mut order = vec![0usize];
    while order.len() < tables.len() {
        let next = (1..tables.len()).find(|candidate| {
            !order.contains(candidate)
                && edges.iter().any(|(a, b, _)| {
                    (*a == *candidate && order.contains(b))
                        || (*b == *candidate && order.contains(a))
                })
        });
        match next {
            Some(index) => order.push(index),
            None => {
                if predicates.iter().any(|token| keyword(token, "OR")) {
                    return rewrite_disjunctive(&tokens, from, end, pred_end, &tables, resolve);
                }
                return Err(ParseError(
                    "comma joins require a connected graph of equality keys".into(),
                ));
            }
        }
    }
    let changed_order = order.iter().enumerate().any(|(i, &index)| i != index);
    // Wildcard output follows the original FROM order. Do not silently change it.
    if changed_order
        && matches!(
            &tokens[1..from],
            [Token::Star] | [Token::Word(_), Token::Star]
        )
    {
        return Err(ParseError(
            "reordered comma joins require explicit projections".into(),
        ));
    }
    let mut position = vec![0usize; tables.len()];
    for (index, &original) in order.iter().enumerate() {
        position[original] = index;
    }
    let mut keys = vec![Vec::<Vec<Token>>::new(); tables.len()];
    for (left, right, piece) in edges {
        keys[position[left].max(position[right])].push(piece);
    }
    let word = |s: &str| Token::Word(s.to_owned());
    let mut result = tokens[..from + 1].to_vec();
    let (first_table, first_alias) = &tables[order[0]];
    result.push(word(first_table));
    result.push(word("AS"));
    result.push(word(first_alias));
    for (index, &original) in order.iter().enumerate().skip(1) {
        let (table, name) = &tables[original];
        result.push(word("JOIN"));
        result.push(word(table));
        result.push(word("AS"));
        result.push(word(name));
        result.push(word("ON"));
        for (i, key) in keys[index].iter().enumerate() {
            if i != 0 {
                result.push(word("AND"));
            }
            result.extend(key.iter().cloned());
        }
    }
    if !remainder.is_empty() {
        result.push(word("WHERE"));
        for (i, part) in remainder.iter().enumerate() {
            if i != 0 {
                result.push(word("AND"));
            }
            result.extend(part.iter().cloned());
        }
    }
    result.extend_from_slice(&tokens[pred_end..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::parse_join;
    #[test]
    fn or_comma_join_requires_a_shared_equality_in_every_branch() {
        let schema = |table: &str| {
            Ok(match table {
                "first" => vec!["a_id".into(), "amount".into()],
                "second" => vec!["b_id".into(), "kind".into()],
                _ => vec![],
            })
        };
        let good = "SELECT SUM(amount * 1) FROM first, second WHERE (a_id = b_id AND kind = 'a') OR (b_id = a_id AND kind = 'b')";
        assert!(
            super::super::parse_join_with_schema(good, schema)
                .unwrap()
                .is_some()
        );
        let unsafe_query = "SELECT SUM(amount * 1) FROM first, second WHERE (a_id = b_id AND kind = 'a') OR kind = 'b'";
        assert!(super::super::parse_join_with_schema(unsafe_query, schema).is_err());
        let wrapped = "SELECT SUM(amount * 1) FROM first, second WHERE ((a_id = b_id AND kind = 'a') OR (b_id = a_id AND kind = 'b'))";
        assert!(
            super::super::parse_join_with_schema(wrapped, schema)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn disjunctive_comma_join_connects_more_than_two_tables() {
        let schema = |table: &str| {
            Ok(match table {
                "fact" => vec!["a_id".into(), "b_id".into(), "amount".into()],
                "dim_a" => vec!["dim_a_id".into(), "kind".into()],
                "dim_b" => vec!["dim_b_id".into()],
                _ => vec![],
            })
        };
        let sql = "SELECT fact.a_id, dim_b.dim_b_id FROM fact, dim_a, dim_b WHERE ((a_id = dim_a_id AND kind = 'a') OR (dim_a_id = a_id AND kind = 'b')) AND b_id = dim_b_id";
        let query = super::super::parse_join_with_schema(sql, schema)
            .unwrap()
            .unwrap();
        assert_eq!(query.steps.len(), 1);
        assert_eq!(query.steps[0].table, "dim_b");
    }

    #[test]
    fn comma_join_folds_unspaced_integer_arithmetic_in_filters() {
        let with_in = "SELECT d3.d_year FROM date_dim d1, date_dim d2, date_dim d3 WHERE d1.d_date_sk = d2.d_date_sk AND d2.d_date_sk = d3.d_date_sk AND d3.d_year IN (1999,1999+1,1999+2)";
        let query = parse_join(with_in).unwrap().unwrap();
        assert!(query.scalar_where.is_some());

        let with_between = "SELECT d3.d_year FROM date_dim d1, date_dim d2, date_dim d3 WHERE d1.d_date_sk = d2.d_date_sk AND d2.d_date_sk = d3.d_date_sk AND d2.d_moy BETWEEN 9 AND 9 + 3";
        let query = parse_join(with_between).unwrap().unwrap();
        assert!(query.scalar_where.is_some());
    }

    #[test]
    fn disjunctive_comma_join_keeps_only_equalities_shared_by_all_branches() {
        let schema = |table: &str| {
            Ok(match table {
                "fact" => vec!["a_id".into(), "b_id".into(), "c_id".into(), "amount".into()],
                "dim_a" => vec!["dim_a_id".into(), "kind".into()],
                "dim_b" => vec!["dim_b_id".into(), "flag".into()],
                "dim_c" => vec!["dim_c_id".into()],
                _ => vec![],
            })
        };
        let sql = "SELECT fact.a_id, dim_c.dim_c_id FROM fact, dim_a, dim_b, dim_c WHERE ((a_id = dim_a_id AND kind = 'a') OR (dim_a_id = a_id AND kind = 'b')) AND ((b_id = dim_b_id AND flag = 'x') OR (dim_b_id = b_id AND flag = 'y')) AND c_id = dim_c_id";
        let query = super::super::parse_join_with_schema(sql, schema)
            .unwrap()
            .unwrap();
        assert_eq!(query.steps.len(), 2);

        let unsafe_sql = "SELECT fact.a_id FROM fact, dim_a, dim_b WHERE ((a_id = dim_a_id AND kind = 'a') OR (kind = 'b')) AND b_id = dim_b_id";
        assert!(super::super::parse_join_with_schema(unsafe_sql, schema).is_err());
    }

    #[test]
    fn qualified_join_does_not_require_schema_resolution() {
        let parsed = super::super::parse_join_with_schema(
            "SELECT a.id FROM first a, second b WHERE a.id = b.id",
            |_| panic!("qualified join does not need a schema"),
        )
        .unwrap();
        assert!(parsed.is_some());
    }

    #[test]
    fn boolean_literals_are_not_resolved_as_columns() {
        let resolve = |table: &str| {
            Ok(if table == "first" {
                vec!["flag".into()]
            } else {
                vec!["TRUE".into()]
            })
        };
        assert!(
            super::super::parse_join_with_schema(
                "SELECT count(*) FROM first, second WHERE flag = TRUE",
                resolve
            )
            .is_err()
        );
    }

    #[test]
    fn handles_qualified_implicit_joins_without_moving_or_predicates() {
        let parsed = parse_join("SELECT a.id, b.id FROM first a, second b WHERE a.id = b.id")
            .unwrap()
            .unwrap();
        assert_eq!(parsed.keys.len(), 1);
        assert!(
            parse_join("SELECT a.id FROM first a, second b WHERE a.id = b.id OR a.id = 1").is_err()
        );
        assert!(parse_join("SELECT a.id FROM first a, second b WHERE a.id = 1").is_err());
        let chained = parse_join(
            "SELECT a.id FROM first a, second b, third c WHERE b.id = c.id AND a.id = b.id",
        )
        .unwrap()
        .unwrap();
        assert_eq!(chained.steps.len(), 1);
    }

    #[test]
    fn supports_job_sized_comma_joins_with_a_bounded_table_count() {
        let query_for_tables = |table_count: usize| {
            let mut sql = String::from("SELECT t0.id FROM t0 AS t0");
            for index in 1..table_count {
                sql.push_str(&format!(", t{index} AS t{index}"));
            }
            sql.push_str(" WHERE t0.id = t1.id");
            for index in 2..table_count {
                sql.push_str(&format!(" AND t{}.id = t{index}.id", index - 1));
            }
            sql
        };

        let query = parse_join(&query_for_tables(super::super::MAX_JOIN_TABLES))
            .unwrap()
            .unwrap();
        assert_eq!(query.steps.len(), super::super::MAX_JOIN_TABLES - 2);
        assert!(parse_join(&query_for_tables(super::super::MAX_JOIN_TABLES + 1)).is_err());
    }

    #[test]
    fn mixed_comma_sources_become_inner_joins_after_a_left_join() {
        let schema = |table: &str| {
            Ok(match table {
                "a" => vec!["a_id".into(), "flag".into()],
                "b" => vec!["b_a_id".into(), "amount".into()],
                "c" => vec![
                    "c_a_id".into(),
                    "flag".into(),
                    "value".into(),
                    "amount".into(),
                ],
                _ => vec![],
            })
        };
        let sql = "SELECT a.a_id, c.value FROM a LEFT OUTER JOIN b ON a.a_id = b.b_a_id, c WHERE c.c_a_id = a.a_id";
        let query = super::super::parse_join_with_schema(sql, schema)
            .unwrap()
            .unwrap();
        assert_eq!(query.kind, super::super::JoinType::Left);
        assert_eq!(query.steps.len(), 1);
        assert_eq!(query.steps[0].table, "c");
        assert_eq!(query.steps[0].kind, super::super::JoinType::Inner);
        assert_eq!(query.steps[0].keys.len(), 1);
        assert_eq!(query.steps[0].keys[0].0.name, "a_id");
        assert_eq!(query.steps[0].keys[0].1.name, "c_a_id");

        let grouped_sql = "SELECT a.a_id, SUM(CASE WHEN c.flag > 0 THEN c.amount - coalesce(b.amount, 0) ELSE 0 END) AS total FROM a LEFT OUTER JOIN b ON a.a_id = b.b_a_id, c WHERE c.c_a_id = a.a_id AND a.flag BETWEEN 1 AND 10 GROUP BY a.a_id";
        let grouped = super::super::parse_join_with_schema(grouped_sql, schema)
            .unwrap()
            .unwrap();
        assert!(matches!(
            &grouped.grouped_case_counts[0].1,
            super::super::GroupedCaseValue::Expression(_)
        ));

        let unsafe_sql = "SELECT a.a_id, c.value FROM a LEFT JOIN b ON a.a_id = b.b_a_id, c WHERE c.c_a_id = b.b_a_id";
        assert!(super::super::parse_join_with_schema(unsafe_sql, schema).is_err());
    }
}
