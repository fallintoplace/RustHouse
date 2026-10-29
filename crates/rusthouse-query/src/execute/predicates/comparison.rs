use super::*;

pub(super) fn compile_predicate_test(
    predicate: &Predicate,
    column: &str,
    convert: &mut impl FnMut(&str, &str) -> Result<Number, ExecutionError>,
) -> Result<CompiledTest, ExecutionError> {
    Ok(match predicate {
        Predicate::Comparison {
            operator, value, ..
        } => {
            let target = convert(column, value)?;
            if matches!(*operator, Operator::Modulo(_) | Operator::ModuloNe(_))
                && matches!(target, Number::Float(_))
            {
                return Err(ExecutionError::Invalid(
                    "modulo requires integer columns".into(),
                ));
            }
            CompiledTest::Comparison {
                operator: *operator,
                target,
            }
        }
        Predicate::In { values, .. } | Predicate::NotIn { values, .. } => compile_in_test(
            values
                .iter()
                .map(|value| convert(column, value))
                .collect::<Result<Vec<_>, _>>()?,
            matches!(predicate, Predicate::NotIn { .. }),
        ),
        Predicate::IsNull { negated, .. } => CompiledTest::Always(*negated),
        Predicate::DistinctFrom { value, negated, .. } => CompiledTest::DistinctFrom {
            target: convert(column, value)?,
            negated: *negated,
        },
        Predicate::Like { .. } => {
            return Err(ExecutionError::Invalid(
                "LIKE requires a String column".into(),
            ));
        }
        Predicate::NotBetween { lower, upper, .. } => CompiledTest::NotBetween {
            lower: convert(column, lower)?,
            upper: convert(column, upper)?,
        },
    })
}

fn scan_column_index(scan_columns: &mut Vec<String>, column: &str) -> usize {
    if let Some(index) = scan_columns
        .iter()
        .position(|candidate| candidate.as_str() == column)
    {
        index
    } else {
        scan_columns.push(column.to_owned());
        scan_columns.len() - 1
    }
}

pub(super) fn compile_part_predicates(
    predicates: &[Predicate],
    scan_columns: &mut Vec<String>,
    mut convert: impl FnMut(&str, &str) -> Result<Number, ExecutionError>,
) -> Result<Vec<CompiledPredicate>, ExecutionError> {
    predicates
        .iter()
        .map(|predicate| {
            let column = predicate.column();
            let test = compile_predicate_test(predicate, column, &mut convert)?;
            let index = scan_column_index(scan_columns, column);
            Ok(CompiledPredicate { index, test })
        })
        .collect()
}

pub(super) fn compare<T: PartialOrd>(value: T, target: T, operator: Operator) -> bool {
    match operator {
        Operator::Eq => value == target,
        Operator::Ne => value != target,
        Operator::Lt => value < target,
        Operator::Le => value <= target,
        Operator::Gt => value > target,
        Operator::Ge => value >= target,
        Operator::Modulo(_) | Operator::ModuloNe(_) => {
            unreachable!("modulo handled before comparison")
        }
    }
}
