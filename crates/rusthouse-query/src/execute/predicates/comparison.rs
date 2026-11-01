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
    let mut compiled = Vec::with_capacity(predicates.len());
    // Keep tiny predicates allocation-free. On wide AND/OR branches,
    // repeated Vec::position scans are quadratic in the number of columns.
    let mut indexed = (predicates.len() > 8).then(|| {
        let mut lookup = std::collections::HashMap::with_capacity(
            scan_columns.len().saturating_add(predicates.len()),
        );
        for (index, name) in scan_columns.iter().enumerate() {
            // Preserve the first occurrence if the caller projected a name
            // more than once; that is what the original linear scan chose.
            lookup.entry(name.clone()).or_insert(index);
        }
        lookup
    });
    for predicate in predicates {
        let column = predicate.column();
        let compiled_test = compile_predicate_test(predicate, column, &mut convert)?;
        let index = if let Some(lookup) = indexed.as_mut() {
            if let Some(&index) = lookup.get(column) {
                index
            } else {
                let index = scan_columns.len();
                scan_columns.push(column.to_owned());
                lookup.insert(column.to_owned(), index);
                index
            }
        } else {
            scan_column_index(scan_columns, column)
        };
        compiled.push(CompiledPredicate {
            index,
            test: compiled_test,
        });
    }
    Ok(compiled)
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

#[cfg(test)]
mod adaptive_predicate_projection_tests {
    use super::*;

    fn nullable_tests(columns: &[&str]) -> Vec<Predicate> {
        columns
            .iter()
            .map(|column| Predicate::IsNull {
                column: (*column).into(),
                negated: true,
            })
            .collect()
    }

    #[test]
    fn wide_predicate_set_uses_first_existing_projection_index() {
        let mut columns = vec!["existing".into(), "duplicate".into(), "existing".into()];
        let names = (0..48).map(|index| format!("col{index}")).collect::<Vec<_>>();
        let mut predicates = nullable_tests(&["existing", "duplicate"]);
        for name in &names {
            predicates.push(Predicate::IsNull {
                column: name.clone(),
                negated: true,
            });
        }
        predicates.extend(nullable_tests(&["col0", "col47", "existing"]));
        let compiled = compile_part_predicates(&predicates, &mut columns, |_, _| {
            panic!("IS NOT NULL does not require literal conversion")
        })
        .unwrap();
        assert_eq!(columns.len(), 51);
        assert_eq!(compiled.len(), 53);
        assert_eq!(compiled[0].index, 0);
        assert_eq!(compiled[1].index, 1);
        assert_eq!(compiled[2].index, 3);
        assert_eq!(compiled[49].index, 50);
        assert_eq!(compiled[50].index, 3);
        assert_eq!(compiled[51].index, 50);
        assert_eq!(compiled[52].index, 0);
    }

    #[test]
    fn small_predicate_sets_preserve_linear_projection_order() {
        let mut columns = vec!["a".into()];
        let predicates = nullable_tests(&["a", "b", "a", "c"]);
        let compiled = compile_part_predicates(&predicates, &mut columns, |_, _| {
            panic!("no conversion is needed")
        })
        .unwrap();
        assert_eq!(compiled.iter().map(|predicate| predicate.index).collect::<Vec<_>>(), vec![0, 1, 0, 2]);
        assert_eq!(columns, ["a", "b", "c"]);
    }
}
