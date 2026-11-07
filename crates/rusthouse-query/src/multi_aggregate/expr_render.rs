use super::*;

pub(crate) fn render_expr(expr: &ScalarExpr) -> String {
    let mut output = String::new();
    render_expr_into(expr, &mut output);
    output
}

// Append directly to the final header instead of formatting a new String
// at every interior AST node. Long, bounded expressions are linear in output
// size rather than repeatedly copying their accumulated prefixes.
pub(super) fn render_expr_into(expr: &ScalarExpr, output: &mut String) {
    match expr {
        ScalarExpr::Column(name) | ScalarExpr::Decimal(name) => output.push_str(name),
        ScalarExpr::TextLiteral(value) => {
            output.push('\'');
            output.push_str(&value.replace('\'', "''"));
            output.push('\'');
        }
        ScalarExpr::TextConcat(arguments) => {
            output.push_str("concat(");
            for (index, argument) in arguments.iter().enumerate() {
                if index != 0 {
                    output.push_str(", ");
                }
                render_expr_into(argument, output);
            }
            output.push(')');
        }
        ScalarExpr::TextSubstring {
            value,
            start,
            length,
        } => {
            output.push_str("substring(");
            render_expr_into(value, output);
            write!(output, ", {start}").expect("writing to String cannot fail");
            if let Some(length) = length {
                write!(output, ", {length}").expect("writing to String cannot fail");
            }
            output.push(')');
        }
        ScalarExpr::Round { value, precision } => {
            output.push_str("round(");
            render_expr_into(value, output);
            write!(output, ", {precision})").expect("writing to String cannot fail");
        }
        ScalarExpr::TextLength(name) => {
            output.push_str("length(");
            output.push_str(name);
            output.push(')');
        }
        ScalarExpr::TextCompare {
            column,
            operator,
            value,
        } => {
            output.push_str(column);
            let symbol = match operator {
                crate::Operator::Eq => "=",
                crate::Operator::Ne => "!=",
                crate::Operator::Lt => "<",
                crate::Operator::Le => "<=",
                crate::Operator::Gt => ">",
                crate::Operator::Ge => ">=",
                _ => unreachable!("text comparisons require relational operators"),
            };
            write!(output, " {symbol} '").expect("writing to String cannot fail");
            output.push_str(&value.replace('\'', "''"));
            output.push('\'');
        }
        ScalarExpr::TextLike {
            column,
            pattern,
            negated,
            case_insensitive,
        } => {
            output.push_str(column);
            output.push_str(match (*negated, *case_insensitive) {
                (false, false) => " LIKE '",
                (false, true) => " ILIKE '",
                (true, false) => " NOT LIKE '",
                (true, true) => " NOT ILIKE '",
            });
            output.push_str(&pattern.replace('\'', "''"));
            output.push('\'');
        }
        ScalarExpr::IsNull { column, negated } => {
            output.push_str(column);
            output.push_str(if *negated { " IS NOT NULL" } else { " IS NULL" });
        }
        ScalarExpr::And(left, right) | ScalarExpr::Or(left, right) => {
            output.push('(');
            render_expr_into(left, output);
            output.push_str(if matches!(expr, ScalarExpr::And(..)) {
                " AND "
            } else {
                " OR "
            });
            render_expr_into(right, output);
            output.push(')');
        }
        ScalarExpr::Integer(number) => {
            write!(output, "{number}").expect("writing to String cannot fail");
        }
        ScalarExpr::Case {
            branches,
            else_expr,
        } => {
            output.push_str("CASE");
            for (condition, value) in branches {
                output.push_str(" WHEN ");
                render_expr_into(condition, output);
                output.push_str(" THEN ");
                render_expr_into(value, output);
            }
            if let Some(value) = else_expr {
                output.push_str(" ELSE ");
                render_expr_into(value, output);
            }
            output.push_str(" END");
        }
        ScalarExpr::Add(left, right) => {
            render_expr_into(left, output);
            output.push_str(" + ");
            render_expr_into(right, output);
        }
        ScalarExpr::Subtract(left, right) => {
            render_expr_into(left, output);
            output.push_str(" - ");
            let wrap = matches!(
                right.as_ref(),
                ScalarExpr::Add(..) | ScalarExpr::Subtract(..)
            );
            if wrap {
                output.push('(');
            }
            render_expr_into(right, output);
            if wrap {
                output.push(')');
            }
        }
        ScalarExpr::Divide(left, right) | ScalarExpr::Modulo(left, right) => {
            output.push('(');
            render_expr_into(left, output);
            output.push_str(if matches!(expr, ScalarExpr::Divide(..)) {
                ") / ("
            } else {
                ") % ("
            });
            render_expr_into(right, output);
            output.push(')');
        }
        ScalarExpr::Compare(left, operator, right) => {
            render_expr_into(left, output);
            let symbol = match operator {
                crate::Operator::Eq => "=",
                crate::Operator::Ne => "!=",
                crate::Operator::Lt => "<",
                crate::Operator::Le => "<=",
                crate::Operator::Gt => ">",
                crate::Operator::Ge => ">=",
                _ => unreachable!("comparisons only accept comparison operators"),
            };
            write!(output, " {symbol} ").expect("writing to String cannot fail");
            render_expr_into(right, output);
        }
        ScalarExpr::Multiply(left, right) => {
            for (index, operand) in [left.as_ref(), right.as_ref()].into_iter().enumerate() {
                if index != 0 {
                    output.push_str(" * ");
                }
                let wrap = matches!(
                    operand,
                    ScalarExpr::Add(..) | ScalarExpr::Subtract(..) | ScalarExpr::Compare(..)
                );
                if wrap {
                    output.push('(');
                }
                render_expr_into(operand, output);
                if wrap {
                    output.push(')');
                }
            }
        }
    }
}
