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
        Ok(Self { branches, indices })
    }

    pub(super) fn passes(&self, row: &[typed::Cell]) -> bool {
        self.branches.iter().any(|branch| {
            branch.iter().all(|condition| {
                let value = &row[condition.index];
                match &condition.test {
                    RowTest::Compare(operator, target) => {
                        typed::matches_comparison(value, target, *operator)
                    }
                    RowTest::Column(operator, other) => {
                        typed::matches_comparison(value, &row[*other], *operator)
                    }
                    RowTest::In(values, negated) => {
                        if values.is_empty() {
                            *negated
                        } else {
                            !matches!(value, typed::Cell::Null)
                                && (values.iter().any(|target| {
                                    typed::matches_comparison(value, target, Operator::Eq)
                                }) != *negated)
                        }
                    }
                    RowTest::IntegerSet(values, negated) => {
                        !matches!(value, typed::Cell::Null)
                            && (values.contains(&value.key()) != *negated)
                    }
                    RowTest::Null(negated) => matches!(value, typed::Cell::Null) != *negated,
                    RowTest::Like(pattern, insensitive, negated) => match value {
                        typed::Cell::Text(bytes) => {
                            typed::text_pattern_matches(bytes, pattern, *insensitive) != *negated
                        }
                        _ => false,
                    },
                }
            })
        })
    }
}
