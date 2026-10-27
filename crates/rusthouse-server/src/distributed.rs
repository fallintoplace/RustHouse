use crate::cluster::{Cluster, Peer};
use crate::remote;
use crate::{MAX_QUERY_BYTES, MAX_RESPONSE_BYTES, Response};
use rusthouse_query::{AggregateFunction, Projection, Select, Source, parse_select};
use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq)]
enum MergeError {
    InvalidResponse,
    SchemaMismatch,
    ShardRowLimit,
    CoordinatorLimit,
    InvalidCount,
    InvalidGroupedCount,
    InvalidSum,
    CountOverflow,
    SumOverflow,
}

fn is_safe_scan(select: &Select) -> bool {
    matches!(select.source, Some(Source::Table(_)))
        && matches!(select.projection, Projection::All | Projection::Columns(_))
        && !select.distinct
        && select.rename.is_none()
        && select.order_by.is_none()
        && select.order_by_extra.is_empty()
        && select.having.is_none()
        && select.limit.is_none()
        && select.offset.is_none()
}

fn is_safe_count(select: &Select) -> bool {
    matches!(
        select.projection,
        Projection::Count | Projection::CountColumn { .. }
    ) && matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.rename.is_none()
        && select.order_by.is_none()
        && select.order_by_extra.is_empty()
        && select.having.is_none()
        && select.limit.is_none()
        && select.offset.is_none()
}

fn is_safe_sum(select: &Select) -> bool {
    matches!(
        select.projection,
        Projection::Aggregate {
            function: AggregateFunction::Sum,
            ..
        }
    ) && matches!(select.source, Some(Source::Table(_)))
        && !select.distinct
        && select.rename.is_none()
        && select.order_by.is_none()
        && select.order_by_extra.is_empty()
        && select.having.is_none()
        && select.limit.is_none()
        && select.offset.is_none()
}

fn integer_type(kind: &str) -> bool {
    let base = kind
        .strip_prefix("Nullable(")
        .and_then(|inner| inner.strip_suffix(')'))
        .unwrap_or(kind);
    matches!(
        base,
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
    )
}

fn is_safe_grouped_count(select: &Select) -> bool {
    matches!(select.source, Some(Source::Table(_)))
        && matches!(select.projection, Projection::GroupedCount { .. })
        && !select.distinct
        && select.rename.is_none()
        && select.order_by.is_none()
        && select.order_by_extra.is_empty()
        && select.having.is_none()
        && select.limit.is_none()
        && select.offset.is_none()
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
    if cap == 0 || cap > 100_000 {
        return Err("unsupported peer row limit".into());
    }
    Ok(cap)
}

fn shard_integer_kind(peer: Peer, table: &str, column: &str) -> Result<String, String> {
    let mut bytes = column.bytes();
    if !bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("distributed SUM requires a simple column identifier".into());
    }
    let response = remote::schema(peer, table)?;
    if response.status != 200 || !response.content_type.starts_with("application/json") {
        return Err("peer schema lookup failed".into());
    }
    let contents =
        std::str::from_utf8(&response.body).map_err(|_| "peer schema response is not UTF-8")?;
    let needle = format!("\"name\":\"{column}\",\"type\":\"");
    let (_, tail) = contents
        .split_once(needle.as_str())
        .ok_or("SUM column missing from peer schema")?;
    let (kind, trailing) = tail
        .split_once('"')
        .ok_or("invalid peer schema column type")?;
    if !trailing.starts_with('}') || !integer_type(kind) {
        return Err("distributed SUM only supports exact integer columns".into());
    }
    Ok(kind.to_owned())
}

fn read_shard(
    peer: Peer,
    sql: &str,
    sum_schema: Option<(&str, &str)>,
) -> Result<(usize, Vec<u8>, Option<String>), String> {
    let cap = shard_row_cap(peer)?;
    let kind = sum_schema
        .map(|(table, column)| shard_integer_kind(peer, table, column))
        .transpose()?;
    let result = remote::query(peer, sql)?;
    if result.status != 200 {
        return Err(format!("peer returned HTTP {}", result.status));
    }
    if result.content_type != "text/tab-separated-values; charset=utf-8" {
        return Err("peer did not return TSV".into());
    }
    Ok((cap, result.body, kind))
}

fn merge_tsv(shards: &[(usize, Vec<u8>)], max_rows: usize) -> Result<Vec<u8>, MergeError> {
    let mut chunks: Vec<&[u8]> = Vec::with_capacity(shards.len());
    let mut header: Option<&[u8]> = None;
    let mut total_rows = 0usize;
    let mut total_bytes = 0usize;
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

fn merge_grouped_counts(
    shards: &[(usize, Vec<u8>)],
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    let mut expected_header: Option<&[u8]> = None;
    let mut positions = HashMap::<Vec<u8>, usize>::new();
    let mut groups = Vec::<(Vec<u8>, u64)>::new();
    for (cap, body) in shards {
        if !body.ends_with(b"\n") || *cap == 0 {
            return Err(MergeError::InvalidGroupedCount);
        }
        let header_end = body
            .iter()
            .position(|&byte| byte == b'\n')
            .ok_or(MergeError::InvalidGroupedCount)?;
        let header = &body[..=header_end];
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
                if groups.len() >= max_rows {
                    return Err(MergeError::CoordinatorLimit);
                }
                positions.insert(key.to_vec(), groups.len());
                groups.push((key.to_vec(), count));
            }
        }
    }
    let header = expected_header.ok_or(MergeError::InvalidGroupedCount)?;
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
        output.extend_from_slice(&key);
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
                || is_safe_grouped_count(&select)
                || is_safe_sum(&select) =>
        {
            select
        }
        _ => {
            return Response::text(
                "400 Bad Request",
                "distributed queries require unordered SELECT, COUNT, integer SUM, or grouped COUNT without global clauses\n",
            );
        }
    };
    let count_query = is_safe_count(&select);
    let grouped_count_query = is_safe_grouped_count(&select);
    let sum_query = is_safe_sum(&select);
    let sum_schema = match (&select.source, &select.projection) {
        (
            Some(Source::Table(table)),
            Projection::Aggregate {
                function: AggregateFunction::Sum,
                column,
            },
        ) => Some((table.as_str(), column.as_str())),
        _ => None,
    };
    if !matches!(select.source, Some(Source::Table(_))) || cluster.peers().is_empty() {
        return Response::text(
            "400 Bad Request",
            "no distributed database peers configured\n",
        );
    }
    let results = std::thread::scope(|scope| {
        let handles = cluster
            .peers()
            .iter()
            .map(|&peer| scope.spawn(move || read_shard(peer, sql, sum_schema)))
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
    for result in results {
        match result {
            Ok((cap, body, kind)) => {
                if let Some(kind) = kind {
                    if sum_type
                        .as_deref()
                        .is_some_and(|existing| existing != kind.as_str())
                    {
                        return Response::text(
                            "409 Conflict",
                            "distributed SUM shard column types disagree\n",
                        );
                    }
                    sum_type = Some(kind);
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
    } else if sum_query {
        sum_type
            .as_deref()
            .map_or(Err(MergeError::InvalidSum), |kind| {
                merge_integer_sums(&shards, kind)
            })
    } else if grouped_count_query {
        merge_grouped_counts(&shards, max_rows.min(100_000))
    } else {
        merge_tsv(&shards, max_rows.min(100_000))
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
        Err(MergeError::InvalidGroupedCount) => {
            Response::text("502 Bad Gateway", "malformed shard GROUP BY response\n")
        }
        Err(MergeError::InvalidSum) => {
            Response::text("502 Bad Gateway", "malformed shard SUM response\n")
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

    fn shard(cap: usize, data: &str) -> (usize, Vec<u8>) {
        (cap, data.as_bytes().to_vec())
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
            "SELECT id FROM events LIMIT 10",
            "SELECT DISTINCT id FROM events",
            "SELECT number FROM numbers(10)",
            "SELECT id FROM events OFFSET 1",
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
