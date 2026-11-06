use super::*;

#[derive(Clone)]
pub(in crate::execute) enum Cell {
    Numeric(Number),
    Text(Vec<u8>),
    Uuid(String),
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(in crate::execute) enum Key {
    Numeric(NumberKey),
    Text(Vec<u8>),
    Uuid(String),
    UuidBytes([u8; 16]),
    Null,
}

impl Cell {
    pub(in crate::execute) fn key(&self) -> Key {
        match self {
            Self::Numeric(value) => Key::Numeric(NumberKey::from(*value)),
            Self::Text(bytes) => Key::Text(bytes.clone()),
            Self::Uuid(value) => Key::Uuid(value.clone()),
            Self::Null => Key::Null,
        }
    }
}

/// Build a grouped hash key directly from a typed batch slot.
/// Avoid creating an owned Cell only to clone its String or UUID again.
pub(in crate::execute) fn key_at(column: &TypedColumn, row: usize) -> Key {
    match column {
        TypedColumn::NullableNumeric { nulls, .. } | TypedColumn::NullableText { nulls, .. }
            if nulls[row] != 0 =>
        {
            Key::Null
        }
        TypedColumn::Numeric(values) | TypedColumn::NullableNumeric { values, .. } => {
            Key::Numeric(NumberKey::from(number_at(values, row)))
        }
        TypedColumn::Text(values) | TypedColumn::NullableText { values, .. } => {
            Key::Text(values.get(row).unwrap().to_vec())
        }
        TypedColumn::Uuid(values) => Key::UuidBytes(*values[row].as_bytes()),
    }
}

/// Test the null bitmap without converting a value or allocating String/UUID cells.
pub(in crate::execute) fn is_null_at(column: &TypedColumn, row: usize) -> bool {
    match column {
        TypedColumn::NullableNumeric { nulls, .. } | TypedColumn::NullableText { nulls, .. } => {
            nulls[row] != 0
        }
        _ => false,
    }
}

pub(in crate::execute) fn cell(column: &TypedColumn, row: usize) -> Cell {
    match column {
        TypedColumn::NullableNumeric { nulls, .. } | TypedColumn::NullableText { nulls, .. }
            if nulls[row] != 0 =>
        {
            Cell::Null
        }
        TypedColumn::Numeric(values) | TypedColumn::NullableNumeric { values, .. } => {
            Cell::Numeric(number_at(values, row))
        }
        TypedColumn::Text(values) | TypedColumn::NullableText { values, .. } => {
            Cell::Text(values.get(row).unwrap().to_owned())
        }
        TypedColumn::Uuid(values) => Cell::Uuid(values[row].to_string()),
    }
}
