//! Bounded grouped REGEXP_REPLACE and string metrics.
use super::*;
use regex::bytes::{Captures, RegexBuilder};
use rusthouse_storage::TypedQueryReader;
const MAX_GROUPS: usize = 200_000;
const MAX_GROUP_BYTES: usize = 64 * 1024 * 1024;
const MAX_REPLACED_BYTES: usize = 1024 * 1024;

fn validate_pattern(pattern: &str) -> Result<(), ExecutionError> {
    let bytes = pattern.as_bytes();
    let mut i = 0;
    let mut in_class = false;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            if matches!(
                bytes[i + 1],
                b'w' | b'W' | b'd' | b'D' | b's' | b'S' | b'b' | b'B' | b'u' | b'U' | b'p' | b'P'
            ) {
                return Err(ExecutionError::Invalid(
                    "REGEXP ASCII shorthand classes are unsupported".into(),
                ));
            }
            i += 2;
            continue;
        }
        if !in_class && bytes[i] == b'{' {
            return Err(ExecutionError::Invalid(
                "REGEXP counted repetition is unsupported".into(),
            ));
        }
        if bytes[i] == b'(' && bytes.get(i + 1) == Some(&b'?') && bytes.get(i + 2) != Some(&b':') {
            return Err(ExecutionError::Invalid(
                "REGEXP inline flags and assertions are unsupported".into(),
            ));
        }
        if in_class
            && matches!(
                (bytes[i], bytes.get(i + 1)),
                (b'&', Some(b'&')) | (b'-', Some(b'-')) | (b'~', Some(b'~')) | (b'[', _)
            )
        {
            return Err(ExecutionError::Invalid(
                "REGEXP character class operations are unsupported".into(),
            ));
        }
        if bytes[i] == b'[' {
            if bytes.get(i + 1) == Some(&b']')
                || (bytes.get(i + 1) == Some(&b'^') && bytes.get(i + 2) == Some(&b']'))
            {
                return Err(ExecutionError::Invalid(
                    "REGEXP leading class bracket is unsupported".into(),
                ));
            }
            in_class = true;
        } else if bytes[i] == b']' {
            in_class = false;
        }
        i += 1;
    }
    Ok(())
}
fn validate_replacement(replacement: &[u8], capture_count: usize) -> Result<(), ExecutionError> {
    let mut i = 0;
    while i < replacement.len() {
        if replacement[i] == b'\\' {
            let next = *replacement.get(i + 1).ok_or_else(|| {
                ExecutionError::Invalid("invalid REGEXP replacement escape".into())
            })?;
            if next.is_ascii_digit() {
                if usize::from(next - b'0') >= capture_count {
                    return Err(ExecutionError::Invalid(
                        "REGEXP replacement references a missing capture".into(),
                    ));
                }
            } else if next != b'\\' {
                return Err(ExecutionError::Invalid(
                    "invalid REGEXP replacement escape".into(),
                ));
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(())
}

struct Group {
    count: u64,
    length_count: u64,
    total_length: u128,
    minimum: Option<Vec<u8>>,
}
fn replacement(
    value: &[u8],
    captures: Option<&Captures<'_>>,
    replacement: &[u8],
) -> Result<Vec<u8>, ExecutionError> {
    let Some(captures) = captures else {
        if value.len() > MAX_REPLACED_BYTES {
            return Err(ExecutionError::Invalid(
                "REGEXP result exceeds 1 MiB".into(),
            ));
        }
        return Ok(value.to_vec());
    };
    let whole = captures.get(0).expect("capture zero exists");
    let mut result = Vec::new();
    let append = |result: &mut Vec<u8>, bytes: &[u8]| -> Result<(), ExecutionError> {
        if result.len().saturating_add(bytes.len()) > MAX_REPLACED_BYTES {
            return Err(ExecutionError::Invalid(
                "REGEXP result exceeds 1 MiB".into(),
            ));
        }
        result.extend_from_slice(bytes);
        Ok(())
    };
    append(&mut result, &value[..whole.start()])?;
    let mut i = 0;
    while i < replacement.len() {
        if replacement[i] == b'\\'
            && i + 1 < replacement.len()
            && replacement[i + 1].is_ascii_digit()
        {
            let index = usize::from(replacement[i + 1] - b'0');
            if let Some(capture) = captures.get(index) {
                append(&mut result, capture.as_bytes())?;
            }
            i += 2;
        } else if replacement[i] == b'\\' && replacement.get(i + 1) == Some(&b'\\') {
            append(&mut result, b"\\")?;
            i += 2;
        } else {
            append(&mut result, &replacement[i..i + 1])?;
            i += 1;
        }
    }
    append(&mut result, &value[whole.end()..])?;
    Ok(result)
}

pub(super) fn execute<W: Write>(
    query: &Select,
    path: Option<&Path>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let plan = query
        .grouped_regex
        .as_ref()
        .ok_or_else(|| ExecutionError::Invalid("REGEXP grouping plan is missing".into()))?;
    let path = path
        .ok_or_else(|| ExecutionError::Invalid("REGEXP grouping requires a stored table".into()))?;
    let metadata = inspect_query_source(path)?;
    if metadata.columns.iter().any(|column| {
        [
            plan.key_label.as_str(),
            plan.average_label.as_str(),
            plan.count_label.as_str(),
        ]
        .iter()
        .any(|alias| column.name.eq_ignore_ascii_case(alias))
    }) {
        return Err(ExecutionError::Invalid(
            "REGEXP alias conflicts with a source column".into(),
        ));
    }
    let column = metadata
        .columns
        .iter()
        .find(|column| column.name == plan.column)
        .ok_or_else(|| ExecutionError::Invalid("unknown REGEXP source column".into()))?;
    if typed::base_type(&column.data_type) != "String" {
        return Err(ExecutionError::Invalid(
            "REGEXP grouping requires String".into(),
        ));
    }
    validate_pattern(&plan.pattern)?;
    let regex = RegexBuilder::new(&plan.pattern)
        .size_limit(4 * 1024 * 1024)
        .build()
        .map_err(|error| ExecutionError::Invalid(format!("invalid REGEXP pattern: {error}")))?;
    validate_replacement(plan.replacement.as_bytes(), regex.captures_len())?;
    let mut names = vec![plan.column.clone()];
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        for name in std::iter::once(predicate.column()).chain(predicate.other_column()) {
            if !names.iter().any(|v| v == name) {
                names.push(name.to_owned());
            }
        }
    }
    let indices = names
        .iter()
        .enumerate()
        .map(|(i, name)| (name.clone(), i))
        .collect::<HashMap<_, _>>();
    let types = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect::<HashMap<_, _>>();
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    if query.limit == Some(0) {
        writeln!(
            output,
            "{}\t{}\t{}\tmin({})",
            plan.key_label, plan.average_label, plan.count_label, plan.column
        )?;
        return Ok(());
    }
    let mut reader = TypedQueryReader::open_with_metadata(path, &names, &metadata)?;
    let mut groups = HashMap::<Option<Vec<u8>>, Group>::new();
    let mut retained = 0usize;
    let mut scanned = 0u64;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            scanned = scanned
                .checked_add(1)
                .ok_or_else(|| ExecutionError::Invalid("REGEXP row count overflow".into()))?;
            if !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let cell = typed::cell(&batch[0], row);
            let value = match &cell {
                typed::Cell::Text(value) => Some(value.as_slice()),
                typed::Cell::Null => None,
                _ => {
                    return Err(ExecutionError::Invalid(
                        "REGEXP grouping requires String".into(),
                    ));
                }
            };
            if value.is_some_and(|value| std::str::from_utf8(value).is_err()) {
                return Err(ExecutionError::Invalid(
                    "REGEXP grouping requires UTF-8 input".into(),
                ));
            }
            let key = value
                .map(|value| {
                    replacement(
                        value,
                        regex.captures(value).as_ref(),
                        plan.replacement.as_bytes(),
                    )
                })
                .transpose()?;
            if !groups.contains_key(&key) {
                if groups.len() >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "REGEXP grouping exceeds 200000 groups".into(),
                    ));
                }
                let bytes = 192usize
                    .checked_add(key.as_ref().map_or(0, Vec::len))
                    .and_then(|n| n.checked_add(value.map_or(0, <[u8]>::len)))
                    .ok_or_else(|| {
                        ExecutionError::Invalid("REGEXP group memory overflow".into())
                    })?;
                retained = retained
                    .checked_add(bytes)
                    .filter(|&n| n <= MAX_GROUP_BYTES)
                    .ok_or_else(|| ExecutionError::Invalid("REGEXP groups exceed 64 MiB".into()))?;
                groups.insert(
                    key.clone(),
                    Group {
                        count: 0,
                        length_count: 0,
                        total_length: 0,
                        minimum: value.map(<[u8]>::to_vec),
                    },
                );
            }
            let group = groups.get_mut(&key).expect("group inserted");
            group.count = group
                .count
                .checked_add(1)
                .ok_or_else(|| ExecutionError::Invalid("REGEXP group count overflow".into()))?;
            if let Some(value) = value {
                group.length_count = group.length_count.checked_add(1).ok_or_else(|| {
                    ExecutionError::Invalid("REGEXP length count overflow".into())
                })?;
                group.total_length = group
                    .total_length
                    .checked_add(value.len() as u128)
                    .ok_or_else(|| ExecutionError::Invalid("REGEXP length sum overflow".into()))?;
                if group
                    .minimum
                    .as_deref()
                    .is_none_or(|current| value < current)
                {
                    let old = group.minimum.as_ref().map_or(0, Vec::len);
                    retained = retained
                        .checked_sub(old)
                        .and_then(|n| n.checked_add(value.len()))
                        .filter(|&n| n <= MAX_GROUP_BYTES)
                        .ok_or_else(|| {
                            ExecutionError::Invalid("REGEXP groups exceed 64 MiB".into())
                        })?;
                    group.minimum = Some(value.to_vec());
                }
            }
        }
    }
    if scanned != metadata.rows {
        return Err(ExecutionError::Invalid(
            "REGEXP input row count changed during scan".into(),
        ));
    }
    let mut rows = groups
        .into_iter()
        .filter(|(_, group)| group.count > plan.threshold)
        .collect::<Vec<_>>();
    let (descending, nulls_first) = query
        .order_by
        .as_ref()
        .map_or((false, None), |order| (order.descending, order.nulls_first));
    let average = |group: &Group| {
        if group.length_count == 0 {
            typed::Cell::Null
        } else {
            typed::Cell::Numeric(Number::Float(
                group.total_length as f64 / group.length_count as f64,
            ))
        }
    };
    rows.sort_by(|a, b| {
        typed::compare_order_cells(&average(&a.1), &average(&b.1), descending, nulls_first)
            .then_with(|| a.0.cmp(&b.0))
    });
    writeln!(
        output,
        "{}\t{}\t{}\tmin({})",
        plan.key_label, plan.average_label, plan.count_label, plan.column
    )?;
    for (key, group) in rows
        .into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        typed::write_value(output, &key.map_or(typed::Cell::Null, typed::Cell::Text))?;
        output.write_all(b"\t")?;
        typed::write_value(output, &average(&group))?;
        write!(output, "\t{}\t", group.count)?;
        typed::write_value(
            output,
            &group.minimum.map_or(typed::Cell::Null, typed::Cell::Text),
        )?;
        writeln!(output)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_respects_unicode_codepoints_and_captures() {
        let value = "éé".as_bytes();
        let dot = RegexBuilder::new(".").build().unwrap();
        assert_eq!(
            replacement(value, dot.captures(value).as_ref(), b"x").unwrap(),
            "xé".as_bytes()
        );
        let group = RegexBuilder::new("^(.)").build().unwrap();
        assert_eq!(
            replacement(value, group.captures(value).as_ref(), b"\\1!").unwrap(),
            "é!é".as_bytes()
        );
    }
    #[test]
    fn replacement_escapes_and_capture_indices_are_checked() {
        let regex = RegexBuilder::new("(.)").build().unwrap();
        assert!(validate_replacement(b"\\2", regex.captures_len()).is_err());
        assert!(validate_replacement(b"\\x", regex.captures_len()).is_err());
        assert!(validate_replacement(b"\\", regex.captures_len()).is_err());
        let replacement_text = b"\\\\1";
        assert!(validate_replacement(replacement_text, regex.captures_len()).is_ok());
        assert_eq!(
            replacement(b"abc", regex.captures(b"abc").as_ref(), replacement_text).unwrap(),
            b"\\1bc"
        );
        assert!(validate_pattern(r"\w").is_err());
        assert!(validate_pattern("[a&&b]").is_err());
        assert!(validate_pattern("[]a&&b]").is_err());
        assert!(validate_pattern("[^]a&&b]").is_err());
        assert!(validate_pattern("(?-u:.)").is_err());
        assert!(validate_pattern(r"\u00E9").is_err());
        assert!(validate_pattern(r"\p{ASCII}").is_err());
        assert!(validate_pattern("a{0,1001}").is_err());
        assert!(validate_pattern("(?:www\\.)?").is_ok());
        assert!(validate_pattern(r"\.").is_ok());
    }
    #[test]
    fn unmatched_large_values_respect_result_bound() {
        let value = vec![b'a'; MAX_REPLACED_BYTES + 1];
        assert!(replacement(&value, None, b"").is_err());
    }
}
