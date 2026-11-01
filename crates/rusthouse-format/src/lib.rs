//! Shared SQL output adapters used by the CLI and HTTP server.

mod csv;
mod json_pretty;
mod tsv_escape;
mod without_header;

pub use csv::write_quoted_csv_field;
pub use json_pretty::{write_pretty_json_row, write_pretty_json_row_prefixed};
pub use tsv_escape::{decode_tsv_field, decode_tsv_field_into, validate_tsv_field};
pub use without_header::WithoutHeader;
