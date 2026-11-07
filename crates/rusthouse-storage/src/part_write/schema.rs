use crate::physical_type::{PhysicalType, numeric_scalar_width};

pub(super) use crate::physical_type::{
    base_type, map_string_value_type, numeric_array_inner_type,
};

pub(super) fn supported(kind: &str) -> bool {
    PhysicalType::parse(kind).is_some_and(PhysicalType::writable)
}

pub(super) fn fixed_width_bytes_per_row(data_type: &str) -> usize {
    let kind = base_type(data_type).0;
    match kind {
        "Date" => 2,
        "Date32" | "DateTime" => 4,
        "UUID" => 16,
        _ => numeric_scalar_width(kind).unwrap_or(0),
    }
}
