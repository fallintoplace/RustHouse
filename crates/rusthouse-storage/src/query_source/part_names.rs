use super::*;

// upstream names are partition_minBlock_maxBlock_level[_mutation].
// The partition id may contain underscores; split from the right instead.
pub(super) fn parse_part_span_suffix(name: &str, suffix: usize) -> Option<(&str, u64, u64)> {
    let mut tokens = name.rsplitn(suffix + 1, '_');
    if suffix == 4 {
        let _mutation = tokens.next()?.parse::<u64>().ok()?;
    }
    let _level = tokens.next()?.parse::<u64>().ok()?;
    let max = tokens.next()?.parse::<u64>().ok()?;
    let min = tokens.next()?.parse::<u64>().ok()?;
    let partition = tokens.next()?;
    (min <= max && !partition.is_empty()).then_some((partition, min, max))
}

pub(super) fn part_span(name: &str) -> Option<(String, u64, u64)> {
    // Preserve the numeric-suffix ambiguity check without allocating two
    // candidate partition strings for each part in a large snapshot.
    match (
        parse_part_span_suffix(name, 3),
        parse_part_span_suffix(name, 4),
    ) {
        (Some((partition, min, max)), None) | (None, Some((partition, min, max))) => {
            Some((partition.to_owned(), min, max))
        }
        _ => None,
    }
}

pub(super) fn sort_and_validate_part_ranges(
    parts: &mut [(String, u64, u64, PathBuf)],
) -> Result<(), PartError> {
    // Each partition's block ranges become adjacent after sorting. Validate
    // overlaps in the same pass without copying partition names into a map
    // and allocating a separate sortable range list per partition.
    parts.sort_unstable_by(|left, right| {
        (&left.0, left.1, left.2, &left.3).cmp(&(&right.0, right.1, right.2, &right.3))
    });
    for adjacent in parts.windows(2) {
        if adjacent[0].0 == adjacent[1].0 && adjacent[0].2 >= adjacent[1].1 {
            return Err(PartError::InvalidScan(
                "overlapping part block ranges: provide a frozen active-part snapshot".into(),
            ));
        }
    }
    Ok(())
}
