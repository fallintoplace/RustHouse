//! Experimental read-only scatter/gather SELECT against distinct local shards.
//! Unordered row projections and global pagination preserve configured peer order.

mod query_policy;

use self::query_policy::*;

use crate::cluster::{Cluster, Peer};
use crate::remote;
use crate::{MAX_QUERY_BYTES, MAX_RESPONSE_BYTES, Response};
use rusthouse_query::{AggregateFunction, Having, Operator, Projection, Select, Source, parse_select};
use std::collections::{HashMap, HashSet};

#[derive(Debug, PartialEq, Eq)]
enum MergeError {
    InvalidResponse,
    SchemaMismatch,
    ShardRowLimit,
    CoordinatorLimit,
    InvalidCount,
    InvalidDistinctCount,
    InvalidGroupedCount,
    InvalidGroupedSum,
    InvalidSum,
    InvalidExtremum,
    CountOverflow,
    SumOverflow,
}

/// Ordinary counts sum across shards; DISTINCT needs global key de-duplication.
/// Only exact integer SUM is decomposable without changing the result type.
/// GROUP BY counts can be reduced by key across disjoint read-only shards.
/// A global HAVING, ORDER BY or LIMIT would require different planning.
/// Remove pagination before sending a scan to shards. Sending the original
/// LIMIT/OFFSET would drop rows on each shard and produce incorrect results.
/// Accept only a prefix that reparses to the exact unpaged query AST.
fn merge_distinct_counts(shards: &[(usize, Vec<u8>)], column: &str) -> Result<Vec<u8>, MergeError> {
    const MAX_KEYS: usize = 100_000;
    let expected_header = format!("{column}\n");
    let mut values: HashSet<&[u8]> = HashSet::new();
    for (cap, body) in shards {
        if !body.ends_with(b"\n") || !body.starts_with(expected_header.as_bytes()) {
            return Err(MergeError::InvalidDistinctCount);
        }
        let rows = &body[expected_header.len()..];
        let mut shard_rows = 0usize;
        // Slice::split includes the final empty segment. Remove the final
        // newline first, but preserve a genuine empty-string row.
        if !rows.is_empty() {
            let rows = &rows[..rows.len() - 1];
            for value in rows.split(|&b| b == b'\n') {
                shard_rows = shard_rows
                    .checked_add(1)
                    .ok_or(MergeError::CoordinatorLimit)?;
                if *cap == 0 || shard_rows >= *cap {
                    return Err(MergeError::ShardRowLimit);
                }
                if value != b"\\N" {
                    if values.len() >= MAX_KEYS && !values.contains(value) {
                        return Err(MergeError::CoordinatorLimit);
                    }
                    values.insert(value);
                }
            }
        }
    }
    Ok(format!("count(DISTINCT {column})\n{}\n", values.len()).into_bytes())
}

fn shard_row_cap(peer: Peer) -> Result<usize, String> {
    let response = remote::capabilities(peer)?;
    if response.status != 200 || !response.content_type.starts_with("application/json") {
        return Err("peer capabilities request failed".into());
    }
    let data =
        std::str::from_utf8(&response.body).map_err(|_| "peer capabilities are not UTF-8")?;
    if !data.contains("\"name\":\"RustHouse\"")
        || !data.contains("\"source\":\"database\"")
        || !data.contains("\"read_only\":true")
    {
        return Err("peer is not a read-only RustHouse database server".into());
    }
    let (_, cap_text) = data
        .split_once("\"max_rows\":")
        .ok_or("peer does not advertise a row limit")?;
    let digits = cap_text.bytes().take_while(|b| b.is_ascii_digit()).count();
    let cap = cap_text[..digits]
        .parse::<usize>()
        .map_err(|_| "invalid peer row limit")?;
    if cap == 0 || cap > 250_000 {
        return Err("unsupported peer row limit".into());
    }
    Ok(cap)
}

/// Fetch a column type for distributed aggregate validation.
/// Simple ASCII identifiers cannot inject JSON or an HTTP request target.
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
    let contents =
        std::str::from_utf8(&response.body).map_err(|_| "peer schema response is not UTF-8")?;
    let needle = format!("\"name\":\"{column}\",\"type\":\"");
    let (_, tail) = contents
        .split_once(needle.as_str())
        .ok_or("aggregate column missing from peer schema")?;
    let (kind, trailing) = tail
        .split_once('"')
        .ok_or("invalid peer schema column type")?;
    if !trailing.starts_with('}') {
        return Err("invalid peer schema column type".into());
    }
    Ok(kind.to_owned())
}

fn shard_integer_kind(response: &remote::PeerResponse, column: &str) -> Result<String, String> {
    let kind = shard_column_kind(response, column)?;
    if !integer_type(&kind) {
        return Err("distributed SUM only supports exact integer columns".into());
    }
    Ok(kind)
}

/// Compare column names, types, and order, not row counts. Each shard
/// independently reports its own row count and should differ on real clusters.
fn schema_signature(response: &remote::PeerResponse) -> Result<Vec<u8>, String> {
    if response.status != 200 || response.content_type != "application/json; charset=utf-8" {
        return Err("peer schema request failed".into());
    }
    let body = std::str::from_utf8(&response.body).map_err(|_| "peer schema is not UTF-8")?;
    let body = body
        .strip_suffix("}\n")
        .ok_or("invalid peer schema response")?;
    let (rows, columns) = body
        .split_once(",\"columns\":")
        .ok_or("peer schema has no columns")?;
    let count = rows
        .strip_prefix("{\"rows\":")
        .ok_or("peer schema has no row count")?;
    if count.is_empty()
        || !count.bytes().all(|b| b.is_ascii_digit())
        || !columns.starts_with('[')
        || !columns.ends_with(']')
        || !columns.contains("\"name\":")
        || !columns.contains("\"type\":")
    {
        return Err("invalid peer schema response".into());
    }
    Ok(columns.as_bytes().to_vec())
}

struct ShardResponse {
    cap: usize,
    body: Vec<u8>,
    kind: Option<String>,
    key_kind: Option<String>,
    schema: Vec<u8>,
    multi_kinds: Option<Vec<String>>,
    group_sort_kinds: Option<Vec<String>>,
}

fn read_shard(
    peer: Peer,
    sql: &str,
    sum_schema: Option<(&str, &str)>,
    group_schema: Option<(&str, &str)>,
    multi_schema: Option<&[(AggregateFunction, String)]>,
    group_sort_columns: Option<&[String]>,
    table: &str,
) -> Result<ShardResponse, String> {
    let cap = shard_row_cap(peer)?;
    let schema_response = remote::schema(peer, table)?;
    let schema = schema_signature(&schema_response)?;
    let kind = sum_schema
        .map(|(_, column)| shard_integer_kind(&schema_response, column))
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
    let multi_kinds = multi_schema.map(|items| {
        items.iter().map(|(function, column)| {
            if *function == AggregateFunction::Count {
                shard_column_kind(&schema_response, column)
            } else {
                shard_integer_kind(&schema_response, column)
            }
        }).collect::<Result<Vec<_>, _>>()
    }).transpose()?;
    let group_sort_kinds = group_sort_columns.map(|columns| {
        columns.iter().map(|column| {
            let kind = shard_column_kind(&schema_response, column)?;
            if !group_key_type(&kind) {
                return Err("unsupported distributed grouping order key type".to_owned());
            }
            Ok(kind)
        }).collect::<Result<Vec<_>, _>>()
    }).transpose()?;
    let result = remote::query(peer, sql)?;
    if result.status != 200 {
        return Err(format!("peer returned HTTP {}", result.status));
    }
    if result.content_type != "text/tab-separated-values; charset=utf-8" {
        return Err("peer did not return TSV".into());
    }
    Ok(ShardResponse {
        cap,
        body: result.body,
        kind,
        key_kind,
        schema,
        multi_kinds,
        group_sort_kinds,
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
        let shard_rows = rows.iter().filter(|&&b| b == b'\n').count();
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

/// Apply LIMIT/OFFSET only after validating every shard's full result and
/// advertised cap. Rows are paged in stable configured-peer order.
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
        let shard_rows = rows.iter().filter(|&&b| b == b'\n').count();
        if *cap == 0 || shard_rows >= *cap {
            return Err(MergeError::ShardRowLimit);
        }
        for row in rows.split_inclusive(|&b| b == b'\n') {
            if row.is_empty() {
                continue;
            }
            let in_page = seen_rows >= offset && limit.is_none_or(|limit| returned_rows < limit);
            seen_rows = seen_rows
                .checked_add(1)
                .ok_or(MergeError::CoordinatorLimit)?;
            if !in_page {
                continue;
            }
            if returned_rows >= max_rows
                || output
                    .len()
                    .checked_add(row.len())
                    .is_none_or(|n| n > MAX_RESPONSE_BYTES)
            {
                return Err(MergeError::CoordinatorLimit);
            }
            output.extend_from_slice(row);
            returned_rows += 1;
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
        if !body.ends_with(b"\n") {
            return Err(MergeError::InvalidCount);
        }
        let end = body
            .iter()
            .position(|&b| b == b'\n')
            .ok_or(MergeError::InvalidCount)?;
        let header = &body[..=end];
        if header.len() <= 1 {
            return Err(MergeError::InvalidCount);
        }
        if let Some(expected) = expected_header {
            if expected != header {
                return Err(MergeError::SchemaMismatch);
            }
        } else {
            expected_header = Some(header);
        }
        let value = &body[end + 1..body.len() - 1];
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
    let mut signed_total = 0i128;
    let mut unsigned_total = 0u128;
    let mut seen = false;
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");

    for (_, body) in shards {
        if !body.ends_with(b"\n") {
            return Err(MergeError::InvalidSum);
        }
        let end = body
            .iter()
            .position(|&byte| byte == b'\n')
            .ok_or(MergeError::InvalidSum)?;
        let header = &body[..=end];
        if header.len() <= 1 || body.len() <= end + 1 {
            return Err(MergeError::InvalidSum);
        }
        if let Some(expected) = expected_header {
            if expected != header {
                return Err(MergeError::SchemaMismatch);
            }
        } else {
            expected_header = Some(header);
        }
        let value = &body[end + 1..body.len() - 1];
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
            signed_total = signed_total
                .checked_add(partial)
                .ok_or(MergeError::SumOverflow)?;
        }
        seen = true;
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
        if !body.ends_with(b"\n") {
            return Err(MergeError::InvalidExtremum);
        }
        let end = body
            .iter()
            .position(|&byte| byte == b'\n')
            .ok_or(MergeError::InvalidExtremum)?;
        let header = &body[..=end];
        if header.len() <= 1 || body.len() <= end + 1 {
            return Err(MergeError::InvalidExtremum);
        }
        if let Some(expected) = expected_header {
            if expected != header {
                return Err(MergeError::SchemaMismatch);
            }
        } else {
            expected_header = Some(header);
        }
        let value = &body[end + 1..body.len() - 1];
        if value == b"NULL" {
            continue;
        }
        let number = std::str::from_utf8(value).map_err(|_| MergeError::InvalidExtremum)?;
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

fn merge_grouped_integer_sums(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    kind: &str,
    key_column: &str,
    value_column: &str,
) -> Result<Vec<u8>, MergeError> {
    merge_grouped_integer_sums_query(shards, max_rows, kind, key_column, value_column, None)
}

fn sum_satisfies_having(value: Option<i128>, having: &Having) -> bool {
    let Some(value) = value else {
        return false;
    };
    let target = i128::from(having.value);
    match having.operator {
        Operator::Eq => value == target,
        Operator::Ne => value != target,
        Operator::Lt => value < target,
        Operator::Le => value <= target,
        Operator::Gt => value > target,
        Operator::Ge => value >= target,
        Operator::Modulo(divisor) => divisor != 0 && value % i128::from(divisor) == target,
        Operator::ModuloNe(divisor) => divisor != 0 && value % i128::from(divisor) != target,
    }
}

fn unsigned_sum_satisfies_having(value: Option<u128>, having: &Having) -> bool {
    let Some(value) = value else {
        return false;
    };
    let target = u128::from(having.value);
    match having.operator {
        Operator::Eq => value == target,
        Operator::Ne => value != target,
        Operator::Lt => value < target,
        Operator::Le => value <= target,
        Operator::Gt => value > target,
        Operator::Ge => value >= target,
        Operator::Modulo(divisor) => divisor != 0 && value % u128::from(divisor) == target,
        Operator::ModuloNe(divisor) => divisor != 0 && value % u128::from(divisor) != target,
    }
}

/// Merge exact partial values first, then apply global HAVING, ranking and paging.
fn merge_grouped_integer_sums_query(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    kind: &str,
    key_column: &str,
    value_column: &str,
    global: Option<&Select>,
) -> Result<Vec<u8>, MergeError> {
    merge_grouped_integer_sums_typed(
        shards, max_rows, kind, key_column, value_column, global, None,
    )
}

fn merge_grouped_integer_sums_typed(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    kind: &str,
    key_column: &str,
    value_column: &str,
    global: Option<&Select>,
    group_kind: Option<&str>,
) -> Result<Vec<u8>, MergeError> {
    merge_grouped_integer_sums_multi_typed(
        shards, max_rows, kind, key_column, value_column, global, group_kind, None,
    )
}

fn merge_grouped_integer_sums_multi_typed(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    kind: &str,
    key_column: &str,
    value_column: &str,
    global: Option<&Select>,
    group_kind: Option<&str>,
    group_kinds: Option<&[String]>,
) -> Result<Vec<u8>, MergeError> {
    let header = format!("{key_column}\tsum({value_column})\n");
    let unsigned = kind
        .strip_prefix("Nullable(")
        .unwrap_or(kind)
        .starts_with("UInt");
    let mut positions = HashMap::<&[u8], usize>::new();
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
            // Composite grouping keys span every field before the SUM value.
            let key_width = key_column.as_bytes().iter().filter(|&&b| b == b'\t').count() + 1;
            if row.iter().filter(|&&b| b == b'\t').count() != key_width {
                return Err(MergeError::InvalidGroupedSum);
            }
            let delimiter = row
                .iter()
                .rposition(|&byte| byte == b'\t')
                .ok_or(MergeError::InvalidGroupedSum)?;
            let key = &row[..delimiter];
            let value = &row[delimiter + 1..];
            let partial = if value == b"NULL" {
                None
            } else {
                Some(std::str::from_utf8(value).map_err(|_| MergeError::InvalidGroupedSum)?)
            };
            let index = if let Some(&index) = positions.get(key) {
                index
            } else {
                let max_groups = if global.is_some() { 250_000 } else { max_rows };
                if groups.len() >= max_groups {
                    return Err(MergeError::CoordinatorLimit);
                }
                let index = groups.len();
                positions.insert(key, index);
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
                    groups[index].1 = Some(
                        groups[index]
                            .1
                            .unwrap_or(0)
                            .checked_add(number)
                            .ok_or(MergeError::SumOverflow)?,
                    );
                }
            }
        }
    }
    if let Some(select) = global {
        if let Some(having) = &select.having {
            groups.retain(|(_, signed, unsigned_value)| {
                if unsigned {
                    unsigned_sum_satisfies_having(*unsigned_value, having)
                } else {
                    sum_satisfies_having(*signed, having)
                }
            });
        }
        if select.order_by.is_some() {
            let terms = select.order_terms().collect::<Vec<_>>();
            let wants_key = terms.iter().any(|order| order.column != "#aggregate");
            let group_columns = key_column.split('\t').collect::<Vec<_>>();
            let kinds = if !wants_key {
                None
            } else if group_columns.len() == 1 {
                Some(vec![group_kind.ok_or(MergeError::InvalidGroupedSum)?])
            } else {
                let kinds = group_kinds.ok_or(MergeError::InvalidGroupedSum)?;
                if kinds.len() != group_columns.len() {
                    return Err(MergeError::InvalidGroupedSum);
                }
                Some(kinds.iter().map(String::as_str).collect::<Vec<_>>())
            };
            let mut indices = groups.iter().enumerate()
                .map(|(index, (key, _, _))| {
                    let parsed = if let Some(kinds) = &kinds {
                        let fields = key.split(|&byte| byte == b'\t').collect::<Vec<_>>();
                        if fields.len() != kinds.len() {
                            return Err(MergeError::InvalidGroupedSum);
                        }
                        Some(fields.into_iter().zip(kinds)
                            .map(|(field, kind)| parse_group_sort_key(field, kind))
                            .collect::<Result<Vec<_>, _>>()?)
                    } else {
                        None
                    };
                    Ok((index, parsed))
                })
                .collect::<Result<Vec<_>, MergeError>>()?;
            indices.sort_by(|(left, left_key), (right, right_key)| {
                for order in &terms {
                    let compare = if order.column == "#aggregate" {
                        let l = &groups[*left];
                        let r = &groups[*right];
                        let cmp = if unsigned { l.2.cmp(&r.2) } else { l.1.cmp(&r.1) };
                        let a_null = if unsigned { l.2.is_none() } else { l.1.is_none() };
                        let b_null = if unsigned { r.2.is_none() } else { r.1.is_none() };
                        match (a_null, b_null, order.nulls_first) {
                            (true, false, Some(true)) => std::cmp::Ordering::Less,
                            (false, true, Some(true)) => std::cmp::Ordering::Greater,
                            (true, false, Some(false)) => std::cmp::Ordering::Greater,
                            (false, true, Some(false)) => std::cmp::Ordering::Less,
                            _ => if order.descending { cmp.reverse() } else { cmp },
                        }
                    } else {
                        let position = group_columns.iter()
                            .position(|column| *column == order.column)
                            .expect("validated grouped key ORDER BY");
                        compare_group_sort_keys(
                            &left_key.as_ref().expect("verified group key")[position],
                            &right_key.as_ref().expect("verified group key")[position],
                            order.descending, order.nulls_first,
                        )
                    };
                    if compare != std::cmp::Ordering::Equal {
                        return compare;
                    }
                }
                std::cmp::Ordering::Equal
            });
            groups = indices.into_iter().map(|(index, _)| groups[index]).collect();
        }
        groups = groups
            .into_iter()
            .skip(select.offset.unwrap_or(0))
            .take(select.limit.unwrap_or(usize::MAX))
            .collect();
    }
    if groups.len() > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = Vec::with_capacity(header.len());
    output.extend_from_slice(header.as_bytes());
    for (key, signed, unsigned_value) in groups {
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


/// Reduce one-key grouped MIN/MAX without trusting a shard-local top-k result.
fn merge_grouped_integer_extrema(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    kind: &str,
    key_column: &str,
    value_column: &str,
    function: AggregateFunction,
    global: Option<&Select>,
) -> Result<Vec<u8>, MergeError> {
    merge_grouped_integer_extrema_typed(
        shards, max_rows, kind, key_column, value_column, function, global, None,
    )
}

fn merge_grouped_integer_extrema_typed(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    kind: &str,
    key_column: &str,
    value_column: &str,
    function: AggregateFunction,
    global: Option<&Select>,
    group_kind: Option<&str>,
) -> Result<Vec<u8>, MergeError> {
    if !matches!(function, AggregateFunction::Min | AggregateFunction::Max) {
        return Err(MergeError::InvalidExtremum);
    }
    let header = format!("{key_column}\t{}({value_column})\n", function.name());
    let unsigned = kind.strip_prefix("Nullable(").unwrap_or(kind).starts_with("UInt");
    let mut positions = HashMap::<&[u8], usize>::new();
    let mut groups = Vec::<(&[u8], Option<i128>, Option<u128>)>::new();
    for (cap, body) in shards {
        if *cap == 0 || !body.ends_with(b"\n") || !body.starts_with(header.as_bytes()) {
            return Err(MergeError::InvalidExtremum);
        }
        let rows = &body[header.len()..];
        if rows.is_empty() {
            continue;
        }
        let mut shard_rows = 0usize;
        for row in rows[..rows.len() - 1].split(|&byte| byte == b'\n') {
            shard_rows = shard_rows.checked_add(1).ok_or(MergeError::CoordinatorLimit)?;
            if shard_rows >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            // All grouping dimensions form the composite key.
            let key_width = key_column.as_bytes().iter().filter(|&&b| b == b'\t').count() + 1;
            if row.iter().filter(|&&b| b == b'\t').count() != key_width {
                return Err(MergeError::InvalidExtremum);
            }
            let split = row.iter().rposition(|&byte| byte == b'\t')
                .ok_or(MergeError::InvalidExtremum)?;
            let key = &row[..split];
            let text = &row[split + 1..];
            if text.is_empty() || text.iter().any(|&byte| byte == b'\t') {
                return Err(MergeError::InvalidExtremum);
            }
            let index = if let Some(&index) = positions.get(key) {
                index
            } else {
                let max_groups = if global.is_some() { 250_000 } else { max_rows };
                if groups.len() >= max_groups {
                    return Err(MergeError::CoordinatorLimit);
                }
                let index = groups.len();
                positions.insert(key, index);
                groups.push((key, None, None));
                index
            };
            if text == b"NULL" {
                continue;
            }
            if unsigned {
                let value = std::str::from_utf8(text)
                    .map_err(|_| MergeError::InvalidExtremum)?
                    .parse::<u128>().map_err(|_| MergeError::InvalidExtremum)?;
                groups[index].2 = Some(groups[index].2.map_or(value, |old| {
                    if function == AggregateFunction::Min { old.min(value) } else { old.max(value) }
                }));
            } else {
                let value = std::str::from_utf8(text)
                    .map_err(|_| MergeError::InvalidExtremum)?
                    .parse::<i128>().map_err(|_| MergeError::InvalidExtremum)?;
                groups[index].1 = Some(groups[index].1.map_or(value, |old| {
                    if function == AggregateFunction::Min { old.min(value) } else { old.max(value) }
                }));
            }
        }
    }
    if let Some(select) = global {
        if let Some(having) = &select.having {
            groups.retain(|(_, signed, unsigned_value)| {
                if unsigned {
                    unsigned_sum_satisfies_having(*unsigned_value, having)
                } else {
                    sum_satisfies_having(*signed, having)
                }
            });
        }
        if select.order_by.is_some() {
            let terms = select.order_terms().collect::<Vec<_>>();
            let group_kind = if terms.iter().any(|term| term.column != "#aggregate") {
                Some(group_kind.ok_or(MergeError::InvalidExtremum)?)
            } else {
                None
            };
            let mut indices = groups.iter().enumerate()
                .map(|(index, (key, _, _))| {
                    group_kind.map_or(Ok(None), |kind| {
                        parse_group_sort_key(key, kind).map(Some)
                    }).map(|key| (index, key))
                })
                .collect::<Result<Vec<_>, MergeError>>()?;
            indices.sort_by(|(left, left_key), (right, right_key)| {
                for term in &terms {
                    let cmp = if term.column == "#aggregate" {
                        let a = &groups[*left];
                        let b = &groups[*right];
                        let cmp = if unsigned { a.2.cmp(&b.2) } else { a.1.cmp(&b.1) };
                        let a_null = if unsigned { a.2.is_none() } else { a.1.is_none() };
                        let b_null = if unsigned { b.2.is_none() } else { b.1.is_none() };
                        match (a_null, b_null, term.nulls_first) {
                            (true, false, Some(true)) => std::cmp::Ordering::Less,
                            (false, true, Some(true)) => std::cmp::Ordering::Greater,
                            (true, false, Some(false)) => std::cmp::Ordering::Greater,
                            (false, true, Some(false)) => std::cmp::Ordering::Less,
                            _ => if term.descending { cmp.reverse() } else { cmp },
                        }
                    } else {
                        compare_group_sort_keys(
                            left_key.as_ref().expect("validated group key"),
                            right_key.as_ref().expect("validated group key"),
                            term.descending, term.nulls_first,
                        )
                    };
                    if cmp != std::cmp::Ordering::Equal { return cmp; }
                }
                std::cmp::Ordering::Equal
            });
            groups = indices.into_iter().map(|(index, _)| groups[index]).collect();
        }
        groups = groups.into_iter()
            .skip(select.offset.unwrap_or(0))
            .take(select.limit.unwrap_or(usize::MAX)).collect();
    }
    if groups.len() > max_rows {
        return Err(MergeError::CoordinatorLimit);
    }
    let mut output = header.into_bytes();
    for (key, signed, unsigned_value) in groups {
        let rendered = if unsigned {
            unsigned_value.map(|value| value.to_string())
        } else {
            signed.map(|value| value.to_string())
        }.unwrap_or_else(|| "NULL".to_owned());
        if output.len().checked_add(key.len() + rendered.len() + 2)
            .is_none_or(|size| size > MAX_RESPONSE_BYTES)
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


/// Separate partial state for each exact aggregate in a fused GROUP BY.
enum GroupedMetric {
    Count(u64),
    Signed(Option<i128>),
    Unsigned(Option<u128>),
}

fn merge_grouped_fused_metrics(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    keys: &[String],
    items: &[(AggregateFunction, String)],
    kinds: &[String],
) -> Result<Vec<u8>, MergeError> {
    if keys.is_empty() || items.len() < 2 || items.len() != kinds.len() {
        return Err(MergeError::InvalidGroupedSum);
    }
    let mut header = keys.join("\t");
    for (function, column) in items {
        header.push_str(&format!("\t{}({column})", function.name()));
    }
    header.push('\n');
    let state = || items.iter().zip(kinds).map(|((function, _), kind)| {
        if *function == AggregateFunction::Count {
            GroupedMetric::Count(0)
        } else if kind.strip_prefix("Nullable(").unwrap_or(kind).starts_with("UInt") {
            GroupedMetric::Unsigned(None)
        } else {
            GroupedMetric::Signed(None)
        }
    }).collect::<Vec<_>>();
    let mut positions = HashMap::<&[u8], usize>::new();
    let mut groups = Vec::<(&[u8], Vec<GroupedMetric>)>::new();
    for (cap, body) in shards {
        if *cap == 0 || !body.ends_with(b"\n") || !body.starts_with(header.as_bytes()) {
            return Err(MergeError::InvalidGroupedSum);
        }
        let records = &body[header.len()..];
        if records.is_empty() {
            continue;
        }
        let mut shard_rows = 0usize;
        for row in records[..records.len() - 1].split(|&b| b == b'\n') {
            shard_rows = shard_rows.checked_add(1).ok_or(MergeError::CoordinatorLimit)?;
            if shard_rows >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            let delimiters = row.iter().enumerate()
                .filter_map(|(position, &b)| (b == b'\t').then_some(position))
                .collect::<Vec<_>>();
            if delimiters.len() != keys.len() + items.len() - 1 {
                return Err(MergeError::InvalidGroupedSum);
            }
            let key_end = delimiters[keys.len() - 1];
            let key = &row[..key_end];
            let fields = row[key_end + 1..].split(|&b| b == b'\t').collect::<Vec<_>>();
            if fields.len() != items.len() {
                return Err(MergeError::InvalidGroupedSum);
            }
            let index = if let Some(&index) = positions.get(key) {
                index
            } else {
                if groups.len() >= max_rows {
                    return Err(MergeError::CoordinatorLimit);
                }
                let index = groups.len();
                positions.insert(key, index);
                groups.push((key, state()));
                index
            };
            for (((function, _), field), metric) in items.iter()
                .zip(fields).zip(groups[index].1.iter_mut())
            {
                if field.is_empty() {
                    return Err(MergeError::InvalidGroupedSum);
                }
                match metric {
                    GroupedMetric::Count(value) if *function == AggregateFunction::Count => {
                        let partial = std::str::from_utf8(field)
                            .map_err(|_| MergeError::InvalidGroupedSum)?
                            .parse::<u64>().map_err(|_| MergeError::InvalidGroupedSum)?;
                        *value = value.checked_add(partial).ok_or(MergeError::CountOverflow)?;
                    }
                    GroupedMetric::Signed(current) => {
                        if field == b"NULL" { continue; }
                        let value = std::str::from_utf8(field)
                            .map_err(|_| MergeError::InvalidGroupedSum)?
                            .parse::<i128>().map_err(|_| MergeError::InvalidGroupedSum)?;
                        *current = Some(match (*function, *current) {
                            (AggregateFunction::Sum, Some(previous)) =>
                                previous.checked_add(value).ok_or(MergeError::SumOverflow)?,
                            (AggregateFunction::Min, Some(previous)) => previous.min(value),
                            (AggregateFunction::Max, Some(previous)) => previous.max(value),
                            (_, None) => value,
                            _ => return Err(MergeError::InvalidGroupedSum),
                        });
                    }
                    GroupedMetric::Unsigned(current) => {
                        if field == b"NULL" { continue; }
                        let value = std::str::from_utf8(field)
                            .map_err(|_| MergeError::InvalidGroupedSum)?
                            .parse::<u128>().map_err(|_| MergeError::InvalidGroupedSum)?;
                        *current = Some(match (*function, *current) {
                            (AggregateFunction::Sum, Some(previous)) =>
                                previous.checked_add(value).ok_or(MergeError::SumOverflow)?,
                            (AggregateFunction::Min, Some(previous)) => previous.min(value),
                            (AggregateFunction::Max, Some(previous)) => previous.max(value),
                            (_, None) => value,
                            _ => return Err(MergeError::InvalidGroupedSum),
                        });
                    }
                    _ => return Err(MergeError::InvalidGroupedSum),
                }
            }
        }
    }
    let mut output = header.into_bytes();
    for (key, metrics) in groups {
        let mut row = key.to_vec();
        for metric in metrics {
            row.push(b'\t');
            let value = match metric {
                GroupedMetric::Count(value) => value.to_string(),
                GroupedMetric::Signed(value) => value.map(|v| v.to_string()).unwrap_or("NULL".into()),
                GroupedMetric::Unsigned(value) => value.map(|v| v.to_string()).unwrap_or("NULL".into()),
            };
            row.extend_from_slice(value.as_bytes());
        }
        row.push(b'\n');
        if output.len().checked_add(row.len()).is_none_or(|size| size > MAX_RESPONSE_BYTES) {
            return Err(MergeError::CoordinatorLimit);
        }
        output.extend_from_slice(&row);
    }
    Ok(output)
}


/// Global exact distinct by a single group key. A shared (key, value) pair
/// contributes to its group once even when the same value occurs on many peers.
fn merge_grouped_distinct_pairs(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    column: &str,
    value_column: &str,
    global: &Select,
) -> Result<Vec<u8>, MergeError> {
    merge_grouped_distinct_pairs_typed(
        shards, max_rows, column, value_column, global, None,
    )
}

fn merge_grouped_distinct_pairs_typed(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    column: &str,
    value_column: &str,
    global: &Select,
    key_kind: Option<&str>,
) -> Result<Vec<u8>, MergeError> {
    const MAX_GROUP_VALUE_PAIRS: usize = 250_000;
    let shard_header = format!("{column}\t{value_column}\n");
    let result_header = format!("{column}\tcountDistinct({value_column})\n");
    let mut positions = HashMap::<&[u8], usize>::new();
    let mut groups = Vec::<(&[u8], u64)>::new();
    let mut pairs = HashSet::<(&[u8], &[u8])>::new();
    for (cap, body) in shards {
        if *cap == 0 || !body.ends_with(b"\n") || !body.starts_with(shard_header.as_bytes()) {
            return Err(MergeError::InvalidDistinctCount);
        }
        let records = &body[shard_header.len()..];
        if records.is_empty() { continue; }
        let mut shard_rows = 0usize;
        for row in records[..records.len() - 1].split(|&b| b == b'\n') {
            shard_rows = shard_rows.checked_add(1).ok_or(MergeError::CoordinatorLimit)?;
            if shard_rows >= *cap { return Err(MergeError::ShardRowLimit); }
            let split = row.iter().position(|&b| b == b'\t')
                .ok_or(MergeError::InvalidDistinctCount)?;
            let key = &row[..split];
            let value = &row[split + 1..];
            if value.iter().any(|&b| b == b'\t') {
                return Err(MergeError::InvalidDistinctCount);
            }
            let index = if let Some(&index) = positions.get(key) {
                index
            } else {
                if groups.len() >= 250_000 {
                    return Err(MergeError::CoordinatorLimit);
                }
                let index = groups.len();
                positions.insert(key, index);
                groups.push((key, 0));
                index
            };
            // COUNT(DISTINCT) excludes NULL, but NULL-only groups still emit 0.
            if value != b"\\N" {
                if pairs.len() >= MAX_GROUP_VALUE_PAIRS && !pairs.contains(&(key, value)) {
                    return Err(MergeError::CoordinatorLimit);
                }
                if pairs.insert((key, value)) {
                    groups[index].1 = groups[index].1.checked_add(1)
                        .ok_or(MergeError::CountOverflow)?;
                }
            }
        }
    }
    if let Some(having) = &global.having {
        groups.retain(|(_, count)| passes_count_having(*count, having));
    }
    if global.order_by.is_some() {
        let terms = global.order_terms().collect::<Vec<_>>();
        if terms.iter().all(|term| term.column == "count") {
            groups.sort_by(|left, right| {
                for term in &terms {
                    let cmp = left.1.cmp(&right.1);
                    let cmp = if term.descending { cmp.reverse() } else { cmp };
                    if cmp != std::cmp::Ordering::Equal { return cmp; }
                }
                std::cmp::Ordering::Equal
            });
        } else {
            let kind = key_kind.ok_or(MergeError::InvalidDistinctCount)?;
            let mut indices = groups.iter().enumerate()
                .map(|(index, (key, _))| {
                    parse_group_sort_key(key, kind).map(|key| (index, key))
                })
                .collect::<Result<Vec<_>, _>>()?;
            indices.sort_by(|(left, left_key), (right, right_key)| {
                for term in &terms {
                    let cmp = if term.column == "count" {
                        groups[*left].1.cmp(&groups[*right].1)
                    } else {
                        compare_group_sort_keys(
                            left_key, right_key, term.descending, term.nulls_first,
                        )
                    };
                    let cmp = if term.column == "count" && term.descending {
                        cmp.reverse()
                    } else { cmp };
                    if cmp != std::cmp::Ordering::Equal { return cmp; }
                }
                std::cmp::Ordering::Equal
            });
            groups = indices.into_iter().map(|(index, _)| groups[index]).collect();
        }
    }
    let mut output = result_header.into_bytes();
    let mut emitted = 0usize;
    for (key, count) in groups.into_iter()
        .skip(global.offset.unwrap_or(0))
        .take(global.limit.unwrap_or(usize::MAX))
    {
        let digits = count.to_string();
        if emitted >= max_rows
            || output.len().checked_add(key.len() + digits.len() + 2)
                .is_none_or(|n| n > MAX_RESPONSE_BYTES)
        {
            return Err(MergeError::CoordinatorLimit);
        }
        output.extend_from_slice(key);
        output.push(b'\t');
        output.extend_from_slice(digits.as_bytes());
        output.push(b'\n');
        emitted += 1;
    }
    Ok(output)
}


/// Parsed GROUP BY key sort values retain numeric SQL ordering.
enum GroupSortKey {
    Null,
    Signed(i128),
    Unsigned(u128),
    Bytes(Vec<u8>),
}

fn parse_group_sort_key(field: &[u8], kind: &str) -> Result<GroupSortKey, MergeError> {
    if field == b"\\N" {
        return Ok(GroupSortKey::Null);
    }
    let kind = kind.strip_prefix("Nullable(").and_then(|s| s.strip_suffix(')'))
        .unwrap_or(kind);
    if kind.starts_with("UInt") {
        return Ok(GroupSortKey::Unsigned(
            std::str::from_utf8(field).map_err(|_| MergeError::InvalidGroupedCount)?
                .parse::<u128>().map_err(|_| MergeError::InvalidGroupedCount)?,
        ));
    }
    if kind.starts_with("Int") {
        return Ok(GroupSortKey::Signed(
            std::str::from_utf8(field).map_err(|_| MergeError::InvalidGroupedCount)?
                .parse::<i128>().map_err(|_| MergeError::InvalidGroupedCount)?,
        ));
    }
    if matches!(kind, "String" | "UUID") {
        return Ok(GroupSortKey::Bytes(
            rusthouse_format::decode_tsv_field(field)
                .map_err(|_| MergeError::InvalidGroupedCount)?,
        ));
    }
    Err(MergeError::InvalidGroupedCount)
}

fn compare_group_sort_keys(
    left: &GroupSortKey,
    right: &GroupSortKey,
    descending: bool,
    nulls_first: Option<bool>,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left, right) {
        (GroupSortKey::Null, GroupSortKey::Null) => Ordering::Equal,
        (GroupSortKey::Null, _) => {
            if nulls_first == Some(true) || (nulls_first.is_none() && !descending) {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (_, GroupSortKey::Null) => {
            if nulls_first == Some(true) || (nulls_first.is_none() && !descending) {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        _ => {
            let ordering = match (left, right) {
                (GroupSortKey::Signed(a), GroupSortKey::Signed(b)) => a.cmp(b),
                (GroupSortKey::Unsigned(a), GroupSortKey::Unsigned(b)) => a.cmp(b),
                (GroupSortKey::Bytes(a), GroupSortKey::Bytes(b)) => a.cmp(b),
                _ => Ordering::Equal,
            };
            if descending { ordering.reverse() } else { ordering }
        }
    }
}

/// Reduce grouped COUNT results by their canonical TSV-encoded key.
/// Preserve first-seen key order, reject malformed rows, and only emit a
/// complete bounded result. Shards may hold overlapping group *keys*, but
/// the underlying source rows must be disjoint.
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
    merge_grouped_counts_typed(shards, max_rows, global, None)
}

fn merge_grouped_counts_typed(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    global: Option<&Select>,
    key_kind: Option<&str>,
) -> Result<Vec<u8>, MergeError> {
    merge_grouped_counts_multi_typed(shards, max_rows, global, key_kind, None)
}

fn merge_grouped_counts_multi_typed(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
    global: Option<&Select>,
    key_kind: Option<&str>,
    group_kinds: Option<&[String]>,
) -> Result<Vec<u8>, MergeError> {
    let group_width = match global.map(|query| &query.projection) {
        Some(Projection::GroupedMultiCount { columns }) => columns.len(),
        _ => 1,
    };
    if !(1..=8).contains(&group_width) {
        return Err(MergeError::InvalidGroupedCount);
    }
    let mut expected_header: Option<&[u8]> = None;
    let mut positions = HashMap::<&[u8], usize>::new();
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
        // Each shard sends exactly N key fields followed by COUNT.
        // TSV-escaped text cannot embed literal tabs in a field.
        if header.len() <= b"\tcount\n".len()
            || !header.ends_with(b"\tcount\n")
            || header.iter().filter(|&&byte| byte == b'\t').count() != group_width
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
            if row.iter().filter(|&&byte| byte == b'\t').count() != group_width {
                return Err(MergeError::InvalidGroupedCount);
            }
            let delimiter = row
                .iter()
                .rposition(|&byte| byte == b'\t')
                .ok_or(MergeError::InvalidGroupedCount)?;
            let key = &row[..delimiter];
            let digits = &row[delimiter + 1..];
            if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
                return Err(MergeError::InvalidGroupedCount);
            }
            let count = std::str::from_utf8(digits)
                .map_err(|_| MergeError::InvalidGroupedCount)?
                .parse::<u64>()
                .map_err(|_| MergeError::CountOverflow)?;
            if let Some(&index) = positions.get(key) {
                let previous = &mut groups[index].1;
                *previous = previous
                    .checked_add(count)
                    .ok_or(MergeError::CountOverflow)?;
            } else {
                let max_groups = if global.is_some() { 250_000 } else { max_rows };
                if groups.len() >= max_groups {
                    return Err(MergeError::CoordinatorLimit);
                }
                positions.insert(key, groups.len());
                groups.push((key, count));
            }
        }
    }
    let header = expected_header.ok_or(MergeError::InvalidGroupedCount)?;
    if let Some(select) = global {
        if let Some(having) = &select.having {
            groups.retain(|(_, count)| passes_count_having(*count, having));
        }
        if select.order_by.is_some() {
            let terms = select.order_terms().collect::<Vec<_>>();
            if terms.iter().all(|order| order.column == "count") {
                groups.sort_by(|left, right| {
                    for order in &terms {
                        let cmp = left.1.cmp(&right.1);
                        let cmp = if order.descending { cmp.reverse() } else { cmp };
                        if cmp != std::cmp::Ordering::Equal { return cmp; }
                    }
                    std::cmp::Ordering::Equal
                });
            } else {
                let kinds = if group_width == 1 {
                    vec![key_kind.ok_or(MergeError::InvalidGroupedCount)?]
                } else {
                    let kinds = group_kinds.ok_or(MergeError::InvalidGroupedCount)?;
                    if kinds.len() != group_width {
                        return Err(MergeError::InvalidGroupedCount);
                    }
                    kinds.iter().map(String::as_str).collect::<Vec<_>>()
                };
                let columns = match &select.projection {
                    Projection::GroupedCount { column } => vec![column.clone()],
                    Projection::GroupedMultiCount { columns } => columns.clone(),
                    _ => return Err(MergeError::InvalidGroupedCount),
                };
                let mut indices = groups.iter().enumerate()
                    .map(|(index, (key, _))| {
                        let fields = key.split(|&byte| byte == b'\t').collect::<Vec<_>>();
                        if fields.len() != kinds.len() {
                            return Err(MergeError::InvalidGroupedCount);
                        }
                        let parsed = fields.into_iter().zip(kinds.iter())
                            .map(|(field, kind)| parse_group_sort_key(field, kind))
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok((index, parsed))
                    })
                    .collect::<Result<Vec<_>, MergeError>>()?;
                indices.sort_by(|(a, a_keys), (b, b_keys)| {
                    for order in &terms {
                        let cmp = if order.column == "count" {
                            groups[*a].1.cmp(&groups[*b].1)
                        } else {
                            let position = columns.iter()
                                .position(|column| *column == order.column)
                                .expect("verified group dimension");
                            compare_group_sort_keys(
                                &a_keys[position], &b_keys[position],
                                order.descending, order.nulls_first,
                            )
                        };
                        let cmp = if order.column == "count" && order.descending {
                            cmp.reverse()
                        } else {
                            cmp
                        };
                        if cmp != std::cmp::Ordering::Equal { return cmp; }
                    }
                    std::cmp::Ordering::Equal
                });
                groups = indices.into_iter().map(|(index, _)| groups[index]).collect();
            }
        }
        groups = groups
            .into_iter()
            .skip(select.offset.unwrap_or(0))
            .take(select.limit.unwrap_or(usize::MAX))
            .collect();
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
    let select = match parse_select(sql) {
        Ok(select)
            if is_safe_scan(&select)
                || is_safe_count(&select)
                || is_safe_count_distinct(&select)
                || is_safe_count_if(&select)
                || is_safe_grouped_count(&select)
                || is_safe_grouped_sum(&select)
                || is_safe_grouped_extremum(&select)
                || is_safe_grouped_multikey_sum(&select)
                || is_safe_grouped_multikey_extremum(&select)
                || is_safe_grouped_fused_metrics(&select)
                || is_safe_grouped_distinct(&select)
                || is_safe_sum(&select)
                || is_safe_extremum(&select) =>
        {
            select
        }
        _ => {
            return Response::text(
                "400 Bad Request",
                "distributed queries require unordered SELECT, COUNT, COUNT(DISTINCT), COUNTIF, integer SUM, MIN, MAX, grouped COUNT, or grouped integer SUM without global clauses\n",
            );
        }
    };
    let count_query = is_safe_count(&select) || is_safe_count_if(&select);
    let distinct_count_column = match &select.projection {
        Projection::CountDistinct { column } => Some(column.as_str()),
        _ => None,
    };
    let grouped_count_query = is_safe_grouped_count(&select);
    let grouped_sum_query = is_safe_grouped_sum(&select);
    let grouped_extremum_query = is_safe_grouped_extremum(&select);
    let grouped_multikey_sum_query = is_safe_grouped_multikey_sum(&select);
    let grouped_multikey_extremum_query = is_safe_grouped_multikey_extremum(&select);
    let grouped_fused_query = is_safe_grouped_fused_metrics(&select);
    let grouped_distinct_query = is_safe_grouped_distinct(&select);
    let sum_query = is_safe_sum(&select);
    let shard_sql = if is_safe_scan(&select) {
        match unpaged_scan_sql(sql, &select) {
            Some(query) => query,
            None => {
                return Response::text("400 Bad Request", "unsupported distributed pagination\n");
            }
        }
    } else if grouped_distinct_query {
        match grouped_distinct_shard_sql(sql, &select) {
            Some(query) => query,
            None => return Response::text("400 Bad Request", "unsupported grouped DISTINCT rewrite
"),
        }
    } else if grouped_sum_query || grouped_extremum_query || grouped_multikey_sum_query {
        match grouped_aggregate_shard_sql(sql, &select) {
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
                function: AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max,
                column,
            },
        ) => Some((table.as_str(), column.as_str())),
        (
            Some(Source::Table(table)),
            Projection::GroupedAggregate {
                function: AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max,
                value_column,
                ..
            },
        ) => Some((table.as_str(), value_column.as_str())),
        (
            Some(Source::Table(table)),
            Projection::GroupedMultiAggregates { items, .. },
        ) if items.len() == 1 && matches!(items[0].0, AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max) =>
            Some((table.as_str(), items[0].1.as_str())),
        _ => None,
    };
    let multi_schema = match &select.projection {
        Projection::GroupedAggregates { items, .. }
        | Projection::GroupedMultiAggregates { items, .. } if grouped_fused_query =>
            Some(items.as_slice()),
        _ => None,
    };
    let group_sort_columns = match &select.projection {
        Projection::GroupedMultiCount { columns } if grouped_count_query
            && select.order_terms().any(|term| term.column != "count") =>
            Some(columns.as_slice()),
        Projection::GroupedMultiAggregates { columns, .. }
            if grouped_multikey_sum_query && select.order_by.is_some() =>
            Some(columns.as_slice()),
        _ => None,
    };
    let group_schema = match (&select.source, &select.projection) {
        (
            Some(Source::Table(table)),
            Projection::GroupedAggregate {
                function: AggregateFunction::Sum | AggregateFunction::Min | AggregateFunction::Max,
                column,
                ..
            },
        ) => Some((table.as_str(), column.as_str())),
        (
            Some(Source::Table(table)),
            Projection::GroupedCount { column },
        ) if select.order_terms().any(|term| term.column == *column) =>
            Some((table.as_str(), column.as_str())),
        (
            Some(Source::Table(table)),
            Projection::GroupedDistinctCount { column, .. },
        ) if select.order_terms().any(|term| term.column == *column) =>
            Some((table.as_str(), column.as_str())),
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
    let results = std::thread::scope(|scope| {
        let shard_query = shard_sql.as_str();
        let handles = cluster
            .peers()
            .iter()
            .map(|&peer| {
                scope.spawn(move || read_shard(
                    peer, shard_query, sum_schema, group_schema, multi_schema,
                    group_sort_columns, table,
                ))
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err("peer query panicked".into()))
            })
            .collect::<Vec<_>>()
    });
    let mut shards = Vec::with_capacity(results.len());
    let mut sum_type: Option<String> = None;
    let mut group_key_kind: Option<String> = None;
    let mut multi_kinds: Option<Vec<String>> = None;
    let mut group_sort_kinds: Option<Vec<String>> = None;
    let mut expected_schema: Option<Vec<u8>> = None;
    for result in results {
        match result {
            Ok(ShardResponse {
                cap,
                body,
                kind,
                key_kind,
                schema,
                multi_kinds: shard_multi_kinds,
                group_sort_kinds: shard_sort_kinds,
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
                if let Some(kinds) = shard_multi_kinds {
                    if multi_kinds.as_ref().is_some_and(|prior| prior != &kinds) {
                        return Response::text(
                            "409 Conflict",
                            "distributed grouped metric column types disagree\n",
                        );
                    }
                    multi_kinds = Some(kinds);
                }
                if let Some(kinds) = shard_sort_kinds {
                    if group_sort_kinds.as_ref().is_some_and(|prior| prior != &kinds) {
                        return Response::text(
                            "409 Conflict", "distributed GROUP BY sort key types disagree\n",
                        );
                    }
                    group_sort_kinds = Some(kinds);
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
        merge_distinct_counts(&shards, column)
    } else if sum_query {
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidSum), |kind| {
                merge_integer_sums(&shards, kind)
            })
    } else if let Some(function) = extrema_function {
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidExtremum), |kind| {
                merge_integer_extrema(&shards, kind, function)
            })
    } else if grouped_sum_query {
        match (sum_type.as_deref(), group_schema, sum_schema) {
            (Some(kind), Some((_, key)), Some((_, value))) => {
                let limit = max_rows.min(250_000);
                if select.having.is_some() || select.order_by.is_some()
                    || select.limit.is_some() || select.offset.is_some()
                {
                    merge_grouped_integer_sums_typed(
                        &shards, limit, kind, key, value, Some(&select), group_key_kind.as_deref(),
                    )
                } else {
                    merge_grouped_integer_sums(&shards, limit, kind, key, value)
                }
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_distinct_query {
        match &select.projection {
            Projection::GroupedDistinctCount { column, value_column } => {
                merge_grouped_distinct_pairs_typed(
                    &shards, max_rows.min(250_000), column, value_column, &select,
                    group_key_kind.as_deref(),
                )
            }
            _ => Err(MergeError::InvalidDistinctCount),
        }
    } else if grouped_fused_query {
        let (keys, items) = match &select.projection {
            Projection::GroupedAggregates { column, items } =>
                (vec![column.clone()], items.as_slice()),
            Projection::GroupedMultiAggregates { columns, items } =>
                (columns.clone(), items.as_slice()),
            _ => unreachable!("validated grouped fused metrics"),
        };
        multi_kinds.as_ref().map_or(Err(MergeError::InvalidGroupedSum), |kinds| {
            merge_grouped_fused_metrics(&shards, max_rows.min(250_000), &keys, items, kinds)
        })
    } else if grouped_multikey_extremum_query {
        match (sum_type.as_deref(), &select.projection) {
            (Some(kind), Projection::GroupedMultiAggregates { columns, items }) => {
                merge_grouped_integer_extrema(
                    &shards, max_rows.min(250_000), kind, &columns.join("\t"), &items[0].1,
                    items[0].0, None,
                )
            }
            _ => Err(MergeError::InvalidExtremum),
        }
    } else if grouped_multikey_sum_query {
        match (sum_type.as_deref(), &select.projection) {
            (Some(kind), Projection::GroupedMultiAggregates { columns, items }) => {
                merge_grouped_integer_sums_multi_typed(
                    &shards, max_rows.min(250_000), kind, &columns.join("\t"), &items[0].1,
                    Some(&select), None, group_sort_kinds.as_deref(),
                )
            }
            _ => Err(MergeError::InvalidGroupedSum),
        }
    } else if grouped_extremum_query {
        match (sum_type.as_deref(), group_schema, sum_schema, &select.projection) {
            (Some(kind), Some((_, key)), Some((_, value)),
                Projection::GroupedAggregate { function, .. }) => {
                merge_grouped_integer_extrema_typed(
                    &shards, max_rows.min(250_000), kind, key, value, *function, Some(&select),
                    group_key_kind.as_deref(),
                )
            }
            _ => Err(MergeError::InvalidExtremum),
        }
    } else if grouped_count_query {
        let limit = max_rows.min(250_000);
        if matches!(select.projection, Projection::GroupedMultiCount { .. })
            || select.having.is_some()
            || select.order_by.is_some()
            || select.limit.is_some()
            || select.offset.is_some()
        {
            merge_grouped_counts_multi_typed(
                &shards, limit, Some(&select), group_key_kind.as_deref(),
                group_sort_kinds.as_deref(),
            )
        } else {
            merge_grouped_counts(&shards, limit)
        }
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
        Err(MergeError::InvalidDistinctCount) => {
            Response::text("502 Bad Gateway", "malformed shard DISTINCT response\n")
        }
        Err(MergeError::InvalidGroupedCount) => {
            Response::text("502 Bad Gateway", "malformed shard GROUP BY response\n")
        }
        Err(MergeError::InvalidGroupedSum) => {
            Response::text("502 Bad Gateway", "malformed shard grouped SUM response\n")
        }
        Err(MergeError::InvalidSum) => {
            Response::text("502 Bad Gateway", "malformed shard SUM response\n")
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
mod tests {
    use super::*;

    #[test]
    fn extracts_column_types_from_one_validated_schema_response() {
        let response = remote::PeerResponse {
            status: 200,
            content_type: "application/json; charset=utf-8".into(),
            body: b"{\"rows\":12,\"columns\":[{\"name\":\"region\",\"type\":\"String\"},{\"name\":\"amount\",\"type\":\"Int64\"}]}\n".to_vec(),
        };
        assert!(schema_signature(&response).is_ok());
        assert_eq!(shard_column_kind(&response, "region").unwrap(), "String");
        assert_eq!(shard_integer_kind(&response, "amount").unwrap(), "Int64");
        assert!(shard_integer_kind(&response, "region").is_err());
    }

    fn shard(cap: usize, data: &str) -> (usize, Vec<u8>) {
        (cap, data.as_bytes().to_vec())
    }


    #[test]
    fn globally_ranks_grouped_counts_after_reducing_shards() {
        let sql = "SELECT region, count(*) FROM events GROUP BY region HAVING count(*) >= 3 ORDER BY count DESC LIMIT 2 OFFSET 1";
        let select = parse_select(sql).unwrap();
        assert!(is_safe_grouped_count(&select));
        assert_eq!(
            grouped_count_shard_sql(sql, &select).as_deref(),
            Some("SELECT region, count(*) FROM events GROUP BY region")
        );
        let shards = [
            shard(10, "region\tcount\nwest\t2\neast\t5\nsouth\t1\n"),
            shard(10, "region\tcount\neast\t1\nwest\t4\nnorth\t9\n"),
        ];
        // Global counts: north=9, west=6, east=6, south=1.
        // HAVING excludes south, stable ties keep west ahead of east,
        // OFFSET discards north and LIMIT returns the two remaining groups.
        assert_eq!(
            merge_grouped_counts_query(&shards, 2, Some(&select)).unwrap(),
            b"region\tcount\nwest\t6\neast\t6\n"
        );
        // Never push a local top-k to shards: it may remove a global winner.
        assert!(!is_safe_grouped_count(
            &parse_select("SELECT region, count(*) FROM events GROUP BY region ORDER BY region LIMIT 2").unwrap()
        ));
    }

    #[test]
    fn grouped_global_having_rejects_capped_and_malformed_shards() {
        let query = parse_select(
            "SELECT region, count(*) FROM events GROUP BY region HAVING count(*) > 0 LIMIT 1",
        )
        .unwrap();
        assert_eq!(
            merge_grouped_counts_query(&[shard(2, "region\tcount\na\t1\nb\t2\n")], 1, Some(&query)),
            Err(MergeError::ShardRowLimit)
        );
        assert_eq!(
            merge_grouped_counts_query(&[shard(10, "region\tcount\na\tnot-a-number\n")], 1, Some(&query)),
            Err(MergeError::InvalidGroupedCount)
        );
    }


    #[test]
    fn distributed_global_grouped_sum_ranks_exact_combined_partials() {
        let sql = "SELECT region, sum(amount) FROM events GROUP BY region HAVING sum(amount) >= 5 ORDER BY 2 DESC LIMIT 2 OFFSET 1";
        let select = parse_select(sql).unwrap();
        assert!(is_safe_grouped_sum(&select));
        assert_eq!(
            grouped_aggregate_shard_sql(sql, &select).as_deref(),
            Some("SELECT region, sum(amount) FROM events GROUP BY region")
        );
        let shards = [
            shard(10, "region\tsum(amount)\na\t4\nb\t10\nc\t3\n"),
            shard(10, "region\tsum(amount)\na\t8\nb\t0\nc\t2\nd\t20\n"),
        ];
        // d=20, a=12, b=10, c=5. Sort globally, then offset and limit.
        assert_eq!(
            merge_grouped_integer_sums_query(&shards, 2, "UInt64", "region", "amount", Some(&select)).unwrap(),
            b"region\tsum(amount)\na\t12\nb\t10\n"
        );
        assert!(!is_safe_grouped_sum(
            &parse_select("SELECT region, sum(amount) FROM events GROUP BY region ORDER BY region").unwrap()
        ));
    }

    #[test]
    fn distributed_grouped_sum_keeps_null_and_checks_capped_shards() {
        let query = parse_select("SELECT region, sum(amount) FROM events GROUP BY region ORDER BY 2 DESC").unwrap();
        let shards = [shard(10, "region\tsum(amount)\na\tNULL\nb\t-12\n"),
                      shard(10, "region\tsum(amount)\na\tNULL\nb\t5\n")];
        assert_eq!(
            merge_grouped_integer_sums_query(&shards, 10, "Int64", "region", "amount", Some(&query)).unwrap(),
            b"region\tsum(amount)\nb\t-7\na\tNULL\n"
        );
        assert_eq!(
            merge_grouped_integer_sums_query(&[shard(2, "region\tsum(amount)\na\t1\nb\t2\n")], 1, "Int64", "region", "amount", Some(&query)),
            Err(MergeError::ShardRowLimit)
        );
    }

    #[test]
    fn distributed_multikey_grouped_count_reduces_composite_keys_globally() {
        let sql = "SELECT region, category, count(*) FROM events GROUP BY region, category HAVING count(*) >= 3 ORDER BY count DESC LIMIT 2";
        let query = parse_select(sql).unwrap();
        assert!(is_safe_grouped_count(&query));
        assert_eq!(
            grouped_count_shard_sql(sql, &query).as_deref(),
            Some("SELECT region, category, count(*) FROM events GROUP BY region, category")
        );
        let shards = [
            shard(10, "region\tcategory\tcount\neu\tfood\t2\nus\tfood\t2\n"),
            shard(10, "region\tcategory\tcount\neu\tfood\t4\nus\tfood\t3\neu\tbooks\t4\n"),
        ];
        assert_eq!(
            merge_grouped_counts_query(&shards, 2, Some(&query)).unwrap(),
            b"region\tcategory\tcount\neu\tfood\t6\nus\tfood\t5\n"
        );
        let malformed = [shard(10, "region\tcategory\tcount\neu\tfood\t2\n"),
                         shard(10, "region\tcategory\tcount\neu\tbroken\n")];
        assert_eq!(merge_grouped_counts_query(&malformed, 2, Some(&query)),
                   Err(MergeError::InvalidGroupedCount));
    }

    #[test]
    fn distributed_grouped_minmax_reduces_exact_numeric_extrema() {
        let query = parse_select(
            "SELECT region, max(amount) FROM events GROUP BY region ORDER BY 2 DESC LIMIT 2",
        ).unwrap();
        assert!(is_safe_grouped_extremum(&query));
        let shards = [
            shard(10, "region\tmax(amount)\na\t3\nb\t9\nc\tNULL\n"),
            shard(10, "region\tmax(amount)\na\t7\nb\t4\nc\t2\n"),
        ];
        assert_eq!(
            merge_grouped_integer_extrema(&shards, 2, "Int64", "region", "amount",
                AggregateFunction::Max, Some(&query)).unwrap(),
            b"region\tmax(amount)\nb\t9\na\t7\n"
        );
        let min_query = parse_select(
            "SELECT region, min(amount) FROM events GROUP BY region HAVING min(amount) >= 3 ORDER BY 2 ASC",
        ).unwrap();
        let min_shards = [
            shard(10, "region\tmin(amount)\na\t3\nb\t9\nc\tNULL\n"),
            shard(10, "region\tmin(amount)\na\t7\nb\t4\nc\t2\n"),
        ];
        assert_eq!(
            merge_grouped_integer_extrema(&min_shards, 2, "Int64", "region", "amount",
                AggregateFunction::Min, Some(&min_query)).unwrap(),
            b"region\tmin(amount)\na\t3\nb\t4\n"
        );
    }

    #[test]
    fn distributed_multidimensional_sum_reduces_shared_composite_keys() {
        let query = parse_select(
            "SELECT region, category, sum(revenue) FROM events GROUP BY region, category",
        ).unwrap();
        assert!(is_safe_grouped_multikey_sum(&query));
        let shards = [
            shard(10, "region\tcategory\tsum(revenue)\neu\ttools\t11\nus\ttools\t2\n"),
            shard(10, "region\tcategory\tsum(revenue)\neu\ttools\t6\nus\ttools\tNULL\neu\tbooks\t3\n"),
        ];
        assert_eq!(
            merge_grouped_integer_sums(&shards, 10, "UInt64", "region\tcategory", "revenue").unwrap(),
            b"region\tcategory\tsum(revenue)\neu\ttools\t17\nus\ttools\t2\neu\tbooks\t3\n"
        );
        let malformed = [shard(10, "region\tcategory\tsum(revenue)\neu\ttools\t5\n"),
            shard(10, "region\tcategory\tsum(revenue)\neu\t3\n")];
        assert_eq!(
            merge_grouped_integer_sums(&malformed, 10, "UInt64", "region\tcategory", "revenue"),
            Err(MergeError::InvalidGroupedSum)
        );
    }

    #[test]
    fn distributed_multidimensional_extrema_preserve_composite_groups() {
        let query = parse_select(
            "SELECT country, product, max(price) FROM orders GROUP BY country, product",
        ).unwrap();
        assert!(is_safe_grouped_multikey_extremum(&query));
        let shards = [
            shard(10, "country\tproduct\tmax(price)\nde\tcars\t9\nfr\tcars\t4\n"),
            shard(10, "country\tproduct\tmax(price)\nde\tcars\t11\nfr\tcars\t1\nde\tbikes\t2\n"),
        ];
        assert_eq!(
            merge_grouped_integer_extrema(&shards, 10, "UInt64", "country\tproduct", "price",
                AggregateFunction::Max, None).unwrap(),
            b"country\tproduct\tmax(price)\nde\tcars\t11\nfr\tcars\t4\nde\tbikes\t2\n"
        );
        assert_eq!(
            merge_grouped_integer_extrema(
                &[shard(10, "country\tproduct\tmax(price)\nde\t1\n")], 10, "UInt64",
                "country\tproduct", "price", AggregateFunction::Max, None,
            ),
            Err(MergeError::InvalidExtremum)
        );
    }

    #[test]
    fn distributed_grouped_fused_metrics_reduce_each_partial_correctly() {
        let query = parse_select(
            "SELECT region, sum(revenue), count(revenue), min(revenue), max(revenue) FROM sales GROUP BY region",
        ).unwrap();
        assert!(is_safe_grouped_fused_metrics(&query));
        let items = vec![
            (AggregateFunction::Sum, "revenue".to_owned()),
            (AggregateFunction::Count, "revenue".to_owned()),
            (AggregateFunction::Min, "revenue".to_owned()),
            (AggregateFunction::Max, "revenue".to_owned()),
        ];
        let shards = [
            shard(10, "region\tsum(revenue)\tcount(revenue)\tmin(revenue)\tmax(revenue)\neu\t4\t2\t1\t3\nus\tNULL\t0\tNULL\tNULL\n"),
            shard(10, "region\tsum(revenue)\tcount(revenue)\tmin(revenue)\tmax(revenue)\neu\t7\t1\t7\t7\nus\t8\t1\t8\t8\n"),
        ];
        assert_eq!(
            merge_grouped_fused_metrics(&shards, 10, &["region".into()], &items,
                &["Int64".into(), "Int64".into(), "Int64".into(), "Int64".into()]).unwrap(),
            b"region\tsum(revenue)\tcount(revenue)\tmin(revenue)\tmax(revenue)\neu\t11\t3\t1\t7\nus\t8\t1\t8\t8\n"
        );
    }

    #[test]
    fn distributed_fused_multikey_metrics_validate_row_width() {
        let query = parse_select(
            "SELECT region, category, sum(price), count(price) FROM sales GROUP BY region, category",
        ).unwrap();
        assert!(is_safe_grouped_fused_metrics(&query));
        let items = vec![(AggregateFunction::Sum, "price".into()),
                         (AggregateFunction::Count, "price".into())];
        let shards = [
            shard(10, "region\tcategory\tsum(price)\tcount(price)\neu\tbook\t5\t2\n"),
            shard(10, "region\tcategory\tsum(price)\tcount(price)\neu\tbook\t4\t1\n"),
        ];
        assert_eq!(
            merge_grouped_fused_metrics(&shards, 10, &["region".into(), "category".into()],
                &items, &["UInt64".into(), "UInt64".into()]).unwrap(),
            b"region\tcategory\tsum(price)\tcount(price)\neu\tbook\t9\t3\n"
        );
        assert_eq!(
            merge_grouped_fused_metrics(
                &[shard(10, "region\tcategory\tsum(price)\tcount(price)\neu\tbook\t5\n")],
                10, &["region".into(), "category".into()],
                &items, &["UInt64".into(), "UInt64".into()]),
            Err(MergeError::InvalidGroupedSum)
        );
    }

    #[test]
    fn distributed_grouped_distinct_deduplicates_cross_shard_pairs() {
        let sql = "SELECT region, count(DISTINCT customer) FROM sales GROUP BY region HAVING count(DISTINCT customer) >= 2 ORDER BY count DESC LIMIT 2";
        let query = parse_select(sql).unwrap();
        assert!(is_safe_grouped_distinct(&query));
        assert_eq!(
            grouped_distinct_shard_sql(sql, &query).as_deref(),
            Some("SELECT DISTINCT region, customer FROM sales")
        );
        let shards = [
            shard(10, "region\tcustomer\neu\talice\neu\tbob\nus\t\\N\n"),
            shard(10, "region\tcustomer\neu\tbob\neu\tcharlie\nus\ttaylor\n"),
            shard(10, "region\tcustomer\nus\ttaylor\nus\talex\n"),
        ];
        assert_eq!(
            merge_grouped_distinct_pairs(&shards, 2, "region", "customer", &query).unwrap(),
            b"region\tcountDistinct(customer)\neu\t3\nus\t2\n"
        );
        assert_eq!(
            merge_grouped_distinct_pairs(
                &[shard(2, "region\tcustomer\neu\talice\neu\tbob\n")],
                10, "region", "customer", &query,
            ),
            Err(MergeError::ShardRowLimit)
        );
    }

    #[test]
    fn distributed_numeric_group_order_uses_numeric_values_not_text() {
        let query = parse_select(
            "SELECT customer, count(*) FROM events GROUP BY customer ORDER BY customer ASC LIMIT 2",
        ).unwrap();
        assert!(is_safe_grouped_count(&query));
        let shards = [shard(10, "customer\tcount\n10\t2\n2\t1\n1\t4\n")];
        assert_eq!(
            merge_grouped_counts_typed(&shards, 2, Some(&query), Some("UInt64")).unwrap(),
            b"customer\tcount\n1\t4\n2\t1\n"
        );
        let descending = parse_select(
            "SELECT customer, count(*) FROM events GROUP BY customer ORDER BY count DESC, customer ASC LIMIT 2",
        ).unwrap();
        let shards = [shard(10, "customer\tcount\n10\t2\n2\t2\n1\t4\n")];
        assert_eq!(
            merge_grouped_counts_typed(&shards, 2, Some(&descending), Some("UInt64")).unwrap(),
            b"customer\tcount\n1\t4\n2\t2\n"
        );
    }

    #[test]
    fn distributed_text_group_order_decodes_escape_sequences() {
        let query = parse_select(
            "SELECT category, count(*) FROM events GROUP BY category ORDER BY category ASC",
        ).unwrap();
        let shards = [shard(10, "category\tcount\na\\tb\t1\na0\t2\n")];
        assert_eq!(
            merge_grouped_counts_typed(&shards, 10, Some(&query), Some("String")).unwrap(),
            b"category\tcount\na\\tb\t1\na0\t2\n"
        );
        assert_eq!(
            merge_grouped_counts_typed(&[shard(10, "category\tcount\n\\q\t1\n")], 10,
                Some(&query), Some("String")),
            Err(MergeError::InvalidGroupedCount)
        );
    }

    #[test]
    fn distributed_grouped_sum_orders_by_typed_key_then_value() {
        let sql = "SELECT customer, sum(amount) FROM events GROUP BY customer ORDER BY customer ASC, 2 DESC LIMIT 2";
        let query = parse_select(sql).unwrap();
        assert!(is_safe_grouped_sum(&query));
        let shards = [
            shard(10, "customer\tsum(amount)\n10\t9\n2\t2\n1\t4\n"),
            shard(10, "customer\tsum(amount)\n10\t5\n2\t8\n1\t1\n"),
        ];
        assert_eq!(
            merge_grouped_integer_sums_typed(
                &shards, 2, "UInt64", "customer", "amount", Some(&query), Some("UInt64"),
            ).unwrap(),
            b"customer\tsum(amount)\n1\t5\n2\t10\n"
        );
        let by_sum = parse_select(
            "SELECT customer, sum(amount) FROM events GROUP BY customer ORDER BY 2 DESC, customer ASC LIMIT 2",
        ).unwrap();
        assert_eq!(
            merge_grouped_integer_sums_typed(
                &shards, 2, "UInt64", "customer", "amount", Some(&by_sum), Some("UInt64"),
            ).unwrap(),
            b"customer\tsum(amount)\n10\t14\n2\t10\n"
        );
    }

    #[test]
    fn distributed_grouped_extrema_orders_typed_dimensions_and_metrics() {
        let sql = "SELECT city, max(amount) FROM events GROUP BY city ORDER BY city ASC, 2 DESC LIMIT 2";
        let query = parse_select(sql).unwrap();
        assert!(is_safe_grouped_extremum(&query));
        let shards = [
            shard(10, "city\tmax(amount)\n10\t20\n2\t1\n1\t7\n"),
            shard(10, "city\tmax(amount)\n10\t12\n2\t11\n1\t4\n"),
        ];
        assert_eq!(
            merge_grouped_integer_extrema_typed(
                &shards, 2, "UInt64", "city", "amount", AggregateFunction::Max,
                Some(&query), Some("UInt32"),
            ).unwrap(),
            b"city\tmax(amount)\n1\t7\n2\t11\n"
        );
        let key_only = parse_select(
            "SELECT city, min(amount) FROM events GROUP BY city ORDER BY city DESC LIMIT 1",
        ).unwrap();
        assert!(is_safe_grouped_extremum(&key_only));
    }

    #[test]
    fn global_grouped_distinct_can_sort_by_typed_key_with_count_tiebreak() {
        let sql = "SELECT customer, count(DISTINCT product) FROM sales GROUP BY customer ORDER BY customer ASC LIMIT 2";
        let query = parse_select(sql).unwrap();
        assert!(is_safe_grouped_distinct(&query));
        assert_eq!(grouped_distinct_shard_sql(sql, &query).as_deref(),
                   Some("SELECT DISTINCT customer, product FROM sales"));
        let shards = [
            shard(10, "customer\tproduct\n10\tx\n2\tx\n1\ty\n"),
            shard(10, "customer\tproduct\n10\ty\n2\tx\n1\tz\n"),
        ];
        assert_eq!(
            merge_grouped_distinct_pairs_typed(&shards, 2, "customer", "product",
                &query, Some("UInt64")).unwrap(),
            b"customer\tcountDistinct(product)\n1\t2\n2\t1\n"
        );
        let count_first = parse_select(
            "SELECT customer, count(DISTINCT product) FROM sales GROUP BY customer ORDER BY count DESC, customer ASC LIMIT 2",
        ).unwrap();
        assert_eq!(
            merge_grouped_distinct_pairs_typed(&shards, 2, "customer", "product",
                &count_first, Some("UInt64")).unwrap(),
            b"customer\tcountDistinct(product)\n1\t2\n10\t2\n"
        );
    }

    #[test]
    fn distributed_multikey_count_orders_numeric_and_text_dimensions() {
        let sql = "SELECT region, category, count(*) FROM sales GROUP BY region, category ORDER BY region ASC, category DESC LIMIT 3";
        let query = parse_select(sql).unwrap();
        assert!(is_safe_grouped_count(&query));
        let shards = [
            shard(10, "region\tcategory\tcount\n10\ta\t4\n2\ta\t2\n"),
            shard(10, "region\tcategory\tcount\n2\tb\t1\n10\ta\t3\n"),
        ];
        assert_eq!(
            merge_grouped_counts_multi_typed(&shards, 3, Some(&query), None,
                Some(&["UInt64".into(), "String".into()])).unwrap(),
            b"region\tcategory\tcount\n2\tb\t1\n2\ta\t2\n10\ta\t7\n"
        );
        assert_eq!(
            merge_grouped_counts_multi_typed(&shards, 3, Some(&query), None,
                Some(&["String".into()])),
            Err(MergeError::InvalidGroupedCount)
        );
    }

    #[test]
    fn distributed_multikey_sum_global_topk_ranks_all_shard_groups() {
        let sql = "SELECT region, category, sum(amount) FROM sales GROUP BY region, category ORDER BY region ASC, category DESC LIMIT 2 OFFSET 1";
        let query = parse_select(sql).unwrap();
        assert!(is_safe_grouped_multikey_sum(&query));
        assert_eq!(
            grouped_aggregate_shard_sql(sql, &query).as_deref(),
            Some("SELECT region, category, sum(amount) FROM sales GROUP BY region, category")
        );
        let shards = [
            shard(10, "region\tcategory\tsum(amount)\n10\ta\t4\n2\tb\t2\n2\ta\t5\n"),
            shard(10, "region\tcategory\tsum(amount)\n10\ta\t6\n2\tb\t3\n2\tc\t7\n"),
        ];
        assert_eq!(
            merge_grouped_integer_sums_multi_typed(
                &shards, 2, "UInt64", "region\tcategory", "amount",
                Some(&query), None, Some(&["UInt64".into(), "String".into()]),
            ).unwrap(),
            b"region\tcategory\tsum(amount)\n2\tb\t5\n2\ta\t5\n"
        );
    }
    #[test]
    fn distributed_count_distinct_uses_global_unique_values() {
        let query = "SELECT count(DISTINCT category) FROM events WHERE id >= 5";
        let parsed = parse_select(query).unwrap();
        assert!(is_safe_count_distinct(&parsed));
        assert_eq!(
            distinct_shard_sql(query, &parsed).as_deref(),
            Some("SELECT DISTINCT category FROM events WHERE id >= 5")
        );
        assert!(!is_safe_count_distinct(
            &parse_select("SELECT count(DISTINCT category) FROM events LIMIT 1").unwrap()
        ));
        let shards = [
            shard(10, "category\nred\nblue\n\\N\n"),
            shard(10, "category\nblue\ngreen\n"),
            shard(10, "category\nred\n"),
        ];
        assert_eq!(
            merge_distinct_counts(&shards, "category").unwrap(),
            b"count(DISTINCT category)\n3\n"
        );
        assert_eq!(
            merge_distinct_counts(&[shard(2, "category\na\nb\n")], "category"),
            Err(MergeError::ShardRowLimit)
        );
        assert_eq!(
            merge_distinct_counts(&[shard(10, "other\na\n")], "category"),
            Err(MergeError::InvalidDistinctCount)
        );
        assert_eq!(
            merge_distinct_counts(&[shard(10, "category\na")], "category"),
            Err(MergeError::InvalidDistinctCount)
        );
    }

    #[test]
    fn merges_rows_in_peer_order_without_duplicate_headers() {
        let shards = [
            shard(10, "id\tname\n1\tAda\n2\tLin\n"),
            shard(10, "id\tname\n"),
            shard(10, "id\tname\n3\tGrace\n"),
        ];
        assert_eq!(
            merge_tsv(&shards, 3).unwrap(),
            b"id\tname\n1\tAda\n2\tLin\n3\tGrace\n"
        );
    }

    #[test]
    fn applies_global_limit_and_offset_in_peer_order() {
        let shards = [shard(8, "id\n1\n2\n3\n"), shard(8, "id\n4\n5\n")];
        assert_eq!(
            merge_tsv_page(&shards, 2, 2, Some(2)).unwrap(),
            b"id\n3\n4\n"
        );
        assert_eq!(merge_tsv_page(&shards, 2, 3, None).unwrap(), b"id\n4\n5\n");
        assert_eq!(merge_tsv_page(&shards, 2, 0, Some(0)).unwrap(), b"id\n");
        assert_eq!(merge_tsv_page(&shards, 2, 999, Some(2)).unwrap(), b"id\n");
        assert_eq!(
            merge_tsv_page(&shards, 1, 0, Some(2)),
            Err(MergeError::CoordinatorLimit)
        );
        assert_eq!(
            merge_tsv_page(&[shard(2, "id\n1\n2\n")], 10, 0, Some(1)),
            Err(MergeError::ShardRowLimit)
        );
    }

    #[test]
    fn strips_only_validated_global_pagination_from_shard_sql() {
        for sql in [
            "SELECT id FROM events LIMIT 2",
            "SELECT id FROM events LIMIT 2 OFFSET 3",
        ] {
            let parsed = parse_select(sql).unwrap();
            assert!(is_safe_scan(&parsed), "{sql}");
            assert_eq!(
                unpaged_scan_sql(sql, &parsed),
                Some("SELECT id FROM events".to_string()),
                "{sql}"
            );
        }
        let sql = "SELECT name FROM events WHERE name = 'LIMIT 2' LIMIT 1";
        let parsed = parse_select(sql).unwrap();
        assert_eq!(
            unpaged_scan_sql(sql, &parsed),
            Some("SELECT name FROM events WHERE name = 'LIMIT 2'".into())
        );
    }

    #[test]
    fn refuses_incomplete_or_incompatible_shard_outputs() {
        assert_eq!(
            merge_tsv(&[shard(10, "id\n1\n"), shard(10, "name\n2\n")], 5),
            Err(MergeError::SchemaMismatch)
        );
        assert_eq!(
            merge_tsv(&[shard(1, "id\n1\n")], 10),
            Err(MergeError::ShardRowLimit)
        );
        assert_eq!(
            merge_tsv(&[shard(10, "id\n1\n2\n")], 1),
            Err(MergeError::CoordinatorLimit)
        );
        assert_eq!(
            merge_tsv(&[shard(10, "id\n1")], 10),
            Err(MergeError::InvalidResponse)
        );
    }

    #[test]
    fn rejects_non_decomposable_sql() {
        for sql in [
            "SELECT count(*) FROM events",
            "SELECT id FROM events ORDER BY id",
            "SELECT DISTINCT id FROM events",
            "SELECT number FROM numbers(10)",
        ] {
            assert!(
                parse_select(sql).map_or(true, |select| !is_safe_scan(&select)),
                "{sql}"
            );
        }
        assert!(is_safe_scan(
            &parse_select("SELECT id FROM events WHERE id > 2").unwrap()
        ));
    }

    #[test]
    fn compares_shard_column_types_independently_of_row_counts() {
        let response = |body: &'static [u8]| remote::PeerResponse {
            status: 200,
            content_type: "application/json; charset=utf-8".into(),
            body: body.to_vec(),
        };
        let a = schema_signature(&response(
            b"{\"rows\":4,\"columns\":[{\"name\":\"id\",\"type\":\"UInt64\"}]}\n",
        ))
        .unwrap();
        let b = schema_signature(&response(
            b"{\"rows\":900,\"columns\":[{\"name\":\"id\",\"type\":\"UInt64\"}]}\n",
        ))
        .unwrap();
        let different = schema_signature(&response(
            b"{\"rows\":4,\"columns\":[{\"name\":\"id\",\"type\":\"String\"}]}\n",
        ))
        .unwrap();
        assert_eq!(a, b);
        assert_ne!(a, different);
        assert!(schema_signature(&response(b"bad")).is_err());
    }

    #[test]
    fn sums_counts_and_nullable_column_counts_safely() {
        assert!(is_safe_count(
            &parse_select("SELECT count(*) FROM events").unwrap()
        ));
        assert!(is_safe_count(
            &parse_select("SELECT count(name) FROM events WHERE id > 10").unwrap()
        ));
        assert!(!is_safe_count(
            &parse_select("SELECT count(DISTINCT name) FROM events").unwrap()
        ));
        assert_eq!(
            sum_shard_counts(&[
                shard(10, "count\n20\n"),
                shard(10, "count\n0\n"),
                shard(10, "count\n300\n"),
            ])
            .unwrap(),
            b"count\n320\n"
        );
    }

    #[test]
    fn reduces_count_if_for_disjoint_shards() {
        assert!(is_safe_count_if(
            &parse_select("SELECT countIf(flag) FROM events WHERE id > 0").unwrap()
        ));
        for sql in [
            "SELECT countIf(flag) FROM events LIMIT 1",
            "SELECT countIf(flag) FROM numbers(10)",
            "SELECT countIf(flag) FROM events OFFSET 1",
            "SELECT sum(flag) FROM events",
        ] {
            assert!(
                parse_select(sql).map_or(true, |query| !is_safe_count_if(&query)),
                "{sql}"
            );
        }
        assert_eq!(
            sum_shard_counts(&[
                shard(1, "countIf(flag)\n2\n"),
                shard(1, "countIf(flag)\n0\n"),
                shard(1, "countIf(flag)\n3\n"),
            ])
            .unwrap(),
            b"countIf(flag)\n5\n"
        );
        assert_eq!(
            sum_shard_counts(&[
                shard(1, "countIf(flag)\n1\n"),
                shard(1, "countIf(other)\n2\n"),
            ]),
            Err(MergeError::SchemaMismatch)
        );
    }

    #[test]
    fn refuses_invalid_count_responses_and_overflow() {
        assert_eq!(
            sum_shard_counts(&[shard(10, "count\n2\n"), shard(10, "sum\n3\n")]),
            Err(MergeError::SchemaMismatch)
        );
        for invalid in ["count\n-2\n", "count\nNaN\n", "count\n3\n4\n", "count\n3"] {
            assert_eq!(
                sum_shard_counts(&[shard(10, invalid)]),
                Err(MergeError::InvalidCount)
            );
        }
        assert_eq!(
            sum_shard_counts(&[
                shard(10, "count\n18446744073709551615\n"),
                shard(10, "count\n1\n"),
            ]),
            Err(MergeError::CountOverflow)
        );
    }

    #[test]
    fn reduces_grouped_count_across_disjoint_shards() {
        assert!(is_safe_grouped_count(
            &parse_select("SELECT kind, count(*) FROM events GROUP BY kind").unwrap()
        ));
        assert!(is_safe_grouped_count(
            &parse_select("SELECT kind, count(*) FROM events WHERE id >= 1 GROUP BY kind").unwrap()
        ));
        let shards = [
            shard(10, "kind\tcount\nalpha\t2\nbeta\t3\n"),
            shard(10, "kind\tcount\nbeta\t4\n\t1\n"),
            shard(10, "kind\tcount\nalpha\t5\n"),
        ];
        assert_eq!(
            merge_grouped_counts(&shards, 3).unwrap(),
            b"kind\tcount\nalpha\t7\nbeta\t7\n\t1\n"
        );
        assert_eq!(
            merge_grouped_counts(&[shard(10, "kind\tcount\n")], 10).unwrap(),
            b"kind\tcount\n"
        );
    }

    #[test]
    fn rejects_unsafe_grouped_count_and_incomplete_shard_output() {
        for sql in [
            "SELECT kind, count(*) FROM events GROUP BY kind ORDER BY kind",
            "SELECT kind, count(*) FROM events GROUP BY kind LIMIT 1",
            "SELECT kind, count(*) FROM events GROUP BY kind HAVING count > 1",
            "SELECT id, sum(value) FROM events GROUP BY id",
            "SELECT kind, count(*) FROM numbers(10) GROUP BY kind",
        ] {
            assert!(
                parse_select(sql).map_or(true, |select| !is_safe_grouped_count(&select)),
                "{sql}"
            );
        }
        assert_eq!(
            merge_grouped_counts(&[shard(10, "kind\tcount\na\t2")], 10),
            Err(MergeError::InvalidGroupedCount)
        );
        assert_eq!(
            merge_grouped_counts(&[shard(10, "kind\tcount\na\t2\t3\n")], 10),
            Err(MergeError::InvalidGroupedCount)
        );
        assert_eq!(
            merge_grouped_counts(&[shard(10, "kind\tcount\na\t-1\n")], 10),
            Err(MergeError::InvalidGroupedCount)
        );
        assert_eq!(
            merge_grouped_counts(
                &[
                    shard(10, "kind\tcount\na\t1\n"),
                    shard(10, "id\tcount\na\t2\n")
                ],
                10
            ),
            Err(MergeError::SchemaMismatch)
        );
        assert_eq!(
            merge_grouped_counts(&[shard(1, "kind\tcount\na\t1\n")], 10),
            Err(MergeError::ShardRowLimit)
        );
        assert_eq!(
            merge_grouped_counts(&[shard(10, "kind\tcount\na\t1\nb\t2\n")], 1),
            Err(MergeError::CoordinatorLimit)
        );
        assert_eq!(
            merge_grouped_counts(
                &[
                    shard(10, "kind\tcount\na\t18446744073709551615\n"),
                    shard(10, "kind\tcount\na\t1\n"),
                ],
                10
            ),
            Err(MergeError::CountOverflow)
        );
    }

    #[test]
    fn allows_distributed_integer_min_and_max_only_without_global_clauses() {
        for sql in [
            "SELECT min(amount) FROM events WHERE id > 3",
            "SELECT max(amount) FROM events",
        ] {
            assert!(is_safe_extremum(&parse_select(sql).unwrap()), "{sql}");
        }
        for sql in [
            "SELECT max(amount) FROM events LIMIT 1",
            "SELECT min(amount) FROM numbers(10)",
            "SELECT min(amount) FROM events OFFSET 1",
            "SELECT avg(amount) FROM events",
        ] {
            assert!(
                parse_select(sql).map_or(true, |query| !is_safe_extremum(&query)),
                "{sql}"
            );
        }
    }

    #[test]
    fn reduces_signed_and_unsigned_extrema_with_null_shards() {
        let shards = [
            shard(10, "min(amount)\nNULL\n"),
            shard(10, "min(amount)\n-9\n"),
            shard(10, "min(amount)\n13\n"),
        ];
        assert_eq!(
            merge_integer_extrema(&shards, "Nullable(Int64)", AggregateFunction::Min).unwrap(),
            b"min(amount)\n-9\n"
        );
        let shards = [
            shard(10, "max(amount)\n18446744073709551615\n"),
            shard(10, "max(amount)\n2\n"),
        ];
        assert_eq!(
            merge_integer_extrema(&shards, "UInt64", AggregateFunction::Max).unwrap(),
            b"max(amount)\n18446744073709551615\n"
        );
        assert_eq!(
            merge_integer_extrema(
                &[shard(10, "min(amount)\nNULL\n")],
                "Int64",
                AggregateFunction::Min,
            )
            .unwrap(),
            b"min(amount)\nNULL\n"
        );
    }

    #[test]
    fn rejects_incompatible_or_malformed_extrema() {
        for bad in [
            "min(amount)\n",
            "min(amount)\n2.5\n",
            "min(amount)\n2\n3\n",
            "min(amount)\n2",
        ] {
            assert_eq!(
                merge_integer_extrema(&[shard(10, bad)], "Int64", AggregateFunction::Min),
                Err(MergeError::InvalidExtremum),
            );
        }
        assert_eq!(
            merge_integer_extrema(
                &[shard(10, "min(amount)\n1\n"), shard(10, "min(other)\n2\n"),],
                "Int64",
                AggregateFunction::Min,
            ),
            Err(MergeError::SchemaMismatch)
        );
        assert_eq!(
            merge_integer_extrema(
                &[shard(10, "max(amount)\n-1\n")],
                "UInt64",
                AggregateFunction::Max,
            ),
            Err(MergeError::InvalidExtremum)
        );
    }

    #[test]
    fn accepts_grouped_integer_sum_without_global_clauses() {
        assert!(is_safe_grouped_sum(
            &parse_select("SELECT region, sum(amount) FROM events WHERE id > 0 GROUP BY region")
                .unwrap()
        ));
        for sql in [
            "SELECT region, sum(amount) FROM events GROUP BY region LIMIT 1",
            "SELECT region, avg(amount) FROM events GROUP BY region",
            "SELECT region, sum(amount) FROM numbers(10) GROUP BY region",
            "SELECT region, sum(amount) FROM events GROUP BY region ORDER BY region",
        ] {
            assert!(
                parse_select(sql).map_or(true, |query| !is_safe_grouped_sum(&query)),
                "{sql}"
            );
        }
        assert!(group_key_type("String"));
        assert!(group_key_type("Nullable(UUID)"));
        assert!(group_key_type("UInt64"));
        assert!(!group_key_type("Float64"));
    }

    #[test]
    fn merges_grouped_sum_with_overlapping_keys_and_null_values() {
        let shards = [
            shard(10, "region\tsum(amount)\nnorth\t15\nsouth\tNULL\n"),
            shard(10, "region\tsum(amount)\nnorth\t-7\nsouth\t5\n"),
            shard(10, "region\tsum(amount)\nnorth\tNULL\neast\t9\n"),
        ];
        assert_eq!(
            merge_grouped_integer_sums(&shards, 3, "Nullable(Int64)", "region", "amount").unwrap(),
            b"region\tsum(amount)\nnorth\t8\nsouth\t5\neast\t9\n"
        );
        assert_eq!(
            merge_grouped_integer_sums(
                &[shard(10, "id\tsum(amount)\n1\tNULL\n")],
                1,
                "UInt64",
                "id",
                "amount"
            )
            .unwrap(),
            b"id\tsum(amount)\n1\tNULL\n"
        );
        assert_eq!(
            merge_grouped_integer_sums(
                &[
                    shard(10, "id\tsum(amount)\n1\t18446744073709551615\n"),
                    shard(10, "id\tsum(amount)\n1\t2\n")
                ],
                1,
                "UInt64",
                "id",
                "amount"
            )
            .unwrap(),
            b"id\tsum(amount)\n1\t18446744073709551617\n"
        );
    }

    #[test]
    fn rejects_malformed_capped_incompatible_or_overflowing_grouped_sums() {
        for bad in [
            "region\tsum(amount)\na\t1\t2\n",
            "region\tsum(amount)\na\t1.1\n",
            "region\tsum(amount)\na\t1",
            "region\tsum(amount)\na\t\n",
        ] {
            assert_eq!(
                merge_grouped_integer_sums(&[shard(10, bad)], 10, "Int64", "region", "amount"),
                Err(MergeError::InvalidGroupedSum)
            );
        }
        assert_eq!(
            merge_grouped_integer_sums(
                &[shard(1, "region\tsum(amount)\na\t1\n")],
                10,
                "Int64",
                "region",
                "amount"
            ),
            Err(MergeError::ShardRowLimit)
        );
        assert_eq!(
            merge_grouped_integer_sums(
                &[shard(10, "region\tsum(amount)\na\t1\nb\t2\n")],
                1,
                "Int64",
                "region",
                "amount"
            ),
            Err(MergeError::CoordinatorLimit)
        );
        assert_eq!(
            merge_grouped_integer_sums(
                &[shard(10, "other\tsum(amount)\na\t1\n")],
                10,
                "Int64",
                "region",
                "amount"
            ),
            Err(MergeError::SchemaMismatch)
        );
        assert_eq!(
            merge_grouped_integer_sums(
                &[
                    shard(
                        10,
                        "region\tsum(amount)\na\t170141183460469231731687303715884105727\n"
                    ),
                    shard(10, "region\tsum(amount)\na\t1\n")
                ],
                10,
                "Int128",
                "region",
                "amount"
            ),
            Err(MergeError::SumOverflow)
        );
    }

    #[test]
    fn only_accepts_decomposable_integer_sum_queries() {
        assert!(is_safe_sum(
            &parse_select("SELECT sum(amount) FROM events WHERE id >= 2").unwrap()
        ));
        for sql in [
            "SELECT sum(amount) FROM events ORDER BY amount",
            "SELECT sum(amount) FROM events LIMIT 1",
            "SELECT sum(amount) FROM numbers(10)",
            "SELECT sum(amount) FROM events GROUP BY amount",
            "SELECT avg(amount) FROM events",
        ] {
            assert!(
                parse_select(sql).map_or(true, |query| !is_safe_sum(&query)),
                "{sql}"
            );
        }
        assert!(integer_type("UInt128"));
        assert!(integer_type("Nullable(Int64)"));
        assert!(!integer_type("Float64"));
        assert!(!integer_type("Nullable(String)"));
    }

    #[test]
    fn merges_exact_unsigned_signed_and_empty_integer_sums() {
        assert_eq!(
            merge_integer_sums(
                &[
                    shard(10, "sum(amount)\n18446744073709551615\n"),
                    shard(10, "sum(amount)\n2\n"),
                    shard(10, "sum(amount)\nNULL\n"),
                ],
                "UInt64",
            )
            .unwrap(),
            b"sum(amount)\n18446744073709551617\n"
        );
        assert_eq!(
            merge_integer_sums(
                &[
                    shard(10, "sum(amount)\n-13\n"),
                    shard(10, "sum(amount)\n5\n"),
                    shard(10, "sum(amount)\nNULL\n"),
                ],
                "Nullable(Int64)",
            )
            .unwrap(),
            b"sum(amount)\n-8\n"
        );
        assert_eq!(
            merge_integer_sums(&[shard(10, "sum(amount)\nNULL\n")], "Int64").unwrap(),
            b"sum(amount)\nNULL\n"
        );
    }

    #[test]
    fn rejects_malformed_or_overflowing_distributed_sums() {
        for invalid in [
            "sum(amount)\n1.25\n",
            "sum(amount)\n1\n2\n",
            "sum(amount)\n",
            "sum(amount)\n1",
        ] {
            assert_eq!(
                merge_integer_sums(&[shard(10, invalid)], "Int64"),
                Err(MergeError::InvalidSum)
            );
        }
        assert_eq!(
            merge_integer_sums(
                &[shard(10, "sum(amount)\n1\n"), shard(10, "sum(total)\n2\n"),],
                "UInt64",
            ),
            Err(MergeError::SchemaMismatch)
        );
        assert_eq!(
            merge_integer_sums(
                &[
                    shard(10, "sum(amount)\n340282366920938463463374607431768211455\n"),
                    shard(10, "sum(amount)\n1\n"),
                ],
                "UInt128",
            ),
            Err(MergeError::SumOverflow)
        );
        assert_eq!(
            merge_integer_sums(
                &[
                    shard(10, "sum(amount)\n170141183460469231731687303715884105727\n"),
                    shard(10, "sum(amount)\n1\n"),
                ],
                "Int128",
            ),
            Err(MergeError::SumOverflow)
        );
    }

    #[test]
    fn requires_post_and_configured_peers() {
        let cluster = Cluster::default();
        assert_eq!(
            handle("GET", b"", &cluster, 10).status,
            "405 Method Not Allowed"
        );
        assert_eq!(
            handle("POST", b"SELECT id FROM events", &cluster, 10).status,
            "400 Bad Request"
        );
    }
}
