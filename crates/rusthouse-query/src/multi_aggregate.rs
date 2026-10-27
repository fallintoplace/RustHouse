use crate::expression::parse_expression_tokens;
use crate::lexer::Token;
use crate::{AggregateFunction, ParseError, Parser, ScalarExpr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateItem {
    pub function: Option<AggregateFunction>,
    pub argument: Option<ScalarExpr>,
    pub distinct: bool,
    pub alias: Option<String>,
}

impl AggregateItem {
    pub fn header(&self) -> String {
        if let Some(alias) = &self.alias {
            return alias.clone();
        }
        let function = self.function.map_or("count", AggregateFunction::name);
        let argument = self.argument.as_ref().map_or("*".into(), render_expr);
        if self.distinct {
            format!("count(DISTINCT {argument})")
        } else {
            format!("{function}({argument})")
        }
    }
}

fn render_expr(expr: &ScalarExpr) -> String {
    match expr {
        ScalarExpr::Column(name) => name.clone(),
        ScalarExpr::Integer(number) => number.to_string(),
        ScalarExpr::Add(left, right) => format!("{} + {}", render_expr(left), render_expr(right)),
        ScalarExpr::Subtract(left, right) => {
            format!("{} - {}", render_expr(left), render_expr(right))
        }
    }
}

fn aggregate_function(name: &str) -> Option<Option<AggregateFunction>> {
    match name.to_ascii_uppercase().as_str() {
        "COUNT" => Some(None),
        "SUM" => Some(Some(AggregateFunction::Sum)),
        "AVG" => Some(Some(AggregateFunction::Avg)),
        "MIN" => Some(Some(AggregateFunction::Min)),
        "MAX" => Some(Some(AggregateFunction::Max)),
        _ => None,
    }
}

pub(crate) fn parse_multiple(
    parser: &mut Parser,
) -> Result<Option<Vec<AggregateItem>>, ParseError> {
    let start = parser.index;
    let mut items = Vec::new();
    loop {
        let Some(Token::Word(name)) = parser.tokens.get(parser.index) else {
            parser.index = start;
            return Ok(None);
        };
        let Some(function) = aggregate_function(name) else {
            if items.is_empty() {
                parser.index = start;
                return Ok(None);
            }
            return Err(ParseError("expected aggregate function after comma".into()));
        };
        if parser.tokens.get(parser.index + 1) != Some(&Token::LParen) {
            parser.index = start;
            return Ok(None);
        }
        parser.index += 2;
        let distinct = parser.keyword("DISTINCT");
        if distinct && function.is_some() {
            return Err(ParseError("DISTINCT is only supported for COUNT".into()));
        }
        let argument = if parser.take(Token::Star) || parser.take(Token::RParen) {
            if function.is_some() {
                return Err(ParseError("numeric aggregate needs a column".into()));
            }
            if parser.tokens.get(parser.index - 1) == Some(&Token::RParen) {
                if distinct {
                    return Err(ParseError("COUNT(DISTINCT) requires a column".into()));
                }
                items.push(AggregateItem {
                    function,
                    argument: None,
                    distinct,
                    alias: if parser.keyword("AS") {
                        Some(parser.word()?)
                    } else {
                        None
                    },
                });
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            None
        } else {
            Some(parse_expression_tokens(parser)?)
        };
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after aggregate argument".into()));
        }
        let alias = if parser.keyword("AS") {
            Some(parser.word()?)
        } else {
            None
        };
        if distinct && argument.is_none() {
            return Err(ParseError("COUNT(DISTINCT) requires a column".into()));
        }
        items.push(AggregateItem {
            function,
            argument,
            distinct,
            alias,
        });
        if items.len() >= 128 && parser.tokens.get(parser.index) == Some(&Token::Comma) {
            return Err(ParseError("SELECT supports at most 128 aggregates".into()));
        }
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if items.len() < 2 {
        parser.index = start;
        return Ok(None);
    }
    Ok(Some(items))
}

#[cfg(test)]
mod tests {
    use crate::{Projection, parse_select};

    #[test]
    fn parses_mixed_aggregates_and_calculated_arguments() {
        let query = parse_select("SELECT SUM(width + 1), COUNT(*), AVG(width) FROM part").unwrap();
        let Projection::MultipleAggregates(items) = query.projection else {
            panic!("expected multiple aggregates");
        };
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].header(), "sum(width + 1)");
        assert_eq!(items[1].header(), "count(*)");
    }

    #[test]
    fn parses_distinct_counts_with_other_aggregates() {
        let query =
            parse_select("SELECT count(DISTINCT customer), count(*), sum(amount) FROM part")
                .unwrap();
        let Projection::MultipleAggregates(items) = query.projection else {
            panic!("expected multiple aggregates");
        };
        assert!(items[0].distinct);
        assert!(!items[1].distinct);
        assert_eq!(items[0].header(), "count(DISTINCT customer)");
        assert!(parse_select("SELECT sum(DISTINCT amount), count(*) FROM part").is_err());
    }

    #[test]
    fn parses_long_multi_aggregate_statements() {
        let expressions = (0..90)
            .map(|i| format!("sum(width + {i})"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT {expressions} FROM part");
        let Projection::MultipleAggregates(items) = parse_select(&sql).unwrap().projection else {
            panic!("expected multiple aggregates");
        };
        assert_eq!(items.len(), 90);
    }
}
