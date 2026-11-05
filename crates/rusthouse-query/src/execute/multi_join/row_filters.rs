//! Bounded boolean predicates over a fully matched join row.
use super::*;
use crate::{Operator, Predicate};
use std::collections::HashSet;

enum RowTest {
    Compare(Operator, typed::Cell),
    Column(Operator, usize),
    In(Vec<typed::Cell>, bool),
    IntegerSet(HashSet<typed::Key>, bool),
    Null(bool),
    Like(typed::TextPattern, bool, bool),
}

struct RowCondition {
    index: usize,
    test: RowTest,
}

pub(super) struct RowFilters {
    branches: Vec<Vec<RowCondition>>,
    common_conditions: Vec<usize>,
    pub(super) indices: Vec<usize>,
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
        let common_conditions = predicates
            .first()
            .map(|first| {
                first
                    .iter()
                    .enumerate()
                    .filter_map(|(index, predicate)| {
                        predicates[1..]
                            .iter()
                            .all(|branch| branch.contains(predicate))
                            .then_some(index)
                    })
                    .collect()
            })
            .unwrap_or_default();
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
        Ok(Self {
            branches,
            common_conditions,
            indices,
        })
    }

    pub(super) fn passes(&self, row: &[typed::Cell]) -> bool {
        self.branches.iter().any(|branch| {
            branch
                .iter()
                .all(|condition| Self::condition_passes(condition, row, 0))
        })
    }

    /// Conditions shared by every OR branch can safely reject rows before
    /// an INNER join, even when the full filter spans multiple relations.
    pub(super) fn can_prefilter_relation(&self, start: usize, width: usize) -> bool {
        let Some(end) = start.checked_add(width) else {
            return false;
        };
        let Some(branch) = self.branches.first() else {
            return false;
        };
        !self.common_conditions.is_empty()
            && self.common_conditions.iter().all(|&index| {
                let condition = &branch[index];
                (start..end).contains(&condition.index)
                    && match &condition.test {
                        RowTest::Column(_, other) => (start..end).contains(other),
                        _ => true,
                    }
            })
    }

    pub(super) fn passes_relation(&self, row: &[typed::Cell], start: usize) -> bool {
        debug_assert!(self.can_prefilter_relation(start, row.len()));
        self.common_conditions
            .iter()
            .all(|&index| Self::condition_passes(&self.branches[0][index], row, start))
    }

    fn condition_passes(condition: &RowCondition, row: &[typed::Cell], offset: usize) -> bool {
        let index = condition.index - offset;
        let value = &row[index];
        match &condition.test {
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
            RowTest::Null(negated) => matches!(value, typed::Cell::Null) != *negated,
            RowTest::Like(pattern, insensitive, negated) => match value {
                typed::Cell::Text(bytes) => {
                    typed::text_pattern_matches(bytes, pattern, *insensitive) != *negated
                }
                _ => false,
            },
        }
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
                start: 0,
                rows: 1,
            },
            Relation {
                alias: "part".into(),
                names: vec!["p_name".into()],
                types: vec!["String".into()],
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
    fn prefilters_only_a_single_conjunctive_branch_local_to_one_relation() {
        let filters =
            RowFilters::bind(&[vec![like("part\0p_name", "%green%")]], &relations()).unwrap();
        assert!(filters.can_prefilter_relation(1, 1));
        assert!(!filters.can_prefilter_relation(0, 1));
        assert!(filters.passes_relation(&[typed::Cell::Text(b"green part".to_vec())], 1));
        assert!(!filters.passes_relation(&[typed::Cell::Text(b"blue part".to_vec())], 1));

        let branches = RowFilters::bind(
            &[
                vec![like("part\0p_name", "%green%")],
                vec![like("part\0p_name", "%blue%")],
            ],
            &relations(),
        )
        .unwrap();
        assert!(!branches.can_prefilter_relation(1, 1));
    }

    #[test]
    fn prefilter_matches_the_full_row_filter_for_a_single_relation() {
        let filters =
            RowFilters::bind(&[vec![like("part\0p_name", "%green%")]], &relations()).unwrap();
        for name in [b"green part".as_slice(), b"blue part"] {
            let local = [typed::Cell::Text(name.to_vec())];
            let full = [
                typed::Cell::Numeric(super::super::super::Number::Unsigned(1)),
                typed::Cell::Text(name.to_vec()),
            ];
            assert_eq!(filters.passes_relation(&local, 1), filters.passes(&full));
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

        assert!(filters.can_prefilter_relation(0, 1));
        assert!(!filters.can_prefilter_relation(1, 1));
        assert!(filters.passes_relation(
            &[typed::Cell::Numeric(super::super::super::Number::Unsigned(
                11
            ))],
            0
        ));
        assert!(!filters.passes_relation(
            &[typed::Cell::Numeric(super::super::super::Number::Unsigned(
                10
            ))],
            0
        ));

        let full_row = [
            typed::Cell::Numeric(super::super::super::Number::Unsigned(11)),
            typed::Cell::Text(b"blue part".to_vec()),
        ];
        assert!(filters.passes(&full_row));
        assert!(filters.passes_relation(&full_row[..1], 0));
    }
}
