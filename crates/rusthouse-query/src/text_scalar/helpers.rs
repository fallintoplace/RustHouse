use super::*;

// Decode URL escapes with strict UTF-8 and bounded linear work.
pub(super) fn decode_percent_encoded(input: &str, form: bool) -> Result<String, ParseError> {
    if !input.contains('%') && (!form || !input.contains('+')) {
        return Ok(input.to_owned());
    }
    let source = input.as_bytes();
    let mut decoded = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            let high = source
                .get(index + 1)
                .and_then(|byte| char::from(*byte).to_digit(16));
            let low = source
                .get(index + 2)
                .and_then(|byte| char::from(*byte).to_digit(16));
            let (Some(high), Some(low)) = (high, low) else {
                return Err(ParseError("invalid URL percent escape".into()));
            };
            decoded.push((high * 16 + low) as u8);
            index += 3;
        } else {
            decoded.push(if form && source[index] == b'+' {
                b' '
            } else {
                source[index]
            });
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| ParseError("URL decode result is not UTF-8".into()))
}

// Format a bounded subset of ClickHouse's Python-style constant placeholders.
pub(super) fn format_constant_template(pattern: &str, args: &[String]) -> Result<String, ParseError> {
    let bytes = pattern.as_bytes();
    let mut result = String::new();
    let mut index = 0;
    let mut next_implicit = 0usize;
    let mut explicit_mode: Option<bool> = None;
    while index < bytes.len() {
        let start = index;
        while index < bytes.len() && !matches!(bytes[index], b'{' | b'}') {
            index += 1;
        }
        if start < index {
            result.push_str(&pattern[start..index]);
        }
        if index == bytes.len() {
            break;
        }
        let brace = bytes[index];
        index += 1;
        if bytes.get(index) == Some(&brace) {
            result.push(char::from(brace));
            index += 1;
        } else if brace == b'{' {
            let start = index;
            while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
            if bytes.get(index) != Some(&b'}') {
                return Err(ParseError("FORMAT requires numeric or empty placeholders".into()));
            }
            let explicit = index != start;
            if explicit_mode.is_some_and(|previous| previous != explicit) {
                return Err(ParseError("FORMAT cannot mix numbered and automatic fields".into()));
            }
            explicit_mode = Some(explicit);
            let argument = if explicit {
                pattern[start..index]
                    .parse::<usize>()
                    .map_err(|_| ParseError("FORMAT placeholder index is too large".into()))?
            } else {
                let current = next_implicit;
                next_implicit += 1;
                current
            };
            result.push_str(args.get(argument).ok_or_else(|| {
                ParseError("FORMAT placeholder exceeds argument count".into())
            })?);
            index += 1;
        } else {
            return Err(ParseError("FORMAT has an unmatched closing brace".into()));
        }
        if result.len() > MAX_TEXT_BYTES {
            return Err(ParseError("FORMAT result is too large".into()));
        }
    }
    if result.len() > MAX_TEXT_BYTES {
        return Err(ParseError("FORMAT result is too large".into()));
    }
    Ok(result)
}

// ClickHouse's built-in significant-domain heuristic for common multi-label
// hostnames. It is not a public-suffix-list lookup.
pub(super) fn significant_hostname_parts(url: &str) -> Option<(&str, &str, &str)> {
    let authority = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()?;
    let host_port = authority.rsplit('@').next()?;
    // Bracketed IPv6 hosts and numeric IPv4 addresses have no registrable label.
    if host_port.starts_with('[') {
        return None;
    }
    let host = host_port.split(':').next()?;
    if host.parse::<std::net::Ipv4Addr>().is_ok() || !host.is_ascii() || host.len() > 253 {
        return None;
    }
    if host.split('.').any(|label| {
        label.is_empty()
            || label.len() > 63
            || !label.as_bytes()[0].is_ascii_alphanumeric()
            || !label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    }) {
        return None;
    }
    let last_dot = host.rfind('.')?;
    let before_tld = &host[..last_dot];
    let second_dot = before_tld.rfind('.');
    let second = &before_tld[second_dot.map_or(0, |index| index + 1)..];
    // ASCII hostname validation above makes case-insensitive byte
    // comparison sufficient; no lowercase allocation is needed.
    let common_second_level = ["com", "net", "org", "co"]
        .iter()
        .any(|suffix| second.eq_ignore_ascii_case(suffix));
    let significant_start = if common_second_level {
        let before_second = &before_tld[..second_dot?];
        before_second.rfind('.').map_or(0, |index| index + 1)
    } else {
        second_dot.map_or(0, |index| index + 1)
    };
    let significant_end = if common_second_level {
        second_dot?
    } else {
        last_dot
    };
    Some((
        host,
        &host[significant_start..significant_end],
        &host[significant_start..],
    ))
}

// Compile the reflected IEEE polynomial table once instead of executing
// eight bit-level recurrence steps for each input byte.
const fn crc32_ieee_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0usize;
    while index < table.len() {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (0u32.wrapping_sub(crc & 1)));
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
}

const CRC32_IEEE_TABLE: [u32; 256] = crc32_ieee_table();

pub(super) fn crc32_ieee_zero_seed(input: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &byte in input {
        crc = (crc >> 8) ^ CRC32_IEEE_TABLE[usize::from((crc as u8) ^ byte)];
    }
    crc
}

pub(super) fn crc32_ieee(input: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in input {
        crc = (crc >> 8) ^ CRC32_IEEE_TABLE[usize::from((crc as u8) ^ byte)];
    }
    !crc
}

// Convert a bounded, unambiguous JSONPath subset to the existing dotted
// object/array lookup used by the strict JSON parser.
pub(super) fn simple_json_path(path: &str) -> Result<Vec<crate::json::JsonPathSegment<'_>>, ParseError> {
    if path.len() > 256 || !path.starts_with('$') {
        return Err(ParseError("JSON_EXISTS requires a bounded root JSONPath".into()));
    }
    let bytes = path.as_bytes();
    let mut cursor = 1;
    let mut components = Vec::new();
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'.' => {
                cursor += 1;
                let start = cursor;
                while cursor < bytes.len()
                    && (bytes[cursor].is_ascii_alphanumeric()
                        || matches!(bytes[cursor], b'_' | b'-'))
                {
                    cursor += 1;
                }
                if start == cursor {
                    return Err(ParseError("unsupported JSON_EXISTS path".into()));
                }
                components.push(crate::json::JsonPathSegment::Key(&path[start..cursor]));
            }
            b'[' => {
                cursor += 1;
                if bytes.get(cursor) == Some(&b'\'') {
                    cursor += 1;
                    let start = cursor;
                    while cursor < bytes.len() && bytes[cursor] != b'\'' {
                        // Keep path components borrowed. Unsupported escapes
                        // must fail, never silently select a different key.
                        if bytes[cursor] == b'\\' || bytes[cursor] < 0x20 {
                            return Err(ParseError("unsupported JSON_EXISTS quoted key".into()));
                        }
                        cursor += 1;
                    }
                    if bytes.get(cursor) != Some(&b'\'')
                        || bytes.get(cursor + 1) != Some(&b']')
                    {
                        return Err(ParseError("unterminated JSON_EXISTS quoted key".into()));
                    }
                    components.push(crate::json::JsonPathSegment::Key(&path[start..cursor]));
                    cursor += 2;
                    continue;
                }
                let start = cursor;
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    cursor += 1;
                }
                if cursor == start || bytes.get(cursor) != Some(&b']') {
                    return Err(ParseError("unsupported JSON_EXISTS array index".into()));
                }
                let index = path[start..cursor]
                    .parse::<usize>()
                    .map_err(|_| ParseError("JSON_EXISTS array index exceeds usize".into()))?;
                components.push(crate::json::JsonPathSegment::Index(index));
                cursor += 1;
            }
            _ => return Err(ParseError("unsupported JSON_EXISTS path".into())),
        }
    }
    Ok(components)
}

pub(super) fn binary_string(bytes: &[u8]) -> Result<String, ParseError> {
    let len = bytes
        .len()
        .checked_mul(8)
        .ok_or_else(|| ParseError("bin result is too large".into()))?;
    if len > MAX_TEXT_BYTES {
        return Err(ParseError("bin result is too large".into()));
    }
    let mut output = String::with_capacity(len);
    for &byte in bytes {
        for shift in (0..8).rev() {
            output.push(if byte >> shift & 1 == 0 { '0' } else { '1' });
        }
    }
    Ok(output)
}

pub(super) fn parse_mac_address(input: &str) -> Option<u64> {
    let mut value = 0_u64;
    let mut count = 0;
    for group in input.split(':') {
        if group.len() != 2 || !group.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let byte = u8::from_str_radix(group, 16).ok()?;
        value = (value << 8) | u64::from(byte);
        count += 1;
    }
    (count == 6).then_some(value)
}

pub(super) fn parse_readable_size_bytes(input: &str) -> Option<i64> {
    let (number, unit) = input.split_once(' ')?;
    let (whole, fractional) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fractional.len() > 18
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let (base, exponent) = match unit.to_ascii_lowercase().as_str() {
        "b" => (1_u128, 0),
        "kb" => (1_000, 1),
        "mb" => (1_000, 2),
        "gb" => (1_000, 3),
        "tb" => (1_000, 4),
        "pb" => (1_000, 5),
        "eb" => (1_000, 6),
        "kib" => (1_024, 1),
        "mib" => (1_024, 2),
        "gib" => (1_024, 3),
        "tib" => (1_024, 4),
        "pib" => (1_024, 5),
        "eib" => (1_024, 6),
        _ => return None,
    };
    let denominator = 10_u128.checked_pow(fractional.len() as u32)?;
    let whole = whole.parse::<u128>().ok()?;
    let fractional = if fractional.is_empty() {
        0
    } else {
        fractional.parse::<u128>().ok()?
    };
    let numerator = whole.checked_mul(denominator)?.checked_add(fractional)?;
    let factor = base.checked_pow(exponent)?;
    let bytes = numerator.checked_mul(factor)?;
    let rounded = bytes
        .checked_add(denominator - 1)?
        .checked_div(denominator)?;
    i64::try_from(rounded).ok()
}

// Equal prefixes and suffixes cannot contribute to Levenshtein distance.
// Remove them before enforcing the quadratic work budget. Long values with a
// small changed region remain cheap without weakening the worst-case bound.
pub(super) fn trim_common_edges<'a, T: Eq>(left: &'a [T], right: &'a [T]) -> (&'a [T], &'a [T]) {
    let prefix = left
        .iter()
        .zip(right.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let left = &left[prefix..];
    let right = &right[prefix..];
    let suffix = left
        .iter()
        .rev()
        .zip(right.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (&left[..left.len() - suffix], &right[..right.len() - suffix])
}

pub(super) fn bounded_edit_distance<T: Eq>(left: &[T], right: &[T]) -> Result<i64, ParseError> {
    if left == right {
        return Ok(0);
    }
    let (left, right) = trim_common_edges(left, right);
    if left.is_empty() {
        return i64::try_from(right.len())
            .map_err(|_| ParseError("editDistance exceeds Int64".into()));
    }
    if right.is_empty() {
        return i64::try_from(left.len())
            .map_err(|_| ParseError("editDistance exceeds Int64".into()));
    }
    if left.len().saturating_mul(right.len()) > 2_000_000 {
        return Err(ParseError("editDistance exceeds computation budget".into()));
    }
    let (short, long) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let mut previous: Vec<usize> = (0..=short.len()).collect();
    let mut current = vec![0; short.len() + 1];
    for (row, item) in long.iter().enumerate() {
        current[0] = row + 1;
        for (column, other) in short.iter().enumerate() {
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(previous[column] + usize::from(item != other));
        }
        std::mem::swap(&mut previous, &mut current);
    }
    i64::try_from(previous[short.len()])
        .map_err(|_| ParseError("editDistance exceeds Int64".into()))
}

pub(super) fn bounded_utf8_edit_distance(left: &str, right: &str) -> Result<i64, ParseError> {
    if left == right {
        return Ok(0);
    }
    // Trim UTF-8 character boundaries before allocating char vectors.
    // A tiny edit in long text should only materialize its changed region.
    let mut left_start = 0;
    let mut right_start = 0;
    for ((left_index, left_char), (right_index, right_char)) in
        left.char_indices().zip(right.char_indices())
    {
        if left_char != right_char {
            break;
        }
        left_start = left_index + left_char.len_utf8();
        right_start = right_index + right_char.len_utf8();
    }
    let left = &left[left_start..];
    let right = &right[right_start..];
    let mut left_end = left.len();
    let mut right_end = right.len();
    for ((left_index, left_char), (right_index, right_char)) in
        left.char_indices().rev().zip(right.char_indices().rev())
    {
        if left_char != right_char {
            break;
        }
        left_end = left_index;
        right_end = right_index;
    }
    let left: Vec<char> = left[..left_end].chars().collect();
    let right: Vec<char> = right[..right_end].chars().collect();
    if left.is_empty() {
        return Ok(right.len() as i64);
    }
    if right.is_empty() {
        return Ok(left.len() as i64);
    }
    if left.len().saturating_mul(right.len()) > 2_000_000 {
        return Err(ParseError(
            "editDistanceUTF8 exceeds computation budget".into(),
        ));
    }
    let (short, long) = if left.len() <= right.len() {
        (&left, &right)
    } else {
        (&right, &left)
    };
    let mut previous: Vec<usize> = (0..=short.len()).collect();
    let mut current = vec![0; short.len() + 1];
    for (row, item) in long.iter().enumerate() {
        current[0] = row + 1;
        for (column, other) in short.iter().enumerate() {
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(previous[column] + usize::from(item != other));
        }
        std::mem::swap(&mut previous, &mut current);
    }
    Ok(previous[short.len()] as i64)
}

pub(super) fn soundex_ascii(input: &str) -> String {
    fn digit(byte: u8) -> u8 {
        match byte.to_ascii_uppercase() {
            b'B' | b'F' | b'P' | b'V' => b'1',
            b'C' | b'G' | b'J' | b'K' | b'Q' | b'S' | b'X' | b'Z' => b'2',
            b'D' | b'T' => b'3',
            b'L' => b'4',
            b'M' | b'N' => b'5',
            b'R' => b'6',
            _ => b'0',
        }
    }

    let mut letters = input.bytes().filter(u8::is_ascii_alphabetic);
    let mut code = String::with_capacity(4);
    let mut previous = b'0';
    if let Some(first) = letters.next() {
        code.push(char::from(first.to_ascii_uppercase()));
        previous = digit(first);
    }
    for letter in letters {
        let next = digit(letter);
        if next != b'0' && next != previous {
            code.push(char::from(next));
            if code.len() == 4 {
                break;
            }
        }
        // ClickHouse treats H/W as zero, breaking runs of the same digit.
        previous = next;
    }
    while code.len() < 4 {
        code.push('0');
    }
    code
}


pub(super) fn regexp_quote_meta(input: &str) -> Result<String, ParseError> {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        if character == '\0' {
            output.push_str("\\0");
        } else {
            if matches!(
                character,
                '\\' | '|' | '(' | ')' | '^' | '$' | '.' | '[' | ']' | '?' | '*' | '+'
                    | '{' | ':' | '-'
            ) {
                output.push('\\');
            }
            output.push(character);
        }
        if output.len() > MAX_TEXT_BYTES {
            return Err(ParseError("regexpQuoteMeta result is too large".into()));
        }
    }
    Ok(output)
}


pub(super) fn base58_encode(input: &str) -> Result<String, ParseError> {
    const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    // Base conversion is quadratic. Keep constant evaluation CPU-bounded.
    if input.len() > 4096 {
        return Err(ParseError("base58Encode constant exceeds 4096 bytes".into()));
    }
    let mut digits = Vec::<u8>::new();
    for &byte in input.as_bytes() {
        let mut carry = usize::from(byte);
        for digit in &mut digits {
            carry += usize::from(*digit) * 256;
            *digit = (carry % 58) as u8;
            carry /= 58;
        }
        while carry != 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let zeros = input.bytes().take_while(|&byte| byte == 0).count();
    let length = zeros.checked_add(digits.len())
        .ok_or_else(|| ParseError("base58Encode output is too large".into()))?;
    if length > MAX_TEXT_BYTES {
        return Err(ParseError("base58Encode output is too large".into()));
    }
    let mut encoded = String::with_capacity(length);
    encoded.extend(std::iter::repeat_n('1', zeros));
    for &digit in digits.iter().rev() {
        encoded.push(char::from(ALPHABET[usize::from(digit)]));
    }
    Ok(encoded)
}


pub(super) fn base58_decode(input: &str) -> Result<String, ParseError> {
    const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    if input.len() > 4096 {
        return Err(ParseError("base58Decode constant exceeds 4096 bytes".into()));
    }
    let mut decoded = Vec::<u8>::new();
    for symbol in input.bytes() {
        let mut carry = ALPHABET.iter().position(|&value| value == symbol)
            .ok_or_else(|| ParseError("base58Decode contains an invalid digit".into()))?;
        for byte in &mut decoded {
            carry += usize::from(*byte) * 58;
            *byte = (carry % 256) as u8;
            carry /= 256;
        }
        while carry != 0 {
            decoded.push((carry % 256) as u8);
            carry /= 256;
        }
    }
    let zeros = input.bytes().take_while(|&byte| byte == b'1').count();
    let length = zeros.checked_add(decoded.len())
        .ok_or_else(|| ParseError("base58Decode output is too large".into()))?;
    if length > MAX_TEXT_BYTES {
        return Err(ParseError("base58Decode output is too large".into()));
    }
    let mut bytes = Vec::with_capacity(length);
    bytes.extend(std::iter::repeat_n(0, zeros));
    bytes.extend(decoded.iter().rev());
    String::from_utf8(bytes)
        .map_err(|_| ParseError("base58Decode constant is not valid UTF-8".into()))
}

pub(super) fn bounded_damerau_levenshtein(left: &[u8], right: &[u8]) -> Result<i64, ParseError> {
    if left == right {
        return Ok(0);
    }
    if left.is_empty() || right.is_empty() {
        return Ok(left.len().max(right.len()) as i64);
    }
    let rows = left.len() + 2;
    let width = right.len() + 2;
    let cells = rows
        .checked_mul(width)
        .filter(|&count| count <= 1_000_000)
        .ok_or_else(|| ParseError("Damerau-Levenshtein exceeds computation budget".into()))?;
    let mut matrix = vec![0usize; cells];
    let maximum = left.len() + right.len();
    matrix[0] = maximum;
    for i in 0..=left.len() {
        matrix[(i + 1) * width] = maximum;
        matrix[(i + 1) * width + 1] = i;
    }
    for j in 0..=right.len() {
        matrix[j + 1] = maximum;
        matrix[width + j + 1] = j;
    }
    let mut last_position = [0usize; 256];
    for i in 1..=left.len() {
        let mut last_match = 0usize;
        for j in 1..=right.len() {
            let previous_i = last_position[right[j - 1] as usize];
            let previous_j = last_match;
            let cost = usize::from(left[i - 1] != right[j - 1]);
            if cost == 0 {
                last_match = j;
            }
            let substitution = matrix[i * width + j] + cost;
            let insertion = matrix[(i + 1) * width + j] + 1;
            let deletion = matrix[i * width + j + 1] + 1;
            let transposition = matrix[previous_i * width + previous_j]
                + (i - previous_i - 1)
                + 1
                + (j - previous_j - 1);
            matrix[(i + 1) * width + j + 1] =
                substitution.min(insertion).min(deletion).min(transposition);
        }
        last_position[left[i - 1] as usize] = i;
    }
    Ok(matrix[(left.len() + 1) * width + right.len() + 1] as i64)
}

pub(super) fn unhex_text(input: &str) -> Result<String, ParseError> {
    if !input.is_ascii() {
        return Err(ParseError("unhex requires ASCII digits".into()));
    }
    fn digit(value: u8) -> Result<u8, ParseError> {
        char::from(value).to_digit(16)
            .map(|value| value as u8)
            .ok_or_else(|| ParseError("unhex contains a non-hexadecimal digit".into()))
    }
    let mut pairs = input.as_bytes().chunks_exact(2);
    let mut bytes = Vec::with_capacity((input.len() + 1) / 2);
    for pair in pairs.by_ref() {
        bytes.push((digit(pair[0])? << 4) | digit(pair[1])?);
    }
    if let Some(&last) = pairs.remainder().first() {
        // ClickHouse takes a lone trailing digit as the low nibble of a byte.
        bytes.push(digit(last)?);
    }
    String::from_utf8(bytes)
        .map_err(|_| ParseError("unhex constant is not valid UTF-8".into()))
}

#[cfg(test)]
mod significant_host_case_fast_path_tests {
    use super::*;

    #[test]
    fn uppercase_common_suffixes_use_the_same_domain_labels() {
        for host in [
            "a.Example.co.uk", "a.Example.CO.uk",
            "a.Example.com.au", "a.Example.COM.au",
            "a.Example.NET.au", "a.Example.ORG.au"
        ] {
            let (_, label, suffix) = significant_hostname_parts(host).unwrap();
            assert_eq!(label, "Example");
            assert!(suffix.starts_with("Example."));
        }
        assert_eq!(
            significant_hostname_parts("a.Example.de").map(|(_, label, suffix)| (label, suffix)),
            Some(("Example", "Example.de"))
        );
    }
}
