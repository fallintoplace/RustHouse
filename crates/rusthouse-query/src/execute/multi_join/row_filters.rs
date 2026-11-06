//! Bounded boolean predicates over a fully matched join row.
use super::*;
use crate::{Operator, Predicate};
use std::collections::HashSet;

enum RowTest {
    Compare(Operator, typed::Cell),
    Column(Operator, usize),
    Expression(super::super::multi_aggregate::BoundExpr),
    In(Vec<typed::Cell>, bool),
    IntegerSet(HashSet<typed::Key>, bool),
    SubstringIn {
        start: usize,
        length: usize,
        values: Vec<Vec<u8>>,
        has_null: bool,
        negated: bool,
    },
    Null(bool),
    Like(typed::TextPattern, bool, bool),
}

struct RowCondition {
    index: usize,
    test: RowTest,
}

pub(super) struct RowFilters {
    branches: Vec<Vec<RowCondition>>,
    common_conditions: Vec<Vec<usize>>,
    pub(super) indices: Vec<usize>,
}

pub(super) struct RelationPrefilter {
    common: Vec<usize>,
    branches: Vec<Vec<usize>>,
}

fn numeric(kind: &str) -> bool {
    matches!(
        kind,
        "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UInt128"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "Int128"
            | "Float32"
            | "Float64"
            | "Date"
            | "Date32"
            | "DateTime"
    )
}

impl RowFilters {
    pub(super) fn bind(
        predicates: &[Vec<Predicate>],
        relations: &[Relation],
    ) -> Result<Self, ExecutionError> {
        let common_predicates = predicates.first().map_or_else(Vec::new, |first| {
            let mut common = Vec::new();
            for predicate in first {
                if predicates[1..]
                    .iter()
                    .all(|branch| branch.contains(predicate))
                    && !common.contains(predicate)
                {
                    common.push(predicate.clone());
                }
            }
            common
        });
        let mut indices = Vec::new();
        let resolve = |name: &str| {
            let (relation, name) = name.split_once('\0').unwrap_or(("", name));
            if relation.contains('\0') || name.contains('\0') {
                return Err(invalid("invalid qualified JOIN filter"));
            }
            column_index(
                relations,
                &JoinColumn {
                    relation: relation.to_owned(),
                    name: name.to_owned(),
                },
            )
        };
        let mut branches = Vec::with_capacity(predicates.len());
        for branch in predicates {
            let mut conditions = Vec::with_capacity(branch.len());
            for predicate in branch {
                if let Predicate::Expression(expression) = predicate {
                    let mut names = Vec::new();
                    expression.referenced_columns(&mut names);
                    let mut bindings = std::collections::HashMap::new();
                    for name in names {
                        let (relation, column) = name
                            .split_once('\0')
                            .map_or(("", name.as_str()), |(relation, column)| (relation, column));
                        let index = resolve(&format!("{relation}\0{column}"))?;
                        indices.push(index);
                        bindings.insert(name, index);
                    }
                    let bound =
                        super::super::multi_aggregate::BoundExpr::bind(expression, &bindings)?;
                    conditions.push(RowCondition {
                        index: 0,
                        test: RowTest::Expression(bound),
                    });
                    continue;
                }
                let index = resolve(predicate.column())?;
                indices.push(index);
                let kind = column_type(relations, index);
                let test = match predicate {
                    Predicate::Comparison {
                        operator, value, ..
                    } => {
                        if matches!(operator, Operator::Modulo(_) | Operator::ModuloNe(_)) {
                            return Err(invalid(
                                "modulo filters are not supported in JOIN scalar SUM",
                            ));
                        }
                        RowTest::Compare(*operator, typed::parse_literal(kind, value)?)
                    }
                    Predicate::ColumnComparison {
                        operator, other, ..
                    } => {
                        let other = resolve(other)?;
                        let left_kind = typed::base_type(kind);
                        let right_kind = typed::base_type(column_type(relations, other));
                        if left_kind != right_kind && !(numeric(left_kind) && numeric(right_kind)) {
                            return Err(invalid(
                                "JOIN column comparison requires compatible types",
                            ));
                        }
                        indices.push(other);
                        RowTest::Column(*operator, other)
                    }
                    Predicate::In { values, .. } | Predicate::NotIn { values, .. } => {
                        let values = values
                            .iter()
                            .map(|value| typed::parse_literal(kind, value))
                            .collect::<Result<Vec<_>, _>>()?;
                        let negated = matches!(predicate, Predicate::NotIn { .. });
                        let integer = matches!(
                            typed::base_type(kind),
                            "UInt8"
                                | "UInt16"
                                | "UInt32"
                                | "UInt64"
                                | "UInt128"
                                | "Int8"
                                | "Int16"
                                | "Int32"
                                | "Int64"
                                | "Int128"
                        );
                        if integer && values.len() >= 16 {
                            RowTest::IntegerSet(
                                values.iter().map(typed::Cell::key).collect(),
                                negated,
                            )
                        } else {
                            RowTest::In(values, negated)
                        }
                    }
                    Predicate::SubstringIn {
                        start,
                        length,
                        values,
                        has_null,
                        negated,
                        ..
                    } => {
                        if typed::base_type(kind) != "String" {
                            return Err(invalid("SUBSTRING IN requires a String column"));
                        }
                        if !(1..=64).contains(start) || !(1..=64).contains(length) {
                            return Err(invalid("SUBSTRING bounds must be in 1..64"));
                        }
                        RowTest::SubstringIn {
                            start: *start,
                            length: *length,
                            values: values
                                .iter()
                                .map(|value| value.as_bytes().to_vec())
                                .collect(),
                            has_null: *has_null,
                            negated: *negated,
                        }
                    }
                    Predicate::IsNull { negated, .. } => RowTest::Null(*negated),
                    Predicate::Like {
                        pattern,
                        case_insensitive,
                        negated,
                        ..
                    } => {
                        if typed::base_type(kind) != "String" {
                            return Err(invalid("JOIN LIKE requires a String column"));
                        }
                        RowTest::Like(
                            typed::compile_text_pattern(pattern)?,
                            *case_insensitive,
                            *negated,
                        )
                    }
                    _ => return Err(invalid("unsupported boolean predicate in JOIN scalar SUM")),
                };
                conditions.push(RowCondition { index, test });
            }
            branches.push(conditions);
        }
        indices.sort_unstable();
        indices.dedup();
        let common_conditions = predicates
            .iter()
            .map(|branch| {
                common_predicates
                    .iter()
                    .filter_map(|predicate| {
                        branch.iter().position(|candidate| candidate == predicate)
                    })
                    .collect()
            })
            .collect();
        Ok(Self {
            branches,
            common_conditions,
            indices,
        })
    }

    pub(super) fn passes(&self, row: &[typed::Cell]) -> Result<bool, ExecutionError> {
        for branch in &self.branches {
            let mut matches = true;
            for condition in branch {
                if !Self::condition_passes(condition, row, 0)? {
                    matches = false;
                    break;
                }
            }
            if matches {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn condition_is_local(condition: &RowCondition, start: usize, end: usize) -> bool {
        (start..end).contains(&condition.index)
            && match &condition.test {
                RowTest::Column(_, other) => (start..end).contains(other),
                RowTest::Expression(_) => false,
                _ => true,
            }
    }

    /// Return predicates that can reject rows from this relation alone.
    /// Every OR branch must contribute a local condition unless a common local
    /// predicate already rejects rows for every branch.
    pub(super) fn prefilter_conditions(
        &self,
        start: usize,
        width: usize,
    ) -> Option<RelationPrefilter> {
        let end = start.checked_add(width)?;
        let common = self
            .branches
            .first()?
            .iter()
            .enumerate()
            .filter_map(|(index, condition)| {
                (self
                    .common_conditions
                    .first()
                    .is_some_and(|conditions| conditions.contains(&index))
                    && Self::condition_is_local(condition, start, end))
                .then_some(index)
            })
            .collect::<Vec<_>>();
        let mut branches = self
            .branches
            .iter()
            .enumerate()
            .map(|(branch_index, branch)| {
                let common = self
                    .common_conditions
                    .get(branch_index)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                branch
                    .iter()
                    .enumerate()
                    .filter_map(|(index, condition)| {
                        (Self::condition_is_local(condition, start, end)
                            && !common.contains(&index))
                        .then_some(index)
                    })
                    .collect()
            })
            .collect::<Vec<_>>();
        if branches.iter().any(Vec::is_empty) {
            branches.clear();
        }
        if common.is_empty() && branches.is_empty() {
            return None;
        }
        Some(RelationPrefilter { common, branches })
    }

    pub(super) fn passes_relation(
        &self,
        row: &[typed::Cell],
        start: usize,
        conditions: &RelationPrefilter,
    ) -> bool {
        let Some(end) = start.checked_add(row.len()) else {
            return false;
        };
        debug_assert!(conditions.common.iter().all(|&index| {
            self.branches
                .first()
                .and_then(|branch| branch.get(index))
                .is_some_and(|condition| Self::condition_is_local(condition, start, end))
        }));
        debug_assert!(
            conditions.branches.is_empty() || {
                conditions.branches.len() == self.branches.len()
                    && conditions
                        .branches
                        .iter()
                        .zip(&self.branches)
                        .all(|(indices, branch)| {
                            !indices.is_empty()
                                && indices.iter().all(|&index| {
                                    branch.get(index).is_some_and(|condition| {
                                        Self::condition_is_local(condition, start, end)
                                    })
                                })
                        })
            }
        );
        let common_passes = conditions.common.iter().all(|&index| {
            self.branches
                .first()
                .and_then(|branch| branch.get(index))
                .is_some_and(|condition| {
                    Self::condition_passes(condition, row, start).is_ok_and(|passes| passes)
                })
        });
        common_passes
            && (conditions.branches.is_empty()
                || conditions
                    .branches
                    .iter()
                    .zip(&self.branches)
                    .any(|(indices, branch)| {
                        indices.iter().all(|&index| {
                            branch.get(index).is_some_and(|condition| {
                                Self::condition_passes(condition, row, start)
                                    .is_ok_and(|passes| passes)
                            })
                        })
                    }))
    }

    fn condition_passes(
        condition: &RowCondition,
        row: &[typed::Cell],
        offset: usize,
    ) -> Result<bool, ExecutionError> {
        if let RowTest::Expression(expression) = &condition.test {
            let value = super::super::multi_aggregate::eval_row_arithmetic(expression, row)?;
            return Ok(match value {
                Some(super::super::Number::Unsigned(value)) => value != 0,
                Some(super::super::Number::Signed(value)) => value != 0,
                Some(super::super::Number::Float(value)) => value != 0.0,
                None => false,
            });
        }
        let index = condition.index - offset;
        let value = &row[index];
        Ok(match &condition.test {
            RowTest::Compare(operator, target) => {
                typed::matches_comparison(value, target, *operator)
            }
            RowTest::Column(operator, other) => {
                typed::matches_comparison(value, &row[*other - offset], *operator)
            }
            RowTest::In(values, negated) => {
                if values.is_empty() {
                    *negated
                } else {
                    !matches!(value, typed::Cell::Null)
                        && (values
                            .iter()
                            .any(|target| typed::matches_comparison(value, target, Operator::Eq))
                            != *negated)
                }
            }
            RowTest::IntegerSet(values, negated) => {
                !matches!(value, typed::Cell::Null) && (values.contains(&value.key()) != *negated)
            }
            RowTest::SubstringIn {
                start,
                length,
                values,
                has_null,
                negated,
            } => {
                let typed::Cell::Text(bytes) = value else {
                    return Ok(false);
                };
                let from = (start - 1).min(bytes.len());
                let to = from.saturating_add(*length).min(bytes.len());
                let substring = &bytes[from..to];
                // Avoid matching a partial UTF-8 code point. The JOIN path
                // currently uses bounded byte offsets for SUBSTRING.
                if std::str::from_utf8(substring).is_err() {
                    return Ok(false);
                }
                let contains = values.iter().any(|target| target == substring);
                if contains {
                    !*negated
                } else {
                    !*has_null && *negated
                }
            }
            RowTest::Null(negated) => matches!(value, typed::Cell::Null) != *negated,
            RowTest::Like(pattern, insensitive, negated) => match value {
                typed::Cell::Text(bytes) => {
                    typed::text_pattern_matches(bytes, pattern, *insensitive) != *negated
                }
                _ => false,
            },
            RowTest::Expression(_) => unreachable!("expressions are handled above"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relations() -> Vec<Relation> {
        vec![
            Relation {
                alias: "orders".into(),
                names: vec!["id".into()],
                types: vec!["UInt64".into()],
                physical_columns: 1,
                virtual_columns: Vec::new(),
                start: 0,
                rows: 1,
            },
            Relation {
                alias: "part".into(),
                names: vec!["p_name".into()],
                types: vec!["String".into()],
                physical_columns: 1,
                virtual_columns: Vec::new(),
                start: 1,
                rows: 1,
            },
        ]
    }

    fn like(column: &str, pattern: &str) -> Predicate {
        Predicate::Like {
            column: column.into(),
            pattern: pattern.into(),
            case_insensitive: false,
            negated: false,
        }
    }

    fn comparison(column: &str, operator: Operator, value: &str) -> Predicate {
        Predicate::Comparison {
            column: column.into(),
            operator,
            value: value.into(),
        }
    }

    #[test]
    fn prefilters_local_conditions_from_every_or_branch() {
        let filters =
            RowFilters::bind(&[vec![like("part\0p_name", "%green%")]], &relations()).unwrap();
        assert!(filters.prefilter_conditions(1, 1).is_some());
        assert!(filters.prefilter_conditions(0, 1).is_none());
        let local = filters.prefilter_conditions(1, 1).unwrap();
        assert!(filters.passes_relation(&[typed::Cell::Text(b"green part".to_vec())], 1, &local));
        assert!(!filters.passes_relation(&[typed::Cell::Text(b"blue part".to_vec())], 1, &local));

        let branches = RowFilters::bind(
            &[
                vec![like("part\0p_name", "%green%")],
                vec![like("part\0p_name", "%blue%")],
            ],
            &relations(),
        )
        .unwrap();
        assert!(branches.prefilter_conditions(1, 1).is_some());
        let local = branches.prefilter_conditions(1, 1).unwrap();
        assert_eq!(local.branches.len(), 2);
        assert!(branches.passes_relation(&[typed::Cell::Text(b"green part".to_vec())], 1, &local,));
        assert!(branches.passes_relation(&[typed::Cell::Text(b"blue part".to_vec())], 1, &local,));
        assert!(!branches.passes_relation(&[typed::Cell::Text(b"red part".to_vec())], 1, &local,));
    }

    #[test]
    fn prefilter_matches_the_full_row_filter_for_a_single_relation() {
        let filters =
            RowFilters::bind(&[vec![like("part\0p_name", "%green%")]], &relations()).unwrap();
        let local_conditions = filters.prefilter_conditions(1, 1).unwrap();
        for name in [b"green part".as_slice(), b"blue part"] {
            let local_row = [typed::Cell::Text(name.to_vec())];
            let full = [
                typed::Cell::Numeric(super::super::super::Number::Unsigned(1)),
                typed::Cell::Text(name.to_vec()),
            ];
            assert_eq!(
                filters.passes_relation(&local_row, 1, &local_conditions),
                filters.passes(&full).unwrap()
            );
        }
    }

    #[test]
    fn prefilters_conditions_shared_by_every_or_branch() {
        let shared = comparison("orders\0id", Operator::Gt, "10");
        let filters = RowFilters::bind(
            &[
                vec![shared.clone(), like("part\0p_name", "%green%")],
                vec![shared, like("part\0p_name", "%blue%")],
            ],
            &relations(),
        )
        .unwrap();

        assert!(filters.prefilter_conditions(0, 1).is_some());
        assert!(filters.prefilter_conditions(1, 1).is_some());
        let local_conditions = filters.prefilter_conditions(0, 1).unwrap();
        assert_eq!(local_conditions.common.len(), 1);
        assert!(local_conditions.branches.is_empty());
        assert!(filters.passes_relation(
            &[typed::Cell::Numeric(super::super::super::Number::Unsigned(
                11
            ))],
            0,
            &local_conditions,
        ));
        assert!(!filters.passes_relation(
            &[typed::Cell::Numeric(super::super::super::Number::Unsigned(
                10
            ))],
            0,
            &local_conditions,
        ));

        let full_row = [
            typed::Cell::Numeric(super::super::super::Number::Unsigned(11)),
            typed::Cell::Text(b"blue part".to_vec()),
        ];
        assert!(filters.passes(&full_row).unwrap());
        assert!(filters.passes_relation(&full_row[..1], 0, &local_conditions));
    }

    #[test]
    fn prefilters_common_conditions_from_each_relation_in_a_conjunction() {
        let filters = RowFilters::bind(
            &[vec![
                comparison("orders\0id", Operator::Gt, "10"),
                like("part\0p_name", "%green%"),
            ]],
            &relations(),
        )
        .unwrap();
        let order_conditions = filters.prefilter_conditions(0, 1).unwrap();
        let part_conditions = filters.prefilter_conditions(1, 1).unwrap();

        assert_eq!(order_conditions.common.len(), 1);
        assert_eq!(part_conditions.common.len(), 1);
        assert!(filters.passes_relation(
            &[typed::Cell::Numeric(super::super::super::Number::Unsigned(
                11
            ))],
            0,
            &order_conditions,
        ));
        assert!(!filters.passes_relation(
            &[typed::Cell::Numeric(super::super::super::Number::Unsigned(
                10
            ))],
            0,
            &order_conditions,
        ));
        assert!(filters.passes_relation(
            &[typed::Cell::Text(b"green part".to_vec())],
            1,
            &part_conditions,
        ));
        assert!(!filters.passes_relation(
            &[typed::Cell::Text(b"blue part".to_vec())],
            1,
            &part_conditions,
        ));
    }

    #[test]
    fn prefilters_disjunctions_of_relation_local_conditions() {
        let shared = comparison("orders\0id", Operator::Gt, "0");
        let filters = RowFilters::bind(
            &[
                vec![
                    shared.clone(),
                    like("part\0p_name", "%green%"),
                    comparison("orders\0id", Operator::Gt, "10"),
                ],
                vec![
                    shared,
                    like("part\0p_name", "%blue%"),
                    comparison("orders\0id", Operator::Lt, "5"),
                ],
            ],
            &relations(),
        )
        .unwrap();
        let order_conditions = filters.prefilter_conditions(0, 1).unwrap();
        let part_conditions = filters.prefilter_conditions(1, 1).unwrap();

        assert_eq!(order_conditions.branches.len(), 2);
        assert_eq!(order_conditions.common.len(), 1);
        assert_eq!(part_conditions.branches.len(), 2);
        for order_id in [3, 8, 11] {
            let local = [typed::Cell::Numeric(super::super::super::Number::Unsigned(
                order_id,
            ))];
            assert_eq!(
                filters.passes_relation(&local, 0, &order_conditions),
                !(5..=10).contains(&order_id),
            );
        }
        assert!(filters.passes_relation(
            &[typed::Cell::Text(b"green part".to_vec())],
            1,
            &part_conditions,
        ));
        assert!(filters.passes_relation(
            &[typed::Cell::Text(b"blue part".to_vec())],
            1,
            &part_conditions,
        ));
        assert!(!filters.passes_relation(
            &[typed::Cell::Text(b"red part".to_vec())],
            1,
            &part_conditions,
        ));
        for order_id in [3, 8, 11] {
            for part_name in [b"green part".as_slice(), b"blue part", b"red part"] {
                let full = [
                    typed::Cell::Numeric(super::super::super::Number::Unsigned(order_id)),
                    typed::Cell::Text(part_name.to_vec()),
                ];
                if filters.passes(&full).unwrap() {
                    assert!(filters.passes_relation(&full[..1], 0, &order_conditions));
                    assert!(filters.passes_relation(&full[1..], 1, &part_conditions));
                }
            }
        }
    }

    #[test]
    fn does_not_prefilter_when_an_or_branch_has_no_local_condition() {
        let filters = RowFilters::bind(
            &[
                vec![comparison("orders\0id", Operator::Gt, "10")],
                vec![like("part\0p_name", "%green%")],
            ],
            &relations(),
        )
        .unwrap();

        assert!(filters.prefilter_conditions(0, 1).is_none());
        assert!(filters.prefilter_conditions(1, 1).is_none());
    }

    #[test]
    fn does_not_prefilter_column_comparisons_that_cross_relations() {
        let mut relations = relations();
        relations[1].names.push("id".into());
        relations[1].types.push("UInt64".into());
        let filters = RowFilters::bind(
            &[vec![Predicate::ColumnComparison {
                column: "orders\0id".into(),
                operator: Operator::Gt,
                other: "part\0id".into(),
            }]],
            &relations,
        )
        .unwrap();

        assert!(filters.prefilter_conditions(0, 1).is_none());
        assert!(filters.prefilter_conditions(1, 2).is_none());
    }
}
