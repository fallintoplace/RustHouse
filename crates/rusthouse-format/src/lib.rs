//! Shared SQL output adapters used by the CLI and HTTP server.

mod csv;
mod tsv_escape;
mod without_header;

pub use csv::write_quoted_csv_field;
pub use tsv_escape::decode_tsv_field;
pub use without_header::WithoutHeader;
