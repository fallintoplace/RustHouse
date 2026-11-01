//! Shared SQL output adapters used by the CLI and HTTP server.

mod csv;
mod json_pretty;
mod json_validate;
mod tsv_escape;
mod without_header;

pub use csv::write_quoted_csv_field;
pub use json_pretty::write_pretty_json_row;
pub use json_validate::validate_json_object;
pub use tsv_escape::{decode_tsv_field, decode_tsv_field_into, validate_tsv_field};
pub use without_header::WithoutHeader;
