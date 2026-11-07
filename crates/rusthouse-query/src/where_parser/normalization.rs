use super::*;

/// Normalize structurally identical predicates before execution. SQL filters
/// have no side effects, so exact duplicate AND/OR terms never change rows.
pub(super) fn branch_contains_all(superset: &[Predicate], subset: &[Predicate]) -> bool {
    subset.len() <= superset.len() && subset.iter().all(|predicate| superset.contains(predicate))
}

// Boolean absorption: A OR (A AND B) is A. All supported predicates are
// pure filters, so removing a stronger redundant branch preserves SQL NULL
// behavior as well as results. Retain the order of surviving branches.
pub(super) fn unique_branches(mut branches: Branches) -> Branches {
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

pub(super) fn check_expansion(branches: &Branches) -> Result<(), ParseError> {
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

pub(super) fn inverse_comparison_operator(operator: &Operator) -> Operator {
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

pub(super) fn inverse(predicate: &Predicate) -> Branches {
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
pub(super) fn negate(branches: Branches) -> Result<Branches, ParseError> {
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

