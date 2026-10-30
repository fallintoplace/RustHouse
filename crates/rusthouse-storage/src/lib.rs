mod arrays;
mod checksums;
mod columns;
mod compression;
mod database_lock;
mod database_table;
mod dates;
mod decimal;
pub mod distributed_activate;
pub mod distributed_atomic_copy;
pub mod distributed_capacity;
pub mod distributed_chunk_plan;
pub mod distributed_copy_diff;
pub mod distributed_crc32c;
pub mod distributed_inventory;
pub mod distributed_lineage;
pub mod distributed_membership;
pub mod distributed_object_key;
pub mod distributed_quorum;
pub mod distributed_range_read;
pub mod distributed_rendezvous;
pub mod distributed_repair_budget;
pub mod distributed_replica_order;
pub mod distributed_resume_map;
pub mod distributed_retention;
pub mod distributed_retry;
pub mod distributed_scrub;
pub mod distributed_snapshot_copy;
mod enums;
mod file_sizes;
mod fixed_strings;
mod fixed_width;
mod granules;
mod ip;
mod mark_validation;
mod marks;
mod metadata;
mod nested_arrays;
mod nullable;
mod nullable_arrays;
mod numeric;
mod part_scan;
mod part_verify;
mod part_write;
mod primary_index;
mod query_source;
mod scan_layout;
mod stream;
mod string_arrays;
mod strings;
mod typed_scan;
mod uuid;

pub use arrays::{
    ArrayOffsets, ArrayOffsetsEncoding, ArrayReadError, NumericArrayColumn, read_array_offsets,
    read_numeric_array_column,
};
pub use checksums::{
    ChecksumSizeSummary, FileChecksum, PartChecksums, UncompressedChecksum, read_checksums,
};
pub use columns::parse_columns;
pub use compression::{CompressedBlock, CompressedBlockReader, DecodedBlock};
pub use database_lock::{DatabaseLock, acquire_database_lock};
pub use database_table::resolve_database_table;
pub use dates::{format_date, format_date32, format_datetime, parse_date, parse_date32, parse_datetime};
pub use decimal::{DecimalColumn, DecimalReadError, read_decimal_column};
pub use enums::{EnumColumn, EnumReadError, read_enum_column};
pub use file_sizes::{
    FileSizeVerification, verify_part_file_hashes, verify_part_file_hashes_full,
    verify_part_file_sizes,
};
pub use fixed_strings::{
    FixedStringColumn, FixedStringReadError, read_fixed_string_granule, read_fixed_strings,
};
pub use granules::{read_mark_bytes, read_numeric_granule, read_u64_granule};
pub use ip::{read_ipv4_values, read_ipv6_values};
pub use mark_validation::validate_wide_marks;
pub use marks::{WideMark, WideMarkFormat, WideMarks, read_wide_marks};
pub use metadata::{Column, PartMetadata, inspect_part, read_part_row_count};
pub use nested_arrays::{
    NestedArrayReadError, NestedNumericArrayColumn, read_nested_numeric_array_column,
};
pub use nullable::{
    NullableColumn, NullableReadError, read_null_map, read_nullable_fixed_strings,
    read_nullable_legacy_strings, read_nullable_numeric_column, read_nullable_sized_strings,
};
pub use nullable_arrays::{
    NullableArrayReadError, NullableNumericArrayColumn, read_nullable_numeric_array_column,
};
pub use numeric::{NumericColumn, NumericReadError, read_numeric_column};
pub use part_scan::NumericPartReader;
pub use part_verify::{PartVerification, verify_wide_part};
pub use part_write::{PartCompression, write_wide_part, write_wide_part_with_compression};
pub use primary_index::{
    FixedPrimaryIndex, candidate_i64_mark_range, candidate_i64_mark_ranges,
    candidate_u64_mark_range, candidate_u64_mark_ranges, read_fixed_primary_index,
};
pub use query_source::{
    NumericQueryReader, inspect_query_source, query_parts, query_parts_with_metadata,
    resolve_active_snapshot,
};
pub use stream::UInt64ColumnReader;
pub use string_arrays::{
    StringArrayColumn, StringArrayReadError, read_legacy_string_array_column,
    read_sized_string_array_column,
};
pub use strings::{StringColumn, StringReadError, read_legacy_strings, read_sized_strings};
pub use typed_scan::{TypedColumn, TypedPartReader, TypedQueryReader};
pub use uuid::{UuidValue, read_uuid_values};

use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum PartError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    InvalidCount(String),
    InvalidChecksums {
        path: PathBuf,
        reason: &'static str,
    },
    InvalidColumns {
        line: usize,
        message: &'static str,
    },
    InvalidMarksLength {
        path: PathBuf,
        size: usize,
        record_size: usize,
    },
    EmptyMarksFile(PathBuf),
    InvalidMarkOffset {
        path: PathBuf,
        index: usize,
        reason: &'static str,
    },
    InvalidCompressedBlock {
        path: PathBuf,
        offset: u64,
        reason: &'static str,
    },
    InvalidPrimaryIndex {
        path: PathBuf,
        reason: &'static str,
    },
    InvalidColumnLength {
        path: PathBuf,
        width: usize,
        trailing_bytes: usize,
    },
    InvalidWideMark {
        compressed_offset: u64,
        decompressed_offset: u64,
        reason: &'static str,
    },
    InvalidGranuleIndex {
        index: usize,
        marks: usize,
    },
    InvalidScan(String),
    InvalidPart {
        path: PathBuf,
        reason: &'static str,
    },
    FixedStringRead(FixedStringReadError),
    NumericRead(NumericReadError),
    UnsupportedMarksFormat(PathBuf),
}

impl fmt::Display for PartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::InvalidCount(value) => write!(f, "invalid count.txt row count: {value:?}"),
            Self::InvalidChecksums { path, reason } => {
                write!(f, "{}: invalid checksums: {reason}", path.display())
            }
            Self::InvalidColumns { line, message } => {
                write!(f, "invalid columns.txt at line {line}: {message}")
            }
            Self::InvalidMarksLength {
                path,
                size,
                record_size,
            } => write!(
                f,
                "{}: {size} bytes is not a multiple of {record_size} bytes per mark",
                path.display()
            ),
            Self::EmptyMarksFile(path) => write!(f, "{}: empty marks file", path.display()),
            Self::InvalidMarkOffset {
                path,
                index,
                reason,
            } => write!(f, "{}: invalid mark {index}: {reason}", path.display()),
            Self::InvalidCompressedBlock {
                path,
                offset,
                reason,
            } => {
                write!(
                    f,
                    "{}: invalid compressed block at byte {offset}: {reason}",
                    path.display()
                )
            }
            Self::InvalidPrimaryIndex { path, reason } => {
                write!(f, "{}: invalid primary index: {reason}", path.display())
            }
            Self::InvalidColumnLength {
                path,
                width,
                trailing_bytes,
            } => write!(
                f,
                "{}: incomplete {width}-byte column value ({trailing_bytes} trailing bytes)",
                path.display()
            ),
            Self::InvalidWideMark {
                compressed_offset,
                decompressed_offset,
                reason,
            } => write!(
                f,
                "invalid wide mark (compressed={compressed_offset}, decompressed={decompressed_offset}): {reason}"
            ),
            Self::InvalidGranuleIndex { index, marks } => {
                write!(f, "granule index {index} is outside {marks} marks")
            }
            Self::InvalidScan(reason) => write!(f, "invalid numeric part scan: {reason}"),
            Self::InvalidPart { path, reason } => {
                write!(f, "{}: invalid wide part: {reason}", path.display())
            }
            Self::FixedStringRead(source) => write!(f, "{source}"),
            Self::NumericRead(source) => write!(f, "{source}"),
            Self::UnsupportedMarksFormat(path) => write!(
                f,
                "{}: only wide-part .mrk, .mrk2, .cmrk, and .cmrk2 files are supported",
                path.display()
            ),
        }
    }
}

impl Error for PartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::FixedStringRead(source) => Some(source),
            Self::NumericRead(source) => Some(source),
            _ => None,
        }
    }
}
