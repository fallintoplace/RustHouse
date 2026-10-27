//! Shared SQL output adapters used by the CLI and HTTP server.

mod tsv_escape;
mod without_header;

pub use tsv_escape::decode_tsv_field;
pub use without_header::WithoutHeader;
