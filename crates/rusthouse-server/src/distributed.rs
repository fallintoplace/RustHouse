//! Experimental read-only scatter/gather SELECT against distinct local shards.
//! Unordered row projections and global pagination preserve configured peer order.

mod avg_merge;
mod dispatch;
mod distinct_merge;
mod multi_bitwise;
mod multi_count_column;
mod multi_countif;
mod multi_extrema;
mod multi_product;
mod multi_sum;
mod multi_sumsquares;
mod query_policy;
mod shard_decode;
use self::shard_decode::{ShardScalarRow, decode_scalar_row};
mod range_merge;
mod rms_merge;
mod sorted_merge;
mod text_merge;
mod uuid_extrema;
use self::distinct_merge::*;

use self::query_policy::*;
use self::sorted_merge::*;
use self::text_merge::*;

use crate::cluster::{Cluster, Peer};
use crate::remote;
use crate::{MAX_QUERY_BYTES, MAX_RESPONSE_BYTES, Response};
use rusthouse_query::{
    AggregateFunction, Having, Operator, Projection, Select, Source, parse_select,
};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

#[derive(Debug, PartialEq, Eq)]
enum MergeError {
    InvalidResponse,
    SchemaMismatch,
    ShardRowLimit,
    CoordinatorLimit,
    InvalidCount,
    InvalidDistinctCount,
    InvalidUniqState,
    InvalidGroupedCount,
    InvalidGroupedSum,
    InvalidGroupedProduct,
    InvalidGroupedBitwise,
    InvalidSortedScan,
    InvalidSum,
    InvalidProduct,
    InvalidBitwise,
    InvalidExtremum,
    CountOverflow,
    SumOverflow,
    ProductOverflow,
}

/// Parse only the canonical RustHouse peer capability document, never
/// trust matching substrings embedded in arbitrary JSON string values.
fn parse_peer_capabilities(data: &[u8]) -> Result<usize, String> {
    let mut cursor = SchemaCursor {
        bytes: data,
        index: 0,
    };
    cursor.byte(b'{')?;
    for (key, value) in [
        ("name", "RustHouse"),
        ("version", ""),
        ("source", "database"),
    ] {
        if cursor.string()? != key {
            return Err("unexpected peer capabilities field".into());
        }
        cursor.byte(b':')?;
        let found = cursor.string()?;
        if key == "version" {
            if found.is_empty()
                || found.len() > 64
                || !found
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
            {
                return Err("invalid peer version".into());
            }
        } else if found != value {
            return Err("peer is not a read-only RustHouse database server".into());
        }
        cursor.byte(b',')?;
    }
    if cursor.string()? != "read_only" {
        return Err("missing peer read-only flag".into());
    }
    cursor.byte(b':')?;
    if cursor.bytes.get(cursor.index..cursor.index + 4) != Some(b"true") {
        return Err("peer is not read-only".into());
    }
    cursor.index += 4;
    cursor.byte(b',')?;
    for (field, expected) in [
        ("max_query_bytes", MAX_QUERY_BYTES as u64),
        ("max_response_bytes", MAX_RESPONSE_BYTES as u64),
    ] {
        if cursor.string()? != field {
            return Err("unexpected peer capabilities field".into());
        }
        cursor.byte(b':')?;
        if cursor.unsigned_count()? != expected {
            return Err("incompatible peer capability limits".into());
        }
        cursor.byte(b',')?;
    }
    if cursor.string()? != "max_rows" {
        return Err("missing peer row limit".into());
    }
    cursor.byte(b':')?;
    let rows = cursor.unsigned_count()?;
    cursor.byte(b'}')?;
    cursor.whitespace();
    if cursor.index != data.len() || !(1..=250_000).contains(&rows) {
        return Err("unsupported peer row limit".into());
    }
    usize::try_from(rows).map_err(|_| "unsupported peer row limit".into())
}

fn shard_row_cap(peer: Peer) -> Result<usize, String> {
    let response = remote::capabilities(peer)?;
    if response.status != 200 || response.content_type != "application/json; charset=utf-8" {
        return Err("peer capabilities request failed".into());
    }
    parse_peer_capabilities(&response.body)
}

/// Validate the complete peer schema structure rather than accepting
/// column-looking substrings inside malformed JSON or unrelated fields.
/// Schema identifiers and physical types are ASCII and do not need JSON escapes.
struct PeerSchema {
    rows: u64,
    columns: Vec<(String, String)>,
}

struct SchemaCursor<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl SchemaCursor<'_> {
    fn whitespace(&mut self) {
        // RFC 8259 JSON whitespace is only SP, HTAB, LF, and CR.
        // is_ascii_whitespace also accepts VT and FF, which are not valid JSON.
        while self
            .bytes
            .get(self.index)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.index += 1;
        }
    }

    fn byte(&mut self, expected: u8) -> Result<(), String> {
        self.whitespace();
        if self.bytes.get(self.index) != Some(&expected) {
            return Err("invalid peer schema JSON".into());
        }
        self.index += 1;
        Ok(())
    }

    fn string(&mut self) -> Result<String, String> {
        self.byte(b'"')?;
        let start = self.index;
        while let Some(&byte) = self.bytes.get(self.index) {
            if byte == b'"' {
                let value = std::str::from_utf8(&self.bytes[start..self.index])
                    .map_err(|_| "invalid UTF-8 in peer schema")?;
                self.index += 1;
                return Ok(value.to_owned());
            }
            // The local schema producer uses validated unescaped identifiers
            // and type declarations; reject noncanonical encodings.
            if byte < b' ' || byte == b'\\' {
                return Err("invalid string in peer schema".into());
            }
            self.index += 1;
        }
        Err("unterminated peer schema string".into())
    }

    fn unsigned_count(&mut self) -> Result<u64, String> {
        self.whitespace();
        let start = self.index;
        while self.bytes.get(self.index).is_some_and(u8::is_ascii_digit) {
            self.index += 1;
        }
        let digits = &self.bytes[start..self.index];
        if digits.is_empty() || (digits.len() > 1 && digits[0] == b'0') {
            return Err("invalid peer row count".into());
        }
        std::str::from_utf8(digits)
            .map_err(|_| "invalid peer row count".to_owned())?
            .parse::<u64>()
            .map_err(|_| "invalid peer row count".to_owned())
    }
}

fn parse_peer_schema(bytes: &[u8]) -> Result<PeerSchema, String> {
    let mut cursor = SchemaCursor { bytes, index: 0 };
    cursor.byte(b'{')?;
    if cursor.string()? != "rows" {
        return Err("peer schema has no rows".into());
    }
    cursor.byte(b':')?;
    let rows = cursor.unsigned_count()?;
    cursor.byte(b',')?;
    if cursor.string()? != "columns" {
        return Err("peer schema has no columns".into());
    }
    cursor.byte(b':')?;
    cursor.byte(b'[')?;
    let mut columns = Vec::new();
    let mut names = HashSet::new();
    cursor.whitespace();
    if cursor.bytes.get(cursor.index) != Some(&b']') {
        loop {
            if columns.len() >= 1024 {
                return Err("peer schema has too many columns".into());
            }
            cursor.byte(b'{')?;
            if cursor.string()? != "name" {
                return Err("invalid peer column name key".into());
            }
            cursor.byte(b':')?;
            let name = cursor.string()?;
            cursor.byte(b',')?;
            if cursor.string()? != "type" {
                return Err("invalid peer column type key".into());
            }
            cursor.byte(b':')?;
            let kind = cursor.string()?;
            cursor.byte(b'}')?;
            if name.is_empty() || kind.is_empty() || !names.insert(name.clone()) {
                return Err("invalid or duplicate peer schema column".into());
            }
            columns.push((name, kind));
            cursor.whitespace();
            if cursor.bytes.get(cursor.index) != Some(&b',') {
                break;
            }
            cursor.index += 1;
        }
    }
    cursor.byte(b']')?;
    cursor.byte(b'}')?;
    cursor.whitespace();
    if cursor.index != bytes.len() || columns.is_empty() {
        return Err("trailing data or empty peer schema".into());
    }
    Ok(PeerSchema { rows, columns })
}

#[cfg(test)]
#[path = "distributed_tests/peer_schema_json_whitespace_tests.rs"]
mod peer_schema_json_whitespace_tests;

/// Resolve a column type from an independently validated peer schema.
fn shard_column_kind(response: &remote::PeerResponse, column: &str) -> Result<String, String> {
    let mut bytes = column.bytes();
    if !bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("distributed aggregate requires a simple column identifier".into());
    }
    if response.status != 200 || !response.content_type.starts_with("application/json") {
        return Err("peer schema lookup failed".into());
    }
    let schema = parse_peer_schema(&response.body)?;
    schema
        .columns
        .into_iter()
        .find_map(|(name, kind)| (name == column).then_some(kind))
        .ok_or_else(|| "aggregate column missing from peer schema".into())
}

fn shard_integer_kind(response: &remote::PeerResponse, column: &str) -> Result<String, String> {
    let kind = shard_column_kind(response, column)?;
    if !integer_type(&kind) {
        return Err("distributed SUM only supports exact integer columns".into());
    }
    Ok(kind)
}

/// Signatures ignore row counts but preserve physical type and column order.
fn schema_signature(response: &remote::PeerResponse) -> Result<Vec<u8>, String> {
    if response.status != 200 || response.content_type != "application/json; charset=utf-8" {
        return Err("peer schema request failed".into());
    }
    let schema = parse_peer_schema(&response.body)?;
    let mut signature = String::from("[");
    for (index, (name, kind)) in schema.columns.iter().enumerate() {
        if index > 0 {
            signature.push(',');
        }
        signature.push_str("{\"name\":\"");
        signature.push_str(name);
        signature.push_str("\",\"type\":\"");
        signature.push_str(kind);
        signature.push_str("\"}");
    }
    signature.push(']');
    Ok(signature.into_bytes())
}

/// Fetch independent control-plane metadata concurrently per shard, rather
/// than paying two sequential network round trips for every distributed scan.
/// Both scoped workers always join before a query can start.
fn preflight_shard(peer: Peer, table: &str) -> Result<(usize, remote::PeerResponse), String> {
    let (cap, schema) = std::thread::scope(|scope| {
        // One scoped worker is enough to overlap both metadata requests.
        // If the OS refuses to create it, perform the same checks serially.
        let cap_worker = std::thread::Builder::new().spawn_scoped(scope, || shard_row_cap(peer));
        let schema = remote::schema(peer, table);
        let cap = match cap_worker {
            Ok(worker) => worker
                .join()
                .unwrap_or_else(|_| Err("peer capabilities panicked".into())),
            Err(_) => shard_row_cap(peer),
        };
        (cap, schema)
    });
    Ok((cap?, schema?))
}

struct ShardResponse {
    cap: usize,
    body: Vec<u8>,
    kind: Option<String>,
    key_kind: Option<String>,
    schema: Vec<u8>,
}

// Count retained peer bodies across one coordinator query, not per peer.
// A failed reservation never raises the accounted total.
const MAX_SHARD_INPUT_BYTES: usize = 64 * 1024 * 1024;

fn reserve_shard_bytes(used: &AtomicUsize, amount: usize) -> Result<(), String> {
    let mut current = used.load(AtomicOrdering::Acquire);
    loop {
        let next = current
            .checked_add(amount)
            .filter(|&total| total <= MAX_SHARD_INPUT_BYTES)
            .ok_or("distributed shard input exceeds 64 MiB")?;
        match used.compare_exchange_weak(
            current,
            next,
            AtomicOrdering::AcqRel,
            AtomicOrdering::Acquire,
        ) {
            Ok(_) => return Ok(()),
            Err(observed) => current = observed,
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Each input is part of the shard request or its shared limits."
)]
fn read_shard(
    peer: Peer,
    sql: &str,
    sum_schema: Option<(&str, &str)>,
    group_schema: Option<(&str, &str)>,
    table: &str,
    sorted_prefix_cap: Option<usize>,
    allow_nonnumeric_count: bool,
    allow_uuid_extremum: bool,
    used_bytes: &AtomicUsize,
) -> Result<ShardResponse, String> {
    let (cap, schema_response) = preflight_shard(peer, table)?;
    if sorted_prefix_cap.is_some_and(|prefix| prefix >= cap) {
        return Err("peer row cap must exceed distributed top-K prefix".into());
    }
    let schema = schema_signature(&schema_response)?;
    let kind = sum_schema
        .map(|(_, column)| {
            if sorted_prefix_cap.is_some() || allow_nonnumeric_count {
                shard_column_kind(&schema_response, column)
            } else if allow_uuid_extremum {
                let kind = shard_column_kind(&schema_response, column)?;
                if integer_type(&kind) || matches!(kind.as_str(), "UUID" | "Nullable(UUID)") {
                    Ok(kind)
                } else {
                    Err("distributed MIN/MAX only supports integer or UUID columns".into())
                }
            } else {
                shard_integer_kind(&schema_response, column)
            }
        })
        .transpose()?;
    let key_kind = group_schema
        .map(|(_, column)| {
            let kind = shard_column_kind(&schema_response, column)?;
            if !group_key_type(&kind) {
                return Err("distributed GROUP BY SUM does not support this key type".to_owned());
            }
            Ok(kind)
        })
        .transpose()?;
    let result = remote::query(peer, sql)?;
    if result.status != 200 {
        return Err(format!("peer returned HTTP {}", result.status));
    }
    if result.content_type != "text/tab-separated-values; charset=utf-8" {
        return Err("peer did not return TSV".into());
    }
    // Check for concurrent schema changes and row-count changes around the
    // peer query. This catches many torn reads, but is not an atomic
    // multi-shard snapshot or protection against same-row-count rewrites.
    let after_response = remote::schema(peer, table)?;
    let after = schema_signature(&after_response)?;
    if after != schema
        || parse_peer_schema(&schema_response.body)?.rows
            != parse_peer_schema(&after_response.body)?.rows
    {
        return Err("peer table changed while distributed query was running".into());
    }
    reserve_shard_bytes(used_bytes, result.body.len())?;
    Ok(ShardResponse {
        cap,
        body: result.body,
        kind,
        key_kind,
        schema,
    })
}

/// Merge only identical TSV headers and whole, newline-terminated rows.
/// Refuse capped shard results instead of silently returning an incomplete scan.
fn merge_tsv(shards: &[(usize, Vec<u8>)], max_rows: usize) -> Result<Vec<u8>, MergeError> {
    let mut chunks: Vec<&[u8]> = Vec::with_capacity(shards.len());
    let mut header: Option<&[u8]> = None;
    let mut total_rows = 0usize;
    let mut total_bytes = 0usize;
    // Validate every shard before allocating or copying the merged response.
    for (cap, body) in shards {
        if !body.ends_with(b"\n") {
            return Err(MergeError::InvalidResponse);
        }
        let header_end = body
            .iter()
            .position(|&b| b == b'\n')
            .ok_or(MergeError::InvalidResponse)?;
        let shard_header = &body[..=header_end];
        if shard_header.len() <= 1 {
            return Err(MergeError::InvalidResponse);
        }
        if let Some(expected) = header {
            if expected != shard_header {
                return Err(MergeError::SchemaMismatch);
            }
        } else {
            header = Some(shard_header);
        }
        let rows = &body[header_end + 1..];
        let required_tabs = shard_header.iter().filter(|&&byte| byte == b'\t').count();
        let mut shard_rows = 0usize;
        let mut row_tabs = 0usize;
        let mut field_start = 0usize;
        // Validate every wire field before forwarding rows, even after pagination.
        for (position, &byte) in rows.iter().enumerate() {
            if byte != b'\t' && byte != b'\n' {
                continue;
            }
            let field = &rows[field_start..position];
            // A bare \N is the TSV null marker, not an escape sequence.
            if field != br"\N" {
                rusthouse_format::validate_tsv_field(field)
                    .map_err(|_| MergeError::InvalidResponse)?;
            }
            field_start = position + 1;
            if byte == b'\t' {
                row_tabs += 1;
            } else {
                if row_tabs != required_tabs {
                    return Err(MergeError::InvalidResponse);
                }
                row_tabs = 0;
                shard_rows += 1;
            }
        }
        if *cap == 0 || shard_rows >= *cap {
            return Err(MergeError::ShardRowLimit);
        }
        total_rows = total_rows
            .checked_add(shard_rows)
            .ok_or(MergeError::CoordinatorLimit)?;
        if total_rows > max_rows {
            return Err(MergeError::CoordinatorLimit);
        }
        let added = if chunks.is_empty() {
            body.as_slice()
        } else {
            rows
        };
        total_bytes = total_bytes
            .checked_add(added.len())
            .filter(|&size| size <= MAX_RESPONSE_BYTES)
            .ok_or(MergeError::CoordinatorLimit)?;
        chunks.push(added);
    }
    if header.is_none() {
        return Err(MergeError::InvalidResponse);
    }
    let mut output = Vec::with_capacity(total_bytes);
    for bytes in chunks {
        output.extend_from_slice(bytes);
    }
    Ok(output)
}

/// Apply global SELECT DISTINCT over complete validated shard rows. Retain
/// borrowed slices into bounded peer bodies rather than cloning row payloads.
/// No shard-local pagination is allowed: it could hide globally unique rows.
fn merge_distinct_tsv(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    offset: usize,
    limit: Option<usize>,
) -> Result<Vec<u8>, MergeError> {
    const MAX_DISTINCT_ROWS: usize = 250_000;
    let mut header: Option<&[u8]> = None;
    // Deduplicate by logical TSV field values, not by one possible on-wire
    // escape spelling. The first encountered wire row remains the output.
    let mut unique = HashSet::<Vec<NormalizedGroupKey<'_>>>::new();
    let mut output = Vec::new();
    let mut returned = 0usize;
    for (cap, body) in shards {
        if !body.ends_with(b"\n") {
            return Err(MergeError::InvalidResponse);
        }
        let header_end = body
            .iter()
            .position(|&b| b == b'\n')
            .ok_or(MergeError::InvalidResponse)?;
        let shard_header = &body[..=header_end];
        if shard_header.len() <= 1 {
            return Err(MergeError::InvalidResponse);
        }
        if let Some(expected) = header {
            if expected != shard_header {
                return Err(MergeError::SchemaMismatch);
            }
        } else {
            header = Some(shard_header);
            output.extend_from_slice(shard_header);
        }
        let required_tabs = shard_header.iter().filter(|&&byte| byte == b'\t').count();
        let rows = &body[header_end + 1..];
        let mut shard_rows = 0usize;
        if !rows.is_empty() {
            for row in rows[..rows.len() - 1].split(|&byte| byte == b'\n') {
                if row.iter().filter(|&&byte| byte == b'\t').count() != required_tabs {
                    return Err(MergeError::InvalidResponse);
                }
                let normalized = row
                    .split(|&byte| byte == b'\t')
                    .map(|field| {
                        normalized_group_key(field).map_err(|_| MergeError::InvalidResponse)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                shard_rows = shard_rows
                    .checked_add(1)
                    .ok_or(MergeError::CoordinatorLimit)?;
                if unique.insert(normalized) {
                    if unique.len() > MAX_DISTINCT_ROWS {
                        return Err(MergeError::CoordinatorLimit);
                    }
                    if unique.len() > offset && limit.is_none_or(|limit| returned < limit) {
                        returned = returned
                            .checked_add(1)
                            .filter(|&count| count <= max_rows)
                            .ok_or(MergeError::CoordinatorLimit)?;
                        let next = output
                            .len()
                            .checked_add(row.len() + 1)
                            .filter(|&size| size <= MAX_RESPONSE_BYTES)
                            .ok_or(MergeError::CoordinatorLimit)?;
                        output.reserve(next - output.len());
                        output.extend_from_slice(row);
                        output.push(b'\n');
                    }
                }
            }
        }
        if *cap == 0 || shard_rows >= *cap {
            return Err(MergeError::ShardRowLimit);
        }
    }
    if header.is_none() {
        return Err(MergeError::InvalidResponse);
    }
    Ok(output)
}

/// Apply LIMIT/OFFSET only after validating every shard's full result and
/// advertised cap. Rows are paged in stable configured-peer order.
/// Page complete shard TSV results in a single row-boundary pass per shard.
/// Retain one contiguous byte slice per shard rather than splitting and
/// copying each matching row. All shard rows and caps are still validated,
/// even after the global page is full.
fn merge_tsv_page(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    offset: usize,
    limit: Option<usize>,
) -> Result<Vec<u8>, MergeError> {
    let mut output = Vec::new();
    let mut header: Option<&[u8]> = None;
    let mut seen_rows = 0usize;
    let mut returned_rows = 0usize;
    for (cap, body) in shards {
        if !body.ends_with(b"\n") {
            return Err(MergeError::InvalidResponse);
        }
        let header_end = body
            .iter()
            .position(|&b| b == b'\n')
            .ok_or(MergeError::InvalidResponse)?;
        let shard_header = &body[..=header_end];
        if shard_header.len() <= 1 {
            return Err(MergeError::InvalidResponse);
        }
        if let Some(expected) = header {
            if expected != shard_header {
                return Err(MergeError::SchemaMismatch);
            }
        } else {
            header = Some(shard_header);
            output.extend_from_slice(shard_header);
        }

        let rows = &body[header_end + 1..];
        let required_tabs = shard_header.iter().filter(|&&byte| byte == b'\t').count();
        let local_skip = offset.saturating_sub(seen_rows);
        let remaining = limit
            .map(|limit| limit.saturating_sub(returned_rows))
            .unwrap_or(usize::MAX);
        let mut shard_rows = 0usize;
        let mut row_start = 0usize;
        let mut row_tabs = 0usize;
        let mut field_start = 0usize;
        let mut selected_start = None;
        let mut selected_end = 0usize;
        for (position, &byte) in rows.iter().enumerate() {
            if byte != b'\t' && byte != b'\n' {
                continue;
            }
            let field = &rows[field_start..position];
            // A bare \N is the TSV null marker, not an escape sequence.
            if field != br"\N" {
                rusthouse_format::validate_tsv_field(field)
                    .map_err(|_| MergeError::InvalidResponse)?;
            }
            field_start = position + 1;
            if byte == b'\t' {
                row_tabs += 1;
                continue;
            }
            if row_tabs != required_tabs {
                return Err(MergeError::InvalidResponse);
            }
            row_tabs = 0;
            // TSV data encodes embedded newlines as escapes, so all literal
            // newlines delimit rows. The selected page is a contiguous range.
            if shard_rows >= local_skip && shard_rows - local_skip < remaining {
                if selected_start.is_none() {
                    selected_start = Some(row_start);
                }
                selected_end = position + 1;
            }
            shard_rows = shard_rows
                .checked_add(1)
                .ok_or(MergeError::CoordinatorLimit)?;
            row_start = position + 1;
        }
        if *cap == 0 || shard_rows >= *cap {
            return Err(MergeError::ShardRowLimit);
        }
        seen_rows = seen_rows
            .checked_add(shard_rows)
            .ok_or(MergeError::CoordinatorLimit)?;
        if let Some(start) = selected_start {
            let appended = shard_rows.saturating_sub(local_skip).min(remaining);
            returned_rows = returned_rows
                .checked_add(appended)
                .filter(|&count| count <= max_rows)
                .ok_or(MergeError::CoordinatorLimit)?;
            let bytes = &rows[start..selected_end];
            if output
                .len()
                .checked_add(bytes.len())
                .is_none_or(|len| len > MAX_RESPONSE_BYTES)
            {
                return Err(MergeError::CoordinatorLimit);
            }
            output.extend_from_slice(bytes);
        }
    }
    if header.is_none() {
        return Err(MergeError::InvalidResponse);
    }
    Ok(output)
}

/// A count shard must return exactly one non-negative integer in one TSV row.
/// Summation checks overflow and never applies the coordinator's row cap to counts.
fn sum_shard_counts(shards: &[(usize, Vec<u8>)]) -> Result<Vec<u8>, MergeError> {
    let mut expected_header: Option<&[u8]> = None;
    let mut sum = 0u64;
    for (_, body) in shards {
        let ShardScalarRow {
            header,
            value,
        } = decode_scalar_row(body, MergeError::InvalidCount)?;
        if let Some(expected) = expected_header {
            if expected != header {
                return Err(MergeError::SchemaMismatch);
            }
        } else {
            expected_header = Some(header);
        }
        if value.is_empty() || !value.iter().all(|b| b.is_ascii_digit()) {
            return Err(MergeError::InvalidCount);
        }
        let count = std::str::from_utf8(value)
            .map_err(|_| MergeError::InvalidCount)?
            .parse::<u64>()
            .map_err(|_| MergeError::CountOverflow)?;
        sum = sum.checked_add(count).ok_or(MergeError::CountOverflow)?;
    }
    let header = expected_header.ok_or(MergeError::InvalidCount)?;
    let mut result = header.to_vec();
    result.extend_from_slice(format!("{sum}\n").as_bytes());
    Ok(result)
}

/// Reduce SUM only after all peers agree on the exact integer column type.
/// NULL represents an empty shard rather than zero or a failed parse.
fn merge_integer_sums(shards: &[(usize, Vec<u8>)], kind: &str) -> Result<Vec<u8>, MergeError> {
    let mut expected_header: Option<&[u8]> = None;
    // Signed shard totals may cancel. Peer-order accumulation could overflow
    // even when the exact global answer fits i128.
    let mut positive_parts = Vec::new();
    let mut negative_parts = Vec::new();
    let mut unsigned_total = 0u128;
    let mut seen = false;
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");

    for (_, body) in shards {
        let ShardScalarRow {
            header,
            value,
        } = decode_scalar_row(body, MergeError::InvalidSum)?;
        if let Some(expected) = expected_header {
            if expected != header {
                return Err(MergeError::SchemaMismatch);
            }
        } else {
            expected_header = Some(header);
        }
        if value == b"NULL" {
            continue;
        }
        let digits = std::str::from_utf8(value).map_err(|_| MergeError::InvalidSum)?;
        if digits.is_empty() {
            return Err(MergeError::InvalidSum);
        }
        if unsigned {
            let partial = digits.parse::<u128>().map_err(|_| MergeError::InvalidSum)?;
            unsigned_total = unsigned_total
                .checked_add(partial)
                .ok_or(MergeError::SumOverflow)?;
        } else {
            let partial = digits.parse::<i128>().map_err(|_| MergeError::InvalidSum)?;
            if partial >= 0 {
                positive_parts.push(partial);
            } else {
                negative_parts.push(partial);
            }
        }
        seen = true;
    }
    // Alternate signs when possible. A partial sum outside i128 range is
    // avoided if the final exact sum is representable, without a BigInt.
    let mut signed_total = 0i128;
    while !positive_parts.is_empty() || !negative_parts.is_empty() {
        let next = if signed_total > 0 && !negative_parts.is_empty() {
            negative_parts.pop().expect("nonempty")
        } else if signed_total < 0 && !positive_parts.is_empty() {
            positive_parts.pop().expect("nonempty")
        } else if let Some(positive) = positive_parts.pop() {
            positive
        } else {
            negative_parts.pop().expect("nonempty")
        };
        signed_total = signed_total
            .checked_add(next)
            .ok_or(MergeError::SumOverflow)?;
    }
    let header = expected_header.ok_or(MergeError::InvalidSum)?;
    let result = if !seen {
        "NULL".to_owned()
    } else if unsigned {
        unsigned_total.to_string()
    } else {
        signed_total.to_string()
    };
    let mut output = header.to_vec();
    output.extend_from_slice(result.as_bytes());
    output.push(b'\n');
    Ok(output)
}

/// Reject a corrupted peer MIN/MAX value that cannot exist in its schema.
/// Unlike SUM, MIN/MAX must always retain the original physical width.
fn extrema_fits_declared_type(kind: &str, raw: &str) -> bool {
    let base = kind
        .strip_prefix("Nullable(")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(kind);
    let (unsigned, bits) = match base {
        "UInt8" => (true, 8),
        "UInt16" => (true, 16),
        "UInt32" => (true, 32),
        "UInt64" => (true, 64),
        "UInt128" => (true, 128),
        "Int8" => (false, 8),
        "Int16" => (false, 16),
        "Int32" => (false, 32),
        "Int64" => (false, 64),
        "Int128" => (false, 128),
        _ => return false,
    };
    if unsigned {
        raw.parse::<u128>()
            .is_ok_and(|value| bits == 128 || value < (1u128 << bits))
    } else {
        raw.parse::<i128>().is_ok_and(|value| {
            bits == 128 || (value >= -(1i128 << (bits - 1)) && value < (1i128 << (bits - 1)))
        })
    }
}

/// Merge bitwise aggregate states. An empty shard has no identity value.
fn merge_integer_bitwise(
    shards: &[(usize, Vec<u8>)],
    kind: &str,
    function: AggregateFunction,
) -> Result<Vec<u8>, MergeError> {
    if !matches!(
        function,
        AggregateFunction::GroupBitAnd
            | AggregateFunction::GroupBitOr
            | AggregateFunction::GroupBitXor
    ) {
        return Err(MergeError::InvalidBitwise);
    }
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");
    let mut header: Option<&[u8]> = None;
    let mut signed: Option<i128> = None;
    let mut unsigned_value: Option<u128> = None;
    for (_, body) in shards {
        let ShardScalarRow {
            header: current_header,
            value,
        } = decode_scalar_row(body, MergeError::InvalidBitwise)?;
        if header.is_some_and(|expected| expected != current_header) {
            return Err(MergeError::SchemaMismatch);
        }
        header = Some(current_header);
        if value == b"NULL" {
            continue;
        }
        let raw = std::str::from_utf8(value).map_err(|_| MergeError::InvalidBitwise)?;
        if unsigned {
            let next = raw
                .parse::<u128>()
                .map_err(|_| MergeError::InvalidBitwise)?;
            unsigned_value = Some(match unsigned_value {
                None => next,
                Some(prior) => match function {
                    AggregateFunction::GroupBitAnd => prior & next,
                    AggregateFunction::GroupBitOr => prior | next,
                    AggregateFunction::GroupBitXor => prior ^ next,
                    _ => unreachable!("validated bitwise function"),
                },
            });
        } else {
            let next = raw
                .parse::<i128>()
                .map_err(|_| MergeError::InvalidBitwise)?;
            signed = Some(match signed {
                None => next,
                Some(prior) => match function {
                    AggregateFunction::GroupBitAnd => prior & next,
                    AggregateFunction::GroupBitOr => prior | next,
                    AggregateFunction::GroupBitXor => prior ^ next,
                    _ => unreachable!("validated bitwise function"),
                },
            });
        }
    }
    let mut output = header.ok_or(MergeError::InvalidBitwise)?.to_vec();
    let rendered = if unsigned {
        unsigned_value.map(|value| value.to_string())
    } else {
        signed.map(|value| value.to_string())
    }
    .unwrap_or_else(|| "NULL".to_owned());
    output.extend_from_slice(rendered.as_bytes());
    output.push(b'\n');
    Ok(output)
}

/// PRODUCT is decomposable over disjoint shard partials. Empty shards do not
/// contribute an identity, and overflow must fail rather than wrap.
fn merge_integer_products(shards: &[(usize, Vec<u8>)], kind: &str) -> Result<Vec<u8>, MergeError> {
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");
    let mut expected_header: Option<&[u8]> = None;
    let mut signed: Option<i128> = None;
    let mut unsigned_value: Option<u128> = None;
    for (_, body) in shards {
        let ShardScalarRow {
            header,
            value,
        } = decode_scalar_row(body, MergeError::InvalidProduct)?;
        if expected_header.is_some_and(|expected| expected != header) {
            return Err(MergeError::SchemaMismatch);
        }
        expected_header = Some(header);
        if value == b"NULL" {
            continue;
        }
        let digits = std::str::from_utf8(value).map_err(|_| MergeError::InvalidProduct)?;
        if unsigned {
            let next = digits
                .parse::<u128>()
                .map_err(|_| MergeError::InvalidProduct)?;
            unsigned_value = Some(match unsigned_value {
                None => next,
                Some(prior) => prior.checked_mul(next).ok_or(MergeError::ProductOverflow)?,
            });
        } else {
            let next = digits
                .parse::<i128>()
                .map_err(|_| MergeError::InvalidProduct)?;
            signed = Some(match signed {
                None => next,
                Some(prior) => prior.checked_mul(next).ok_or(MergeError::ProductOverflow)?,
            });
        }
    }
    let mut output = expected_header.ok_or(MergeError::InvalidProduct)?.to_vec();
    let rendered = if unsigned {
        unsigned_value.map(|value| value.to_string())
    } else {
        signed.map(|value| value.to_string())
    }
    .unwrap_or_else(|| "NULL".to_owned());
    output.extend_from_slice(rendered.as_bytes());
    output.push(b'\n');
    Ok(output)
}

fn merge_integer_extrema(
    shards: &[(usize, Vec<u8>)],
    kind: &str,
    function: AggregateFunction,
) -> Result<Vec<u8>, MergeError> {
    if !matches!(function, AggregateFunction::Min | AggregateFunction::Max) {
        return Err(MergeError::InvalidExtremum);
    }
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");
    let mut expected_header: Option<&[u8]> = None;
    let mut signed: Option<i128> = None;
    let mut unsigned_value: Option<u128> = None;
    for (_, body) in shards {
        let ShardScalarRow {
            header,
            value,
        } = decode_scalar_row(body, MergeError::InvalidExtremum)?;
        if let Some(expected) = expected_header {
            if expected != header {
                return Err(MergeError::SchemaMismatch);
            }
        } else {
            expected_header = Some(header);
        }
        if value == b"NULL" {
            continue;
        }
        let number = std::str::from_utf8(value).map_err(|_| MergeError::InvalidExtremum)?;
        if !extrema_fits_declared_type(kind, number) {
            return Err(MergeError::InvalidExtremum);
        }
        if unsigned {
            let parsed = number
                .parse::<u128>()
                .map_err(|_| MergeError::InvalidExtremum)?;
            unsigned_value = Some(unsigned_value.map_or(parsed, |prior| match function {
                AggregateFunction::Min => prior.min(parsed),
                AggregateFunction::Max => prior.max(parsed),
                _ => unreachable!(),
            }));
        } else {
            let parsed = number
                .parse::<i128>()
                .map_err(|_| MergeError::InvalidExtremum)?;
            signed = Some(signed.map_or(parsed, |prior| match function {
                AggregateFunction::Min => prior.min(parsed),
                AggregateFunction::Max => prior.max(parsed),
                _ => unreachable!(),
            }));
        }
    }
    let header = expected_header.ok_or(MergeError::InvalidExtremum)?;
    let rendered = if unsigned {
        unsigned_value.map(|number| number.to_string())
    } else {
        signed.map(|number| number.to_string())
    }
    .unwrap_or_else(|| "NULL".to_owned());
    let mut output = header.to_vec();
    output.extend_from_slice(rendered.as_bytes());
    output.push(b'\n');
    Ok(output)
}

#[derive(Hash, PartialEq, Eq)]
enum NormalizedGroupKey<'a> {
    Null,
    Value(Cow<'a, [u8]>),
}

fn normalized_group_key(raw: &[u8]) -> Result<NormalizedGroupKey<'_>, ()> {
    if raw == b"\\N" {
        return Ok(NormalizedGroupKey::Null);
    }
    if raw.contains(&b'\\') {
        let decoded = rusthouse_format::decode_tsv_field(raw).map_err(|_| ())?;
        Ok(NormalizedGroupKey::Value(Cow::Owned(decoded)))
    } else {
        Ok(NormalizedGroupKey::Value(Cow::Borrowed(raw)))
    }
}

/// A signed shard SUM may temporarily exceed i128 and later cancel with
/// another shard. Retain a fixed-size signed high limb rather than rejecting
/// a representable final aggregate. At most 32 peers contribute to each group.
#[derive(Clone, Copy)]
struct WideSignedSum {
    high: i64,
    low: u128,
}

impl WideSignedSum {
    fn new(value: i128) -> Self {
        Self {
            high: if value < 0 { -1 } else { 0 },
            low: value as u128,
        }
    }

    fn add(&mut self, value: i128) -> Result<(), MergeError> {
        let (low, carry) = self.low.overflowing_add(value as u128);
        let sign = if value < 0 { -1i64 } else { 0 };
        self.high = self
            .high
            .checked_add(sign)
            .and_then(|high| high.checked_add(i64::from(carry)))
            .ok_or(MergeError::SumOverflow)?;
        self.low = low;
        Ok(())
    }

    fn to_i128(self) -> Result<i128, MergeError> {
        let signed = self.low as i128;
        if self.high == if signed < 0 { -1 } else { 0 } {
            Ok(signed)
        } else {
            Err(MergeError::SumOverflow)
        }
    }
}

/// Exact grouped COUNT(DISTINCT value) across shards. Each peer returns
/// complete distinct (group, value) rows instead of partial cardinalities.
/// Preserve NULL groups, but exclude NULL aggregate values.
fn merge_grouped_distinct_pairs(
    shards: &[(usize, Vec<u8>)],
    key_column: &str,
    value_column: &str,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    const MAX_GROUPS: usize = 100_000;
    const MAX_PAIRS: usize = 200_000;
    const MAX_KEY_BYTES: usize = 64 * 1024 * 1024;
    let header = format!("{key_column}\t{value_column}\n");
    let mut positions = HashMap::<NormalizedGroupKey<'_>, usize>::new();
    let mut groups = Vec::<(&[u8], u64)>::new();
    let mut seen = HashSet::<(usize, NormalizedGroupKey<'_>)>::new();
    let mut retained_bytes = 0usize;

    for (cap, body) in shards {
        if *cap == 0 || !body.starts_with(header.as_bytes()) || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidGroupedCount);
        }
        let rows = &body[header.len()..];
        if rows.is_empty() {
            continue;
        }
        let mut count = 0usize;
        for row in rows[..rows.len() - 1].split(|&byte| byte == b'\n') {
            count = count.checked_add(1).ok_or(MergeError::CoordinatorLimit)?;
            if count >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            let Some(split) = row.iter().position(|&byte| byte == b'\t') else {
                return Err(MergeError::InvalidGroupedCount);
            };
            let key = &row[..split];
            let value = &row[split + 1..];
            if value.contains(&b'\t') {
                return Err(MergeError::InvalidGroupedCount);
            }
            let normalized_key =
                normalized_group_key(key).map_err(|_| MergeError::InvalidGroupedCount)?;
            let normalized_value =
                normalized_group_key(value).map_err(|_| MergeError::InvalidGroupedCount)?;

            let index = if let Some(&index) = positions.get(&normalized_key) {
                index
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(MergeError::CoordinatorLimit);
                }
                retained_bytes = retained_bytes
                    .checked_add(key.len())
                    .filter(|&bytes| bytes <= MAX_KEY_BYTES)
                    .ok_or(MergeError::CoordinatorLimit)?;
                let next = groups.len();
                positions.insert(normalized_key, next);
                groups.push((key, 0));
                next
            };
            if value == br"\N" {
                continue;
            }
            if seen.insert((index, normalized_value)) {
                if seen.len() > MAX_PAIRS {
                    return Err(MergeError::CoordinatorLimit);
                }
                retained_bytes = retained_bytes
                    .checked_add(value.len())
                    .filter(|&bytes| bytes <= MAX_KEY_BYTES)
                    .ok_or(MergeError::CoordinatorLimit)?;
                groups[index].1 = groups[index]
                    .1
                    .checked_add(1)
                    .ok_or(MergeError::CountOverflow)?;
            }
        }
    }
    if groups.len() > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = format!("{key_column}\tcountDistinct({value_column})\n").into_bytes();
    for (key, count) in groups {
        let digits = count.to_string();
        if output
            .len()
            .checked_add(key.len() + digits.len() + 2)
            .is_none_or(|length| length > MAX_RESPONSE_BYTES)
        {
            return Err(MergeError::CoordinatorLimit);
        }
        output.extend_from_slice(key);
        output.push(b'\t');
        output.extend_from_slice(digits.as_bytes());
        output.push(b'\n');
    }
    Ok(output)
}

/// Complete exact grouped DISTINCT across multiple dimensions. Preserve
/// first-seen logical tuple order and count each unique (tuple, value) once.
fn merge_multi_grouped_distinct_pairs(
    shards: &[(usize, Vec<u8>)],
    columns: &[String],
    value_column: &str,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    const MAX_GROUPS: usize = 100_000;
    const MAX_PAIRS: usize = 200_000;
    const MAX_KEY_BYTES: usize = 64 * 1024 * 1024;
    if !(2..=8).contains(&columns.len()) {
        return Err(MergeError::InvalidGroupedCount);
    }
    let prefix = columns.join("\t");
    let incoming_header = format!("{prefix}\t{value_column}\n");
    let mut positions = HashMap::<Vec<NormalizedGroupKey<'_>>, usize>::new();
    let mut groups = Vec::<(&[u8], u64)>::new();
    let mut seen = HashSet::<(usize, NormalizedGroupKey<'_>)>::new();
    let mut retained_bytes = 0usize;
    for (cap, body) in shards {
        if *cap == 0 || !body.starts_with(incoming_header.as_bytes()) || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidGroupedCount);
        }
        let rows = &body[incoming_header.len()..];
        if rows.is_empty() {
            continue;
        }
        let mut count = 0usize;
        for row in rows[..rows.len() - 1].split(|&byte| byte == b'\n') {
            count = count.checked_add(1).ok_or(MergeError::CoordinatorLimit)?;
            if count >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            let fields = row.split(|&byte| byte == b'\t').collect::<Vec<_>>();
            if fields.len() != columns.len() + 1 {
                return Err(MergeError::InvalidGroupedCount);
            }
            let group = fields[..columns.len()]
                .iter()
                .map(|field| {
                    normalized_group_key(field).map_err(|_| MergeError::InvalidGroupedCount)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let distinct = normalized_group_key(fields[columns.len()])
                .map_err(|_| MergeError::InvalidGroupedCount)?;
            let group_len = row
                .len()
                .checked_sub(fields[columns.len()].len() + 1)
                .ok_or(MergeError::InvalidGroupedCount)?;
            let id = if let Some(&found) = positions.get(&group) {
                found
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(MergeError::CoordinatorLimit);
                }
                retained_bytes = retained_bytes
                    .checked_add(group_len)
                    .filter(|&total| total <= MAX_KEY_BYTES)
                    .ok_or(MergeError::CoordinatorLimit)?;
                let id = groups.len();
                positions.insert(group, id);
                groups.push((&row[..group_len], 0));
                id
            };
            if fields[columns.len()] == br"\N" {
                continue;
            }
            if seen.insert((id, distinct)) {
                if seen.len() > MAX_PAIRS {
                    return Err(MergeError::CoordinatorLimit);
                }
                retained_bytes = retained_bytes
                    .checked_add(fields[columns.len()].len())
                    .filter(|&total| total <= MAX_KEY_BYTES)
                    .ok_or(MergeError::CoordinatorLimit)?;
                groups[id].1 = groups[id]
                    .1
                    .checked_add(1)
                    .ok_or(MergeError::CountOverflow)?;
            }
        }
    }
    if groups.len() > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = format!("{prefix}\tcountDistinct({value_column})\n").into_bytes();
    for (key, count) in groups {
        let digits = count.to_string();
        if output
            .len()
            .checked_add(key.len() + digits.len() + 2)
            .is_none_or(|size| size > MAX_RESPONSE_BYTES)
        {
            return Err(MergeError::CoordinatorLimit);
        }
        output.extend_from_slice(key);
        output.push(b'\t');
        output.extend_from_slice(digits.as_bytes());
        output.push(b'\n');
    }
    Ok(output)
}

fn merge_grouped_integer_sums(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    kind: &str,
    key_column: &str,
    value_column: &str,
) -> Result<Vec<u8>, MergeError> {
    merge_grouped_integer_sums_query(shards, max_rows, kind, key_column, value_column, None)
}

fn merge_grouped_integer_sums_query(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    kind: &str,
    key_column: &str,
    value_column: &str,
    global: Option<&Select>,
) -> Result<Vec<u8>, MergeError> {
    let function = match global.map(|select| &select.projection) {
        Some(Projection::GroupedAggregate {
            function: AggregateFunction::SumSquares,
            ..
        }) => "sumSquares",
        Some(Projection::GroupedAggregate {
            function: AggregateFunction::CountIf,
            ..
        }) => "countIf",
        Some(Projection::GroupedAggregate {
            function: AggregateFunction::Count,
            ..
        }) => "count",
        _ => "sum",
    };
    let header = format!("{key_column}\t{function}({value_column})\n");
    // COUNTIF and COUNT(column) return unsigned counts for all input types.
    let unsigned = matches!(function, "countIf" | "count")
        || kind
            .strip_prefix("Nullable(")
            .unwrap_or(kind)
            .starts_with("UInt");
    let mut positions = HashMap::<NormalizedGroupKey<'_>, usize>::new();
    let mut groups = Vec::<(&[u8], Option<WideSignedSum>, Option<u128>)>::new();
    for (cap, body) in shards {
        if *cap == 0 || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidGroupedSum);
        }
        let header_end = body
            .iter()
            .position(|&byte| byte == b'\n')
            .ok_or(MergeError::InvalidGroupedSum)?;
        if &body[..=header_end] != header.as_bytes() {
            return Err(MergeError::SchemaMismatch);
        }
        let rows = &body[header_end + 1..];
        if rows.is_empty() {
            continue;
        }
        let mut shard_rows = 0usize;
        for row in rows[..rows.len() - 1].split(|&byte| byte == b'\n') {
            if row.is_empty() {
                return Err(MergeError::InvalidGroupedSum);
            }
            shard_rows = shard_rows
                .checked_add(1)
                .ok_or(MergeError::CoordinatorLimit)?;
            if shard_rows >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            let delimiter = row
                .iter()
                .position(|&byte| byte == b'\t')
                .ok_or(MergeError::InvalidGroupedSum)?;
            let key = &row[..delimiter];
            let normalized =
                normalized_group_key(key).map_err(|_| MergeError::InvalidGroupedSum)?;
            let value = &row[delimiter + 1..];
            let partial = if value == b"NULL" {
                None
            } else {
                Some(std::str::from_utf8(value).map_err(|_| MergeError::InvalidGroupedSum)?)
            };
            let index = if let Some(&index) = positions.get(&normalized) {
                index
            } else {
                // Output caps apply after global HAVING, sorting and pagination.
                // A separate bounded working set preserves valid small top-K pages.
                let group_budget = if global.is_some() { 250_000 } else { max_rows };
                if groups.len() >= group_budget {
                    return Err(MergeError::CoordinatorLimit);
                }
                let index = groups.len();
                positions.insert(normalized, index);
                groups.push((key, None, None));
                index
            };
            if let Some(number) = partial {
                if unsigned {
                    let number = number
                        .parse::<u128>()
                        .map_err(|_| MergeError::InvalidGroupedSum)?;
                    groups[index].2 = Some(
                        groups[index]
                            .2
                            .unwrap_or(0)
                            .checked_add(number)
                            .ok_or(MergeError::SumOverflow)?,
                    );
                } else {
                    let number = number
                        .parse::<i128>()
                        .map_err(|_| MergeError::InvalidGroupedSum)?;
                    if let Some(current) = &mut groups[index].1 {
                        current.add(number)?;
                    } else {
                        groups[index].1 = Some(WideSignedSum::new(number));
                    }
                }
            }
        }
    }
    // Convert to i128 only after all shards contribute. A true global
    // overflow still fails, but intermediate cancellation is allowed.
    let groups = groups
        .into_iter()
        .map(|(key, signed, unsigned)| {
            Ok((
                key,
                signed.map(WideSignedSum::to_i128).transpose()?,
                unsigned,
            ))
        })
        .collect::<Result<Vec<_>, MergeError>>()?;
    // Reduce all shards before any global filtering, ranking, or paging.
    // Shard-local HAVING or LIMIT would produce incorrect aggregated results.
    let mut selected = (0..groups.len()).collect::<Vec<_>>();
    if let Some(query) = global {
        if let Some(having) = &query.having {
            selected.retain(|&index| {
                let relation = if unsigned {
                    groups[index]
                        .2
                        .map(|value| value.cmp(&u128::from(having.value)))
                } else {
                    groups[index]
                        .1
                        .map(|value| value.cmp(&i128::from(having.value)))
                };
                relation.is_some_and(|ordering| match having.operator {
                    Operator::Eq => ordering.is_eq(),
                    Operator::Ne => ordering.is_ne(),
                    Operator::Lt => ordering.is_lt(),
                    Operator::Le => ordering.is_le(),
                    Operator::Gt => ordering.is_gt(),
                    Operator::Ge => ordering.is_ge(),
                    _ => false,
                })
            });
        }
        if let Some(order) = &query.order_by {
            let compare = |&left: &usize, &right: &usize| {
                let comparison = if unsigned {
                    groups[left].2.cmp(&groups[right].2)
                } else {
                    groups[left].1.cmp(&groups[right].1)
                };
                let left_null = if unsigned {
                    groups[left].2.is_none()
                } else {
                    groups[left].1.is_none()
                };
                let right_null = if unsigned {
                    groups[right].2.is_none()
                } else {
                    groups[right].1.is_none()
                };
                let comparison = match (left_null, right_null, order.nulls_first) {
                    (true, false, Some(true)) | (false, true, Some(false)) => {
                        std::cmp::Ordering::Less
                    }
                    (true, false, Some(false)) | (false, true, Some(true)) => {
                        std::cmp::Ordering::Greater
                    }
                    _ => {
                        if order.descending {
                            comparison.reverse()
                        } else {
                            comparison
                        }
                    }
                };
                comparison.then_with(|| left.cmp(&right))
            };
            // Large distributed GROUP BY reports often request only the
            // highest few metric groups. Partition first, then sort the small
            // winning prefix rather than fully ranking every group.
            let prefix = query
                .limit
                .and_then(|limit| query.offset.unwrap_or(0).checked_add(limit));
            if let Some(end) = prefix.filter(|&end| end < selected.len()) {
                selected.select_nth_unstable_by(end, &compare);
                selected.truncate(end);
            }
            selected.sort_unstable_by(compare);
        }
    }
    let offset = global.and_then(|query| query.offset).unwrap_or(0);
    let limit = global.and_then(|query| query.limit).unwrap_or(usize::MAX);
    if selected.len().saturating_sub(offset).min(limit) > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = Vec::with_capacity(header.len());
    output.extend_from_slice(header.as_bytes());
    for index in selected.into_iter().skip(offset).take(limit) {
        let (key, signed, unsigned_value) = groups[index];
        let rendered = if unsigned {
            unsigned_value.map(|value| value.to_string())
        } else {
            signed.map(|value| value.to_string())
        }
        .unwrap_or_else(|| "NULL".to_owned());
        if output
            .len()
            .checked_add(key.len() + rendered.len() + 2)
            .is_none_or(|length| length > MAX_RESPONSE_BYTES)
        {
            return Err(MergeError::CoordinatorLimit);
        }
        output.extend_from_slice(key);
        output.push(b'\t');
        output.extend_from_slice(rendered.as_bytes());
        output.push(b'\n');
    }
    Ok(output)
}

fn merge_grouped_integer_aggregate_query(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    kind: &str,
    key_column: &str,
    value_column: &str,
    function: AggregateFunction,
    global: Option<&Select>,
) -> Result<Vec<u8>, MergeError> {
    if !matches!(
        function,
        AggregateFunction::Min
            | AggregateFunction::Max
            | AggregateFunction::Product
            | AggregateFunction::GroupBitAnd
            | AggregateFunction::GroupBitOr
            | AggregateFunction::GroupBitXor
    ) {
        return Err(MergeError::InvalidGroupedSum);
    }
    let header = format!("{key_column}\t{}({value_column})\n", function.name());
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");
    let mut positions = HashMap::<NormalizedGroupKey<'_>, usize>::new();
    let mut groups = Vec::<(&[u8], Option<i128>, Option<u128>)>::new();
    for (cap, body) in shards {
        if *cap == 0 || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidGroupedSum);
        }
        let header_end = body
            .iter()
            .position(|&byte| byte == b'\n')
            .ok_or(MergeError::InvalidGroupedSum)?;
        if &body[..=header_end] != header.as_bytes() {
            return Err(MergeError::SchemaMismatch);
        }
        let rows = &body[header_end + 1..];
        if rows.is_empty() {
            continue;
        }
        let mut shard_rows = 0usize;
        for row in rows[..rows.len() - 1].split(|&byte| byte == b'\n') {
            if row.is_empty() {
                return Err(MergeError::InvalidGroupedSum);
            }
            shard_rows = shard_rows
                .checked_add(1)
                .ok_or(MergeError::CoordinatorLimit)?;
            if shard_rows >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            let delimiter = row
                .iter()
                .position(|&byte| byte == b'\t')
                .ok_or(MergeError::InvalidGroupedSum)?;
            let key = &row[..delimiter];
            let normalized =
                normalized_group_key(key).map_err(|_| MergeError::InvalidGroupedSum)?;
            let value = &row[delimiter + 1..];
            let partial = if value == b"NULL" {
                None
            } else {
                Some(std::str::from_utf8(value).map_err(|_| MergeError::InvalidGroupedSum)?)
            };
            let index = if let Some(&index) = positions.get(&normalized) {
                index
            } else {
                // Output caps apply after global HAVING, sorting and pagination.
                // A separate bounded working set preserves valid small top-K pages.
                let group_budget = if global.is_some() { 250_000 } else { max_rows };
                if groups.len() >= group_budget {
                    return Err(MergeError::CoordinatorLimit);
                }
                let index = groups.len();
                positions.insert(normalized, index);
                groups.push((key, None, None));
                index
            };
            if let Some(number) = partial {
                if matches!(function, AggregateFunction::Min | AggregateFunction::Max)
                    && !extrema_fits_declared_type(kind, number)
                {
                    return Err(MergeError::InvalidGroupedSum);
                }
                if unsigned {
                    let number = number
                        .parse::<u128>()
                        .map_err(|_| MergeError::InvalidGroupedSum)?;
                    groups[index].2 = Some(match groups[index].2 {
                        None => number,
                        Some(prior) => match function {
                            AggregateFunction::Min => prior.min(number),
                            AggregateFunction::Max => prior.max(number),
                            AggregateFunction::Product => prior
                                .checked_mul(number)
                                .ok_or(MergeError::ProductOverflow)?,
                            AggregateFunction::GroupBitAnd => prior & number,
                            AggregateFunction::GroupBitOr => prior | number,
                            AggregateFunction::GroupBitXor => prior ^ number,
                            _ => unreachable!("validated grouped aggregate"),
                        },
                    });
                } else {
                    let number = number
                        .parse::<i128>()
                        .map_err(|_| MergeError::InvalidGroupedSum)?;
                    groups[index].1 = Some(match groups[index].1 {
                        None => number,
                        Some(prior) => match function {
                            AggregateFunction::Min => prior.min(number),
                            AggregateFunction::Max => prior.max(number),
                            AggregateFunction::Product => prior
                                .checked_mul(number)
                                .ok_or(MergeError::ProductOverflow)?,
                            AggregateFunction::GroupBitAnd => prior & number,
                            AggregateFunction::GroupBitOr => prior | number,
                            AggregateFunction::GroupBitXor => prior ^ number,
                            _ => unreachable!("validated grouped aggregate"),
                        },
                    });
                }
            }
        }
    }
    // Reduce all shards before any global filtering, ranking, or paging.
    // Shard-local HAVING or LIMIT would produce incorrect aggregated results.
    let mut selected = (0..groups.len()).collect::<Vec<_>>();
    if let Some(query) = global {
        if let Some(having) = &query.having {
            selected.retain(|&index| {
                let relation = if unsigned {
                    groups[index]
                        .2
                        .map(|value| value.cmp(&u128::from(having.value)))
                } else {
                    groups[index]
                        .1
                        .map(|value| value.cmp(&i128::from(having.value)))
                };
                relation.is_some_and(|ordering| match having.operator {
                    Operator::Eq => ordering.is_eq(),
                    Operator::Ne => ordering.is_ne(),
                    Operator::Lt => ordering.is_lt(),
                    Operator::Le => ordering.is_le(),
                    Operator::Gt => ordering.is_gt(),
                    Operator::Ge => ordering.is_ge(),
                    _ => false,
                })
            });
        }
        if let Some(order) = &query.order_by {
            let compare = |&left: &usize, &right: &usize| {
                let comparison = if unsigned {
                    groups[left].2.cmp(&groups[right].2)
                } else {
                    groups[left].1.cmp(&groups[right].1)
                };
                let left_null = if unsigned {
                    groups[left].2.is_none()
                } else {
                    groups[left].1.is_none()
                };
                let right_null = if unsigned {
                    groups[right].2.is_none()
                } else {
                    groups[right].1.is_none()
                };
                let comparison = match (left_null, right_null, order.nulls_first) {
                    (true, false, Some(true)) | (false, true, Some(false)) => {
                        std::cmp::Ordering::Less
                    }
                    (true, false, Some(false)) | (false, true, Some(true)) => {
                        std::cmp::Ordering::Greater
                    }
                    _ => {
                        if order.descending {
                            comparison.reverse()
                        } else {
                            comparison
                        }
                    }
                };
                comparison.then_with(|| left.cmp(&right))
            };
            // Large distributed GROUP BY reports often request only the
            // highest few metric groups. Partition first, then sort the small
            // winning prefix rather than fully ranking every group.
            let prefix = query
                .limit
                .and_then(|limit| query.offset.unwrap_or(0).checked_add(limit));
            if let Some(end) = prefix.filter(|&end| end < selected.len()) {
                selected.select_nth_unstable_by(end, &compare);
                selected.truncate(end);
            }
            selected.sort_unstable_by(compare);
        }
    }
    let offset = global.and_then(|query| query.offset).unwrap_or(0);
    let limit = global.and_then(|query| query.limit).unwrap_or(usize::MAX);
    if selected.len().saturating_sub(offset).min(limit) > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = Vec::with_capacity(header.len());
    output.extend_from_slice(header.as_bytes());
    for index in selected.into_iter().skip(offset).take(limit) {
        let (key, signed, unsigned_value) = groups[index];
        let rendered = if unsigned {
            unsigned_value.map(|value| value.to_string())
        } else {
            signed.map(|value| value.to_string())
        }
        .unwrap_or_else(|| "NULL".to_owned());
        if output
            .len()
            .checked_add(key.len() + rendered.len() + 2)
            .is_none_or(|length| length > MAX_RESPONSE_BYTES)
        {
            return Err(MergeError::CoordinatorLimit);
        }
        output.extend_from_slice(key);
        output.push(b'\t');
        output.extend_from_slice(rendered.as_bytes());
        output.push(b'\n');
    }
    Ok(output)
}

fn merge_grouped_integer_products_query(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    kind: &str,
    key_column: &str,
    value_column: &str,
    global: Option<&Select>,
) -> Result<Vec<u8>, MergeError> {
    merge_grouped_integer_aggregate_query(
        shards,
        max_rows,
        kind,
        key_column,
        value_column,
        AggregateFunction::Product,
        global,
    )
    .map_err(|error| match error {
        MergeError::InvalidGroupedSum => MergeError::InvalidGroupedProduct,
        other => other,
    })
}

fn merge_grouped_integer_bitwise_query(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    kind: &str,
    key_column: &str,
    value_column: &str,
    function: AggregateFunction,
    global: Option<&Select>,
) -> Result<Vec<u8>, MergeError> {
    merge_grouped_integer_aggregate_query(
        shards,
        max_rows,
        kind,
        key_column,
        value_column,
        function,
        global,
    )
    .map_err(|error| match error {
        MergeError::InvalidGroupedSum => MergeError::InvalidGroupedBitwise,
        other => other,
    })
}

#[cfg(test)]
#[path = "distributed_tests/grouped_sum_intermediate_cap_tests.rs"]
mod grouped_sum_intermediate_cap_tests;
/// Reduce grouped COUNT results by their canonical TSV-encoded key.
/// Preserve first-seen key order, reject malformed rows, and only emit a
/// complete bounded result. Shards may hold overlapping group *keys*, but
/// the underlying source rows must be disjoint.
/// Reduce GROUP BY tuples of up to eight typed dimensions. TSV escapes
/// are canonicalized for hashing, while the first shard's original encoding
/// is retained for output. The tuple key never merges NULL with text "\\N".
fn merge_grouped_multi_counts(
    shards: &[(usize, Vec<u8>)],
    columns: &[String],
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    const MAX_GROUPS: usize = 100_000;
    const MAX_RETAINED_KEYS: usize = 64 * 1024 * 1024;
    if !(2..=8).contains(&columns.len()) {
        return Err(MergeError::InvalidGroupedCount);
    }
    let header = format!("{}\tcount\n", columns.join("\t"));
    let mut groups = Vec::<(&[u8], u64)>::new();
    let mut positions = HashMap::<Vec<NormalizedGroupKey<'_>>, usize>::new();
    let mut key_bytes = 0usize;
    for (cap, body) in shards {
        if *cap == 0 || !body.starts_with(header.as_bytes()) || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidGroupedCount);
        }
        let rows = &body[header.len()..];
        if rows.is_empty() {
            continue;
        }
        let mut shard_rows = 0usize;
        for row in rows[..rows.len() - 1].split(|&byte| byte == b'\n') {
            shard_rows = shard_rows
                .checked_add(1)
                .ok_or(MergeError::CoordinatorLimit)?;
            if shard_rows >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            let fields = row.split(|&byte| byte == b'\t').collect::<Vec<_>>();
            if fields.len() != columns.len() + 1 {
                return Err(MergeError::InvalidGroupedCount);
            }
            let value = std::str::from_utf8(fields[columns.len()])
                .map_err(|_| MergeError::InvalidGroupedCount)?;
            let partial = value
                .parse::<u64>()
                .map_err(|_| MergeError::InvalidGroupedCount)?;
            if partial.to_string() != value {
                return Err(MergeError::InvalidGroupedCount);
            }
            let normalized = fields[..columns.len()]
                .iter()
                .map(|field| {
                    normalized_group_key(field).map_err(|_| MergeError::InvalidGroupedCount)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(&index) = positions.get(&normalized) {
                groups[index].1 = groups[index]
                    .1
                    .checked_add(partial)
                    .ok_or(MergeError::CountOverflow)?;
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(MergeError::CoordinatorLimit);
                }
                let tuple_len = row
                    .len()
                    .checked_sub(value.len() + 1)
                    .ok_or(MergeError::InvalidGroupedCount)?;
                key_bytes = key_bytes
                    .checked_add(tuple_len)
                    .filter(|&bytes| bytes <= MAX_RETAINED_KEYS)
                    .ok_or(MergeError::CoordinatorLimit)?;
                let next = groups.len();
                positions.insert(normalized, next);
                groups.push((&row[..tuple_len], partial));
            }
        }
    }
    if groups.len() > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = header.into_bytes();
    for (key, count) in groups {
        let value = count.to_string();
        if output
            .len()
            .checked_add(key.len() + value.len() + 2)
            .is_none_or(|bytes| bytes > MAX_RESPONSE_BYTES)
        {
            return Err(MergeError::CoordinatorLimit);
        }
        output.extend_from_slice(key);
        output.push(b'\t');
        output.extend_from_slice(value.as_bytes());
        output.push(b'\n');
    }
    Ok(output)
}

fn merge_grouped_counts(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    merge_grouped_counts_query(shards, max_rows, None)
}

fn passes_count_having(count: u64, having: &Having) -> bool {
    match having.operator {
        Operator::Eq => count == having.value,
        Operator::Ne => count != having.value,
        Operator::Lt => count < having.value,
        Operator::Le => count <= having.value,
        Operator::Gt => count > having.value,
        Operator::Ge => count >= having.value,
        Operator::Modulo(divisor) => divisor != 0 && count % divisor == having.value,
        Operator::ModuloNe(divisor) => divisor != 0 && count % divisor != having.value,
    }
}

/// Fully merge shard counts before applying HAVING, ORDER BY and LIMIT.
/// Retain up to the peer protocol's maximum number of distinct group keys,
/// while enforcing the coordinator's output row and byte budgets afterward.
fn merge_grouped_counts_query(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    global: Option<&Select>,
) -> Result<Vec<u8>, MergeError> {
    let mut expected_header: Option<&[u8]> = None;
    let mut positions = HashMap::<NormalizedGroupKey<'_>, usize>::new();
    let mut groups = Vec::<(&[u8], u64)>::new();
    for (cap, body) in shards {
        if !body.ends_with(b"\n") || *cap == 0 {
            return Err(MergeError::InvalidGroupedCount);
        }
        let header_end = body
            .iter()
            .position(|&byte| byte == b'\n')
            .ok_or(MergeError::InvalidGroupedCount)?;
        let header = &body[..=header_end];
        // The local GROUP BY executor writes one key column and count.
        if header.len() <= b"\tcount\n".len()
            || !header.ends_with(b"\tcount\n")
            || header.iter().filter(|&&byte| byte == b'\t').count() != 1
        {
            return Err(MergeError::InvalidGroupedCount);
        }
        if let Some(expected) = expected_header {
            if expected != header {
                return Err(MergeError::SchemaMismatch);
            }
        } else {
            expected_header = Some(header);
        }
        let mut shard_rows = 0usize;
        let rows = &body[header_end + 1..];
        if rows.is_empty() {
            continue;
        }
        for row in rows[..rows.len() - 1].split(|&byte| byte == b'\n') {
            if row.is_empty() {
                return Err(MergeError::InvalidGroupedCount);
            }
            shard_rows = shard_rows
                .checked_add(1)
                .ok_or(MergeError::CoordinatorLimit)?;
            if shard_rows >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            let delimiter = row
                .iter()
                .position(|&byte| byte == b'\t')
                .ok_or(MergeError::InvalidGroupedCount)?;
            let key = &row[..delimiter];
            let normalized =
                normalized_group_key(key).map_err(|_| MergeError::InvalidGroupedCount)?;
            let digits = &row[delimiter + 1..];
            if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
                return Err(MergeError::InvalidGroupedCount);
            }
            let count = std::str::from_utf8(digits)
                .map_err(|_| MergeError::InvalidGroupedCount)?
                .parse::<u64>()
                .map_err(|_| MergeError::CountOverflow)?;
            if let Some(&index) = positions.get(&normalized) {
                let previous = &mut groups[index].1;
                *previous = previous
                    .checked_add(count)
                    .ok_or(MergeError::CountOverflow)?;
            } else {
                let max_groups = if global.is_some() { 250_000 } else { max_rows };
                if groups.len() >= max_groups {
                    return Err(MergeError::CoordinatorLimit);
                }
                positions.insert(normalized, groups.len());
                groups.push((key, count));
            }
        }
    }
    let header = expected_header.ok_or(MergeError::InvalidGroupedCount)?;
    if let Some(select) = global {
        if let Some(having) = &select.having {
            groups.retain(|(_, count)| passes_count_having(*count, having));
        }
        let mut bounded_order = None;
        if let Some(order) = &select.order_by {
            let descending = order.descending;
            let compare = |left: &(&[u8], u64), right: &(&[u8], u64)| {
                let cmp = left.1.cmp(&right.1);
                if descending { cmp.reverse() } else { cmp }
            };
            let end = select
                .limit
                .and_then(|limit| select.offset.unwrap_or(0).checked_add(limit));
            if let Some(end) = end.filter(|&end| end < groups.len()) {
                let mut indices = (0..groups.len()).collect::<Vec<_>>();
                let compare_index = |left: &usize, right: &usize| {
                    compare(&groups[*left], &groups[*right]).then_with(|| left.cmp(right))
                };
                indices.select_nth_unstable_by(end, &compare_index);
                indices.truncate(end);
                indices.sort_unstable_by(compare_index);
                bounded_order = Some(indices);
            } else {
                groups.sort_by(compare);
            }
        }
        groups = if let Some(indices) = bounded_order {
            indices
                .into_iter()
                .skip(select.offset.unwrap_or(0))
                .map(|index| groups[index])
                .collect()
        } else {
            groups
                .into_iter()
                .skip(select.offset.unwrap_or(0))
                .take(select.limit.unwrap_or(usize::MAX))
                .collect()
        };
    }
    if groups.len() > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = Vec::with_capacity(header.len());
    output.extend_from_slice(header);
    for (key, count) in groups {
        let count = count.to_string();
        if output
            .len()
            .checked_add(key.len() + count.len() + 2)
            .is_none_or(|length| length > MAX_RESPONSE_BYTES)
        {
            return Err(MergeError::CoordinatorLimit);
        }
        output.extend_from_slice(key);
        output.push(b'\t');
        output.extend_from_slice(count.as_bytes());
        output.push(b'\n');
    }
    Ok(output)
}

pub fn handle(method: &str, body: &[u8], cluster: &Cluster, max_rows: usize) -> Response {
    if method != "POST" {
        return Response::text(
            "405 Method Not Allowed",
            "distributed query requires POST\n",
        );
    }
    if body.is_empty() || body.len() > MAX_QUERY_BYTES {
        return Response::text("400 Bad Request", "invalid distributed query length\n");
    }
    let sql = match std::str::from_utf8(body) {
        Ok(sql) => sql,
        Err(_) => return Response::text("400 Bad Request", "SQL must be UTF-8\n"),
    };
    let (select, query_kind) = match parse_select(sql).ok().and_then(|select| {
        distributed_query_kind(&select).map(|kind| (select, kind))
    }) {
        Some(plan) => plan,
        None => {
            return Response::text(
                "400 Bad Request",
                "distributed queries require unordered SELECT, bounded integer top-K SELECT, COUNT, COUNT(DISTINCT), uniq, COUNTIF, integer SUM, MIN, MAX, or grouped aggregates\n",
            );
        }
    };
    let sorted_query = matches!(query_kind, DistributedQueryKind::SortedScan);
    let sorted_prefix =
        sorted_query.then(|| select.offset.unwrap_or(0) + select.limit.unwrap_or(0));
    let count_query = is_safe_count(&select) || is_safe_count_if(&select);
    let distinct_rows_query = matches!(query_kind, DistributedQueryKind::DistinctRows);
    let multi_grouped_distinct_query = is_safe_multi_grouped_distinct(&select);
    let grouped_distinct_query = is_safe_grouped_distinct(&select);
    let distinct_count_column = match &select.projection {
        Projection::CountDistinct { column } => Some(column.as_str()),
        _ => None,
    };
    let grouped_count_query = is_safe_grouped_count(&select);
    let grouped_multi_count_query = is_safe_grouped_multi_count(&select);
    let grouped_multi_count_column_query = is_safe_grouped_multi_count_column(&select);
    let grouped_multi_bitwise_query = is_safe_grouped_multi_bitwise(&select);
    let grouped_multi_product_query = is_safe_grouped_multi_product(&select);
    let grouped_multi_sumsquares_query = is_safe_grouped_multi_sumsquares(&select);
    let grouped_multi_countif_query = is_safe_grouped_multi_countif(&select);
    let grouped_multi_extremum_query = is_safe_grouped_multi_extremum(&select);
    let grouped_multi_sum_query = is_safe_grouped_multi_sum(&select);
    let grouped_sum_query = is_safe_grouped_sum(&select);
    let grouped_count_column_query = matches!(
        select.projection,
        Projection::GroupedAggregate {
            function: AggregateFunction::Count,
            ..
        }
    );
    let grouped_extremum_query = is_safe_grouped_extremum(&select);
    let grouped_product_query = is_safe_grouped_product(&select);
    let grouped_bitwise_query = is_safe_grouped_bitwise(&select);
    let sum_query = is_safe_sum(&select) || is_safe_sum_squares(&select);
    let product_query = is_safe_product(&select);
    let rms_query = is_safe_rms(&select);
    let avg_query = is_safe_avg(&select);
    let range_query = is_safe_range(&select);
    let uniq_query = is_safe_uniq(&select);
    let shard_sql = if sorted_query {
        match sorted_scan_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported distributed sort\n"),
        }
    } else if multi_grouped_distinct_query {
        match multi_grouped_distinct_shard_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported grouped DISTINCT\n"),
        }
    } else if grouped_distinct_query {
        match grouped_distinct_shard_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported grouped DISTINCT\n"),
        }
    } else if distinct_rows_query {
        match unpaged_scan_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text(
                    "400 Bad Request",
                    "unsupported distributed DISTINCT pagination\n",
                );
            }
        }
    } else if is_safe_scan(&select) {
        match bounded_scan_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text("400 Bad Request", "unsupported distributed pagination\n");
            }
        }
    } else if grouped_sum_query
        || grouped_extremum_query
        || grouped_product_query
        || grouped_bitwise_query
    {
        match grouped_count_shard_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text("400 Bad Request", "unsupported global grouped aggregate\n");
            }
        }
    } else if grouped_count_query {
        match grouped_count_shard_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text("400 Bad Request", "unsupported global grouped query\n");
            }
        }
    } else if rms_query {
        match rms_shard_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported distributed RMS\n"),
        }
    } else if avg_query {
        match avg_shard_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported distributed AVG\n"),
        }
    } else if range_query {
        match range_shard_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported distributed RANGE\n"),
        }
    } else if uniq_query {
        match uniq_state_shard_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text("400 Bad Request", "unsupported distributed uniq query\n");
            }
        }
    } else if distinct_count_column.is_some() {
        match distinct_shard_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text(
                    "400 Bad Request",
                    "unsupported distributed DISTINCT query\n",
                );
            }
        }
    } else {
        sql.to_owned()
    };
    let bitwise_function = match &select.projection {
        Projection::Aggregate {
            function:
                function @ (AggregateFunction::GroupBitAnd
                | AggregateFunction::GroupBitOr
                | AggregateFunction::GroupBitXor),
            ..
        } => Some(*function),
        _ => None,
    };
    let extrema_function = match &select.projection {
        Projection::Aggregate {
            function: function @ (AggregateFunction::Min | AggregateFunction::Max),
            ..
        } => Some(*function),
        _ => None,
    };
    let sum_schema = match (&select.source, &select.projection) {
        (
            Some(Source::Table(table)),
            Projection::Aggregate {
                function:
                    AggregateFunction::Sum
                    | AggregateFunction::SumSquares
                    | AggregateFunction::Rms
                    | AggregateFunction::Avg
                    | AggregateFunction::Range
                    | AggregateFunction::Product
                    | AggregateFunction::Min
                    | AggregateFunction::Max
                    | AggregateFunction::GroupBitAnd
                    | AggregateFunction::GroupBitOr
                    | AggregateFunction::GroupBitXor,
                column,
            },
        ) => Some((table.as_str(), column.as_str())),
        (
            Some(Source::Table(table)),
            Projection::GroupedAggregate {
                function:
                    AggregateFunction::Sum
                    | AggregateFunction::SumSquares
                    | AggregateFunction::Count
                    | AggregateFunction::CountIf
                    | AggregateFunction::Min
                    | AggregateFunction::Max
                    | AggregateFunction::Product
                    | AggregateFunction::GroupBitAnd
                    | AggregateFunction::GroupBitOr
                    | AggregateFunction::GroupBitXor,
                value_column,
                ..
            },
        ) => Some((table.as_str(), value_column.as_str())),
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_count_column_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_bitwise_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_product_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_sumsquares_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_countif_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_extremum_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::GroupedMultiAggregates { items, .. })
            if grouped_multi_sum_query =>
        {
            Some((table.as_str(), items[0].1.as_str()))
        }
        (Some(Source::Table(table)), Projection::Columns(columns)) if sorted_query => columns
            .first()
            .map(|column| (table.as_str(), column.as_str())),
        _ => None,
    };
    let group_schema = match (&select.source, &select.projection) {
        (
            Some(Source::Table(table)),
            Projection::GroupedAggregate {
                function:
                    AggregateFunction::Sum
                    | AggregateFunction::SumSquares
                    | AggregateFunction::Count
                    | AggregateFunction::CountIf
                    | AggregateFunction::Min
                    | AggregateFunction::Max
                    | AggregateFunction::Product
                    | AggregateFunction::GroupBitAnd
                    | AggregateFunction::GroupBitOr
                    | AggregateFunction::GroupBitXor,
                column,
                ..
            },
        ) => Some((table.as_str(), column.as_str())),
        _ => None,
    };
    // Verify that only a named database table can be scanned.
    if !matches!(select.source, Some(Source::Table(_))) || cluster.peers().is_empty() {
        return Response::text(
            "400 Bad Request",
            "no distributed database peers configured\n",
        );
    }
    let table = match &select.source {
        Some(Source::Table(table)) => table.as_str(),
        _ => unreachable!("validated named table"),
    };
    let retained_input = AtomicUsize::new(0);
    // Bound concurrency without wave barriers: a fast peer immediately
    // claims the next pending shard, even if an earlier peer is still slow.
    const MAX_INFLIGHT_SHARDS: usize = 8;
    let results =
        dispatch::map_peers_bounded_failfast(cluster.peers(), MAX_INFLIGHT_SHARDS, |peer| {
            read_shard(
                peer,
                shard_sql.as_str(),
                sum_schema,
                group_schema,
                table,
                sorted_prefix,
                grouped_multi_count_column_query || grouped_count_column_query,
                extrema_function.is_some(),
                &retained_input,
            )
        });
    let mut shards = Vec::with_capacity(results.len());
    let mut sum_type: Option<String> = None;
    let mut group_key_kind: Option<String> = None;
    let mut expected_schema: Option<Vec<u8>> = None;
    for result in results {
        match result {
            Ok(ShardResponse {
                cap,
                body,
                kind,
                key_kind,
                schema,
            }) => {
                if expected_schema
                    .as_ref()
                    .is_some_and(|expected| expected != &schema)
                {
                    return Response::text("409 Conflict", "distributed shard schemas disagree\n");
                }
                if expected_schema.is_none() {
                    expected_schema = Some(schema);
                }
                if let Some(kind) = kind {
                    if sum_type
                        .as_deref()
                        .is_some_and(|existing| existing != kind.as_str())
                    {
                        return Response::text(
                            "409 Conflict",
                            "distributed integer aggregate shard column types disagree\n",
                        );
                    }
                    sum_type = Some(kind);
                }
                if let Some(key_kind) = key_kind {
                    if group_key_kind
                        .as_deref()
                        .is_some_and(|existing| existing != key_kind.as_str())
                    {
                        return Response::text(
                            "409 Conflict",
                            "distributed GROUP BY key types disagree\n",
                        );
                    }
                    group_key_kind = Some(key_kind);
                }
                shards.push((cap, body));
            }
            Err(message) => {
                return Response::text("502 Bad Gateway", &format!("{message}\n"));
            }
        }
    }
    let merged = if count_query {
        sum_shard_counts(&shards)
    } else if let Some(column) = distinct_count_column {
        let kind = expected_schema
            .as_deref()
            .and_then(|schema| validated_schema_kind(schema, column));
        match kind {
            Some(
                kind @ ("Int8" | "UInt8" | "Int16" | "UInt16" | "Nullable(Int8)"
                | "Nullable(UInt8)" | "Nullable(Int16)" | "Nullable(UInt16)"),
            ) => merge_distinct_bitmap16(&shards, column, kind),
            Some("Int32" | "Nullable(Int32)") => merge_distinct_bitmap32(&shards, column, true),
            Some("UInt32" | "Nullable(UInt32)") => merge_distinct_bitmap32(&shards, column, false),
            Some("UUID") => merge_distinct_uuid(&shards, column, false),
            Some("Nullable(UUID)") => merge_distinct_uuid(&shards, column, true),
            Some(
                kind @ ("Int64" | "UInt64" | "Int128" | "UInt128" | "Nullable(Int64)"
                | "Nullable(UInt64)" | "Nullable(Int128)" | "Nullable(UInt128)"),
            ) => merge_distinct_wide_integers(&shards, column, kind),
            _ => merge_distinct_counts(&shards, column),
        }
    } else if multi_grouped_distinct_query {
        match &select.projection {
            Projection::GroupedMultiDistinctCount {
                columns,
                value_column,
            } => merge_multi_grouped_distinct_pairs(
                &shards,
                columns,
                value_column,
                max_rows.min(250_000),
            ),
            _ => Err(MergeError::InvalidGroupedCount),
        }
    } else if grouped_distinct_query {
        match &select.projection {
            Projection::GroupedDistinctCount {
                column,
                value_column,
            } => merge_grouped_distinct_pairs(&shards, column, value_column, max_rows.min(250_000)),
            _ => Err(MergeError::InvalidGroupedCount),
        }
    } else if uniq_query {
        match &select.projection {
            Projection::ApproxDistinct { column } => {
                merge_uniq_shard_states(&shards, column, select.alias.as_deref())
            }
            _ => Err(MergeError::InvalidUniqState),
        }
    } else if sum_query {
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidSum), |kind| {
                merge_integer_sums(&shards, kind)
            })
    } else if rms_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::Aggregate { column, .. }, Some(kind)) if integer_type(kind) => {
                rms_merge::merge_global_rms(&shards, column)
            }
            _ => Err(MergeError::InvalidSum),
        }
    } else if avg_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::Aggregate { column, .. }, Some(kind)) => {
                avg_merge::merge_global_avg(&shards, column, kind)
            }
            _ => Err(MergeError::InvalidSum),
        }
    } else if range_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::Aggregate { column, .. }, Some(kind)) => {
                range_merge::merge_global_range(&shards, column, kind)
            }
            _ => Err(MergeError::InvalidExtremum),
        }
    } else if product_query {
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidProduct), |kind| {
                merge_integer_products(&shards, kind)
            })
    } else if let Some(function) = bitwise_function {
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidBitwise), |kind| {
                merge_integer_bitwise(&shards, kind, function)
            })
    } else if let Some(function) = extrema_function {
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidExtremum), |kind| {
                if matches!(kind, "UUID" | "Nullable(UUID)") {
                    uuid_extrema::merge_uuid_extrema(&shards, function)
                } else {
                    merge_integer_extrema(&shards, kind, function)
                }
            })
    } else if grouped_extremum_query || grouped_product_query || grouped_bitwise_query {
        match (
            sum_type.as_deref(),
            group_schema,
            sum_schema,
            &select.projection,
        ) {
            (
                Some(kind),
                Some((_, key)),
                Some((_, value)),
                Projection::GroupedAggregate { function, .. },
            ) => {
                let limit = max_rows.min(250_000);
                match function {
                    AggregateFunction::Min | AggregateFunction::Max => {
                        merge_grouped_integer_aggregate_query(
                            &shards,
                            limit,
                            kind,
                            key,
                            value,
                            *function,
                            Some(&select),
                        )
                    }
                    AggregateFunction::Product => merge_grouped_integer_products_query(
                        &shards,
                        limit,
                        kind,
                        key,
                        value,
                        Some(&select),
                    ),
                    AggregateFunction::GroupBitAnd
                    | AggregateFunction::GroupBitOr
                    | AggregateFunction::GroupBitXor => merge_grouped_integer_bitwise_query(
                        &shards,
                        limit,
                        kind,
                        key,
                        value,
                        *function,
                        Some(&select),
                    ),
                    _ => Err(MergeError::InvalidGroupedSum),
                }
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_sum_query {
        match (sum_type.as_deref(), group_schema, sum_schema) {
            (Some(kind), Some((_, key)), Some((_, value))) => {
                if matches!(
                    select.projection,
                    Projection::GroupedAggregate {
                        function: AggregateFunction::SumSquares,
                        ..
                    }
                ) || select.having.is_some()
                    || select.order_by.is_some()
                    || select.limit.is_some()
                    || select.offset.is_some()
                {
                    merge_grouped_integer_sums_query(
                        &shards,
                        max_rows.min(250_000),
                        kind,
                        key,
                        value,
                        Some(&select),
                    )
                } else {
                    merge_grouped_integer_sums(&shards, max_rows.min(250_000), kind, key, value)
                }
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_multi_sum_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::GroupedMultiAggregates { columns, items, .. }, Some(kind)) => {
                multi_sum::merge_multi_key_sums(
                    &shards,
                    columns,
                    &items[0].1,
                    kind,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_multi_extremum_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::GroupedMultiAggregates { columns, items, .. }, Some(kind)) => {
                multi_extrema::merge_multi_key_extrema(
                    &shards,
                    columns,
                    &items[0].1,
                    kind,
                    items[0].0,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_multi_countif_query {
        match &select.projection {
            Projection::GroupedMultiAggregates { columns, items, .. } => {
                multi_countif::merge_multi_key_countif(
                    &shards,
                    columns,
                    &items[0].1,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_multi_sumsquares_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::GroupedMultiAggregates { columns, items, .. }, Some(kind)) => {
                multi_sumsquares::merge_multi_key_sumsquares(
                    &shards,
                    columns,
                    &items[0].1,
                    kind,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_multi_count_column_query {
        match &select.projection {
            Projection::GroupedMultiAggregates { columns, items, .. } => {
                multi_count_column::merge_multi_key_count_column(
                    &shards,
                    columns,
                    &items[0].1,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_multi_count_query {
        match &select.projection {
            Projection::GroupedMultiCount { columns } => {
                merge_grouped_multi_counts(&shards, columns, max_rows.min(250_000))
            }
            _ => Err(MergeError::InvalidGroupedCount),
        }
    } else if grouped_multi_bitwise_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::GroupedMultiAggregates { columns, items, .. }, Some(kind)) => {
                multi_bitwise::merge_multi_key_bitwise(
                    &shards,
                    columns,
                    &items[0].1,
                    kind,
                    items[0].0,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedBitwise),
        }
    } else if grouped_multi_product_query {
        match (&select.projection, sum_type.as_deref()) {
            (Projection::GroupedMultiAggregates { columns, items, .. }, Some(kind)) => {
                multi_product::merge_multi_key_product(
                    &shards,
                    columns,
                    &items[0].1,
                    kind,
                    max_rows.min(250_000),
                )
            }
            _ => Err(MergeError::InvalidGroupedProduct),
        }
    } else if grouped_count_query {
        let limit = max_rows.min(250_000);
        if select.having.is_some()
            || select.order_by.is_some()
            || select.limit.is_some()
            || select.offset.is_some()
        {
            merge_grouped_counts_query(&shards, limit, Some(&select))
        } else {
            merge_grouped_counts(&shards, limit)
        }
    } else if sorted_query {
        let order = select.order_by.as_ref().expect("validated bounded sort");
        let Projection::Columns(columns) = &select.projection else {
            unreachable!("validated bounded sort");
        };
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidSortedScan), |kind| {
                let names = columns.iter().map(String::as_str).collect::<Vec<_>>();
                if matches!(kind, "String" | "Nullable(String)") {
                    merge_sorted_text_rows_page_with_nulls(
                        &shards,
                        &names,
                        kind,
                        order.descending,
                        order.nulls_first,
                        select.offset.unwrap_or(0),
                        select.limit.unwrap_or(0),
                        max_rows.min(250_000),
                    )
                } else {
                    merge_sorted_integer_rows_page_with_nulls(
                        &shards,
                        &names,
                        kind,
                        order.descending,
                        order.nulls_first,
                        select.offset.unwrap_or(0),
                        select.limit.unwrap_or(0),
                        max_rows.min(250_000),
                    )
                }
            })
    } else if distinct_rows_query {
        merge_distinct_tsv(
            &shards,
            max_rows.min(250_000),
            select.offset.unwrap_or(0),
            select.limit,
        )
    } else if select.limit.is_some() || select.offset.is_some() {
        merge_tsv_page(
            &shards,
            max_rows.min(250_000),
            select.offset.unwrap_or(0),
            select.limit,
        )
    } else {
        merge_tsv(&shards, max_rows.min(250_000))
    };
    match merged {
        Ok(body) => Response {
            status: "200 OK",
            content_type: "text/tab-separated-values; charset=utf-8",
            body,
        },
        Err(MergeError::SchemaMismatch) => {
            Response::text("409 Conflict", "distributed shard headers disagree\n")
        }
        Err(MergeError::ShardRowLimit) => Response::text(
            "409 Conflict",
            "shard reached its result cap; increase peer --max-rows\n",
        ),
        Err(MergeError::CoordinatorLimit) => Response::text(
            "413 Payload Too Large",
            "distributed result exceeds coordinator limits\n",
        ),
        Err(MergeError::InvalidResponse) => {
            Response::text("502 Bad Gateway", "malformed shard TSV response\n")
        }
        Err(MergeError::InvalidCount) => {
            Response::text("502 Bad Gateway", "malformed shard COUNT response\n")
        }
        Err(MergeError::InvalidUniqState) => {
            Response::text("502 Bad Gateway", "malformed shard uniq state\n")
        }
        Err(MergeError::InvalidDistinctCount) => {
            Response::text("502 Bad Gateway", "malformed shard DISTINCT response\n")
        }
        Err(MergeError::InvalidGroupedCount) => {
            Response::text("502 Bad Gateway", "malformed shard GROUP BY response\n")
        }
        Err(MergeError::InvalidGroupedSum) => {
            Response::text("502 Bad Gateway", "malformed shard grouped SUM response\n")
        }
        Err(MergeError::InvalidGroupedProduct) => Response::text(
            "502 Bad Gateway",
            "malformed shard grouped PRODUCT response\n",
        ),
        Err(MergeError::InvalidGroupedBitwise) => Response::text(
            "502 Bad Gateway",
            "malformed shard grouped bitwise response\n",
        ),
        Err(MergeError::InvalidSortedScan) => Response::text(
            "502 Bad Gateway",
            "malformed shard sorted integer response\n",
        ),
        Err(MergeError::InvalidSum) => {
            Response::text("502 Bad Gateway", "malformed shard SUM response\n")
        }
        Err(MergeError::InvalidBitwise) => Response::text(
            "502 Bad Gateway",
            "malformed shard bitwise aggregate response\n",
        ),
        Err(MergeError::InvalidProduct) => {
            Response::text("502 Bad Gateway", "malformed shard PRODUCT response\n")
        }
        Err(MergeError::ProductOverflow) => {
            Response::text("413 Payload Too Large", "distributed PRODUCT overflow\n")
        }
        Err(MergeError::InvalidExtremum) => {
            Response::text("502 Bad Gateway", "malformed shard MIN/MAX response\n")
        }
        Err(MergeError::SumOverflow) => {
            Response::text("413 Payload Too Large", "distributed SUM overflow\n")
        }
        Err(MergeError::CountOverflow) => {
            Response::text("413 Payload Too Large", "distributed count overflow\n")
        }
    }
}

#[cfg(test)]
#[path = "distributed_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "distributed_tests/single_pass_page_tests.rs"]
mod single_pass_page_tests;

#[cfg(test)]
#[path = "distributed_tests/bounded_grouped_count_topk_tests.rs"]
mod bounded_grouped_count_topk_tests;

#[cfg(test)]
mod concurrent_preflight_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn both_metadata_requests_can_be_in_flight_before_either_completes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = Peer::parse(&listener.local_addr().unwrap().to_string()).unwrap();
        let server = std::thread::spawn(move || {
            // Accepting both before answering either is an intentional
            // concurrency requirement, not a wall-clock performance test.
            let (mut first, _) = listener.accept().unwrap();
            let (mut second, _) = listener.accept().unwrap();
            for stream in [&mut first, &mut second] {
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut incoming = [0u8; 1024];
                let size = stream.read(&mut incoming).unwrap();
                let path = std::str::from_utf8(&incoming[..size]).unwrap();
                let payload = if path.starts_with("GET /capabilities ") {
                    format!(
                        "{{\"name\":\"RustHouse\",\"version\":\"0.1.0\",\"source\":\"database\",\"read_only\":true,\"max_query_bytes\":{MAX_QUERY_BYTES},\"max_response_bytes\":{MAX_RESPONSE_BYTES},\"max_rows\":10000}}"
                    )
                } else if path.starts_with("GET /schema?table=facts ") {
                    r#"{"rows":3,"columns":[{"name":"id","type":"UInt64"}]}"#.to_string()
                } else {
                    panic!("unexpected metadata request {path}");
                };
                write!(stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json; charset=utf-8\r\n\r\n{}",
                    payload.len(), payload
                ).unwrap();
            }
        });
        let (cap, response) = preflight_shard(peer, "facts").unwrap();
        assert_eq!(cap, 10_000);
        assert_eq!(response.status, 200);
        server.join().unwrap();
    }
}

#[cfg(test)]
#[path = "distributed_tests/distributed_grouped_sum_selection_tests.rs"]
mod distributed_grouped_sum_selection_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_uniq_state_tests.rs"]
mod distributed_uniq_state_tests;

#[cfg(test)]
#[path = "distributed_tests/coordinator_input_budget_tests.rs"]
mod coordinator_input_budget_tests;

#[cfg(test)]
#[path = "distributed_tests/shard_dispatch_wave_tests.rs"]
mod shard_dispatch_wave_tests;

#[cfg(test)]
#[path = "distributed_tests/peer_schema_row_count_tests.rs"]
mod peer_schema_row_count_tests;

#[cfg(test)]
#[path = "distributed_tests/peer_capability_suffix_tests.rs"]
mod peer_capability_suffix_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_normalized_group_key_tests.rs"]
mod distributed_normalized_group_key_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_distinct_escape_normalization_tests.rs"]
mod distributed_distinct_escape_normalization_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_scan_tsv_shape_tests.rs"]
mod distributed_scan_tsv_shape_tests;

#[cfg(test)]
mod structured_peer_schema_tests {
    use super::*;

    fn peer(body: &str) -> remote::PeerResponse {
        remote::PeerResponse {
            status: 200,
            content_type: "application/json; charset=utf-8".into(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn accepts_whitespace_without_changing_the_column_signature() {
        let compact = peer(r#"{"rows":1,"columns":[{"name":"id","type":"UInt64"}]}"#);
        let spaced = peer(
            " { \"rows\" : 999 , \"columns\" : [ { \"name\" : \"id\", \"type\" : \"UInt64\" } ] }\n",
        );
        assert_eq!(
            schema_signature(&compact).unwrap(),
            schema_signature(&spaced).unwrap()
        );
        assert_eq!(shard_column_kind(&spaced, "id").unwrap(), "UInt64");
    }

    #[test]
    fn rejects_duplicate_columns_and_trailing_or_embedded_json() {
        for bad in [
            r#"{"rows":1,"columns":[{"name":"id","type":"UInt64"},{"name":"id","type":"String"}]}"#,
            r#"{"rows":1,"columns":[{"name":"id","type":"UInt64"}],"ignored":1}"#,
            r#"{"rows":1,"columns":[{"name":"id","type":"UInt64"}]} true"#,
            r#"{"rows":1,"columns":[{"name":"id","type":"UInt64"}"#,
        ] {
            assert!(schema_signature(&peer(bad)).is_err(), "{bad}");
        }
    }
}

#[cfg(test)]
#[path = "distributed_tests/global_distinct_row_tests.rs"]
mod global_distinct_row_tests;

#[cfg(test)]
#[path = "distributed_tests/malformed_distributed_tsv_tests.rs"]
mod malformed_distributed_tsv_tests;

#[cfg(test)]
#[path = "distributed_tests/logical_distinct_tsv_equivalence_tests.rs"]
mod logical_distinct_tsv_equivalence_tests;

#[cfg(test)]
mod strict_distributed_capabilities_tests {
    use super::*;

    fn valid() -> Vec<u8> {
        format!(
            "{{\"name\":\"RustHouse\",\"version\":\"0.1.0\",\"source\":\"database\",\"read_only\":true,\"max_query_bytes\":{MAX_QUERY_BYTES},\"max_response_bytes\":{MAX_RESPONSE_BYTES},\"max_rows\":42}}\n"
        ).into_bytes()
    }

    #[test]
    fn accepts_only_structured_canonical_peer_capabilities() {
        assert_eq!(parse_peer_capabilities(&valid()).unwrap(), 42);
        let mut spaced = valid();
        spaced.splice(1..1, b"  ".iter().copied());
        assert_eq!(parse_peer_capabilities(&spaced).unwrap(), 42);
    }

    #[test]
    fn rejects_embedded_decoys_duplicate_keys_and_wrong_source_modes() {
        let good = String::from_utf8(valid()).unwrap();
        for bad in [
            good.replace("\"source\":\"database\"", "\"source\":\"part\""),
            good.replace("\"read_only\":true", "\"read_only\":false"),
            good.replace("\"max_rows\":42", "\"max_rows\":42,\"max_rows\":7"),
            good.replace("\"max_rows\":42", "\"max_rows\":0"),
            good.replace("\"max_rows\":42", "\"max_rows\":042"),
            good.replace("\"max_rows\":42", "\"max_rows\":\"42\""),
            good.replace(
                "\"version\":\"0.1.0\"",
                "\"version\":\"0.1.0\",\"version\":\"evil\"",
            ),
            good.replace("\"name\":\"RustHouse\"", "\"name\":\"FakeRustHouse\""),
            good.replace(
                "\"max_response_bytes\":16777216",
                "\"max_response_bytes\":1",
            ),
            format!("{{\"spoof\":\"{}\"}}", good.replace('"', "\\\"")),
        ] {
            assert!(parse_peer_capabilities(bad.as_bytes()).is_err(), "{bad}");
        }
    }
}

#[cfg(test)]
#[path = "distributed_tests/peer_preflight_version_checks.rs"]
mod peer_preflight_version_checks;

#[cfg(test)]
#[path = "distributed_tests/distributed_extrema_width_validation_tests.rs"]
mod distributed_extrema_width_validation_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_signed_sum_cancellation_tests.rs"]
mod distributed_signed_sum_cancellation_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_grouped_signed_cancellation_tests.rs"]
mod distributed_grouped_signed_cancellation_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_bitwise_aggregate_tests.rs"]
mod distributed_bitwise_aggregate_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_sum_squares_tests.rs"]
mod distributed_sum_squares_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_integer_product_tests.rs"]
mod distributed_integer_product_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_grouped_sum_squares_tests.rs"]
mod distributed_grouped_sum_squares_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_grouped_extrema_tests.rs"]
mod distributed_grouped_extrema_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_grouped_product_tests.rs"]
mod distributed_grouped_product_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_grouped_bitwise_tests.rs"]
mod distributed_grouped_bitwise_tests;

#[cfg(test)]
#[path = "distributed_tests/narrow_global_distinct_bitmap_tests.rs"]
mod narrow_global_distinct_bitmap_tests;

#[cfg(test)]
#[path = "distributed_tests/global_wide_distinct_tests.rs"]
mod global_wide_distinct_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_uuid_distinct_tests.rs"]
mod distributed_uuid_distinct_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_multi_key_grouped_count_tests.rs"]
mod distributed_multi_key_grouped_count_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_grouped_distinct_count_tests.rs"]
mod distributed_grouped_distinct_count_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_multi_key_distinct_tests.rs"]
mod distributed_multi_key_distinct_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_grouped_countif_tests.rs"]
mod distributed_grouped_countif_tests;

#[cfg(test)]
#[path = "distributed_tests/distributed_grouped_count_column_tests.rs"]
mod distributed_grouped_count_column_tests;
