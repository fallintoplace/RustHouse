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

pub(super) fn valid_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    if bytes.is_empty() || bytes.len() > 512 {
        return false;
    }
    let mut segment_start = true;
    let mut segment_offset = 0usize;
    let mut segment_length = 0usize;
    let mut previous_dot = false;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'/' {
            if segment_start || previous_dot || windows_device_segment(&key[segment_offset..index])
            {
                return false;
            }
            segment_start = true;
            segment_offset = index + 1;
            segment_length = 0;
            previous_dot = false;
            continue;
        }
        segment_length += 1;
        if segment_length > 128 {
            return false;
        }
        if (segment_start && !byte.is_ascii_alphanumeric())
            || (!byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-' | b'.'))
            || (byte == b'.' && previous_dot)
        {
            return false;
        }
        segment_start = false;
        previous_dot = byte == b'.';
    }
    !segment_start && !previous_dot && !windows_device_segment(&key[segment_offset..])
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
