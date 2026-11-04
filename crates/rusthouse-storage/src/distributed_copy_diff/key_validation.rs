use super::*;

pub(super) fn windows_device_segment(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or(segment);
    ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|name| stem.eq_ignore_ascii_case(name))
        || (stem.len() == 4
            && (stem[..3].eq_ignore_ascii_case("COM") || stem[..3].eq_ignore_ascii_case("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

fn valid_key_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes.last() != Some(&b'.')
        && !bytes.windows(2).any(|pair| pair == b"..")
        && bytes[1..].iter().all(|&byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
        })
        && !windows_device_segment(segment)
}

pub(super) fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 512 && key.split('/').all(valid_key_segment)
}

// A case-sensitive source may carry two distinct keys that map to the same
// filename on a case-insensitive replica. Check aliases across both
// inventories, including keys that only differ between peers. Lowercase-only
// inventories retain the allocation-free fast path.
pub(super) fn reject_portable_key_aliases(
    source: &[ObjectMetadata],
    destination: &[ObjectMetadata],
) -> Result<(), &'static str> {
    // Enforce the inventory cap before hashing or allocating for untrusted
    // caller-provided inventories.
    if source.len() > 65_536 || destination.len() > 65_536 {
        return Err("replica inventory too large");
    }
    if !source
        .iter()
        .chain(destination)
        .any(|object| object.key.bytes().any(|byte| byte.is_ascii_uppercase()))
    {
        return Ok(());
    }
    let mut seen = HashMap::<String, &str>::with_capacity(source.len() + destination.len());
    for object in source.iter().chain(destination) {
        let folded = object.key.to_ascii_lowercase();
        if let Some(previous) = seen.insert(folded, object.key.as_str()) {
            if previous != object.key {
                return Err("case-insensitive replica object key collision");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod key_segment_tests {
    use super::*;

    #[test]
    fn validates_each_segment_without_changing_portable_rules() {
        for key in ["data/part-01.avro", "x1/y_2/z-3", "LPT0/log", "a/b.c"] {
            assert!(valid_key(key), "{key}");
        }
        for key in [
            "", "/a", "a/", "a//b", "a/./b", "a/..b", "a/b..c", "a/b.",
            "a/CON", "a/com1.txt", "a/LPT9", "a/é", "a/b c",
        ] {
            assert!(!valid_key(key), "{key}");
        }
    }
}
