use super::{ConstantValue, ParseError, Parser, Token};

const MAX_DEPTH: usize = 16;
const MAX_TEXT_BYTES: usize = 1_048_576;

pub(super) fn is_function(name: &str) -> bool {
    super::date_scalar::is_function(name) || matches!(
        name.to_ascii_uppercase().as_str(),
        "ISIPADDRESSINRANGE"
            | "MACSTRINGTOOUI"
            | "MACNUMTOSTRING"
            | "MACSTRINGTONUM"
            | "IPV4NUMTOSTRINGCLASSC"
            | "IPV4STRINGTONUMORDEFAULT"
            | "CUTWWW"
            | "DOMAINWITHOUTWWW"
            | "CUTURLPARAMETER"
            | "JSONEXTRACTBOOL"
            | "IPV4NUMTOSTRING"
            | "IPV4STRINGTONUM"
            | "IPV4STRINGTONUMORNULL"
            | "DECODEURLFORMCOMPONENT"
            | "LENGTH"
            | "TOYEAR"
            | "LENGTHUTF8"
            | "TOSTARTOFFIVEMINUTES"
            | "CHAR_LENGTH"
            | "OCTET_LENGTH"
            | "TOSTARTOFFIFTEENMINUTES"
            | "ASCII"
            | "TOSTARTOFMINUTE"
            | "UTF16_LENGTH"
            | "ISIPV6STRING"
            | "HASSUBSEQUENCE"
            | "BYTEHAMMINGDISTANCE"
            | "CHR"
            | "TOUNIXTIMESTAMP"
            | "STRINGBYTESUNIQ"
            | "BIT_LENGTH"
            | "BITMASKTOLIST"
            | "IS_INT64"
            | "TOMODIFIEDJULIANDAY"
            | "BTRIM"
            | "TOLASTDAYOFWEEK"
            | "TRIMBOTH"
            | "RTRIM"
            | "CUTQUERYSTRING"
            | "COLLAPSE_WHITESPACE"
            | "LOWER"
            | "LOWERUTF8"
            | "HEX_ENCODE"
            | "DECODEXMLCOMPONENT"
            | "ENCODEXMLCOMPONENT"
            | "HEX"
            | "IS_ASCII"
            | "ISIPV4STRING"
            | "UPPER"
            | "UPPERUTF8"
            | "URL_DECODE"
            | "DECODEURLCOMPONENT"
            | "IS_EMPTY"
            | "EMPTY"
            | "REVERSE"
            | "REVERSEUTF8"
            | "INITCAP"
            | "TOSTARTOFTENMINUTES"
            | "CONCAT"
            | "CONCATASSUMEINJECTIVE"
            | "QUOTE_LITERAL"
            | "FROMUNIXTIMESTAMP64MILLI"
            | "CUTQUERYSTRINGANDFRAGMENT"
            | "COALESCE"
            | "SUBTRACTHOURS"
            | "CONCAT_WS"
            | "CONCATWITHSEPARATOR"
            | "CUTFRAGMENT"
            | "URL_ENCODE"
            | "FRAGMENT"
            | "DATETRUNC"
            | "JSON_QUOTE"
            | "ISVALIDJSON"
            | "JSONLENGTH"
            | "JSONHAS"
            | "JSONEXTRACTSTRING"
            | "JSONEXTRACTINT"
            | "JSONEXTRACTRAW"
            | "SUBSTRING"
            | "SUBSTRINGUTF8"
            | "HASSUBSEQUENCECASEINSENSITIVE"
            | "OVERLAY"
            | "FORMATREADABLEQUANTITY"
            | "QUERYSTRING"
            | "SPLIT_PART"
            | "SUBSTRING_INDEX"
            | "SUBSTRINGINDEXUTF8"
            | "QUERYSTRINGANDFRAGMENT"
            | "FORMATREADABLETIMEDELTA"
            | "TO_INT64"
            | "TOSTRING"
            | "TRIM"
            | "LTRIM"
            | "TRIMLEFT"
            | "BASENAME"
            | "FROMUNIXTIMESTAMP64MICRO"
            | "TRIM_WHITESPACE"
            | "TRY_TO_INT64"
            | "TOUINT8ORNULL"
            | "TOUINT16ORNULL"
            | "TOUINT32ORNULL"
            | "TOINT8ORNULL"
            | "TOINT16ORNULL"
            | "TOINT32ORNULL"
            | "TOUINT8ORZERO"
            | "TOUINT16ORZERO"
            | "TOUINT32ORZERO"
            | "TOINT8ORZERO"
            | "TOINT16ORZERO"
            | "TOINT32ORZERO"
            | "TOINT64ORZERO"
            | "TOUINT8ORDEFAULT"
            | "TOUINT16ORDEFAULT"
            | "TOUINT32ORDEFAULT"
            | "TOINT8ORDEFAULT"
            | "TOINT16ORDEFAULT"
            | "TOINT32ORDEFAULT"
            | "TOINT64ORDEFAULT"
            | "PARSEREADABLESIZE"
            | "PARSEREADABLESIZEORZERO"
            | "PARSEREADABLESIZEORNULL"
            | "TOINT64ORNULL"
            | "LPAD"
            | "LEFTPADUTF8"
            | "COUNTSUBSTRINGSCASEINSENSITIVE"
            | "HEX_DECODE"
            | "UNBIN"
            | "REPLACE"
            | "REPLACEALL"
            | "PORT"
            | "PROTOCOL"
            | "FROMUNIXTIMESTAMP64NANO"
            | "TRANSLATE"
            | "TRANSLATEUTF8"
            | "POSITIONCASEINSENSITIVE"
            | "STRPOS"
            | "TOSTARTOFISOYEAR"
            | "POSITIONUTF8"
            | "COUNT_SUBSTRING"
            | "COUNTSUBSTRINGS"
            | "EXTRACTURLPARAMETER"
            | "NULLIF"
            | "IFNULL"
            | "LAST_INDEX_OF"
            | "DATEADD"
            | "DATESUB"
            | "PATH"
            | "LEFT"
            | "LEFTUTF8"
            | "REPLACE_FIRST"
            | "REPLACEONE"
            | "STARTS_WITH"
            | "STARTSWITH"
            | "HASTOKEN"
            | "TOMONTHNUMSINCEEPOCH"
            | "CONTAINS"
            | "STRIP_PREFIX"
            | "RIGHT"
            | "RIGHTUTF8"
            | "BASE64_ENCODE"
            | "BASE64URLENCODE"
            | "BASE64ENCODE"
            | "HASTOKENCASEINSENSITIVE"
            | "ENDS_WITH"
            | "ENDSWITHUTF8"
            | "ENDSWITH"
            | "PATHFULL"
            | "REPLACE_LAST"
            | "SUBTRACTMINUTES"
            | "NETLOC"
            | "DOMAIN"
            | "STRIP_SUFFIX"
            | "SUBTRACTMONTHS"
            | "TOSTRINGCUTTOZERO"
            | "REPEAT"
            | "BASE64_DECODE"
            | "BASE64URLDECODE"
            | "TRYBASE64DECODE"
            | "BASE64DECODE"
            | "APPENDTRAILINGCHARIFABSENT"
            | "ADDMONTHS"
            | "RPAD"
            | "NOTEMPTY"
            | "SPACE"
            | "INTDIVORNULL"
            | "MODULOORNULL"
            | "FIRSTLINE"
            | "TOPLEVELDOMAIN"
            | "EDITDISTANCE"
            | "EDITDISTANCEUTF8"
            | "FORMATREADABLESIZE"
            | "FORMATREADABLEDECIMALSIZE"
            | "TOTYPENAME"
            | "POSITION"
            | "ENCODEURLFORMCOMPONENT"
            | "ISIPV4PRIVATE"
            | "ISIPV4LOOPBACK"
            | "ISIPV4MULTICAST"
            | "ISIPV4LINKLOCAL"
            | "ISIPV6UNIQUELOCAL"
            | "ISIPV6LOOPBACK"
            | "ISIPV6MULTICAST"
            | "NORMALIZEIPV6"
    )
}

fn parse_mac_address(input: &str) -> Option<u64> {
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

fn parse_readable_size_bytes(input: &str) -> Option<i64> {
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
    let rounded = bytes.checked_add(denominator - 1)?.checked_div(denominator)?;
    i64::try_from(rounded).ok()
}

fn bounded_edit_distance<T: Eq>(left: &[T], right: &[T]) -> Result<i64, ParseError> {
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

fn bounded_utf8_edit_distance(left: &str, right: &str) -> Result<i64, ParseError> {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    if left.is_empty() {
        return Ok(right.len() as i64);
    }
    if right.is_empty() {
        return Ok(left.len() as i64);
    }
    if left.len().saturating_mul(right.len()) > 2_000_000 {
        return Err(ParseError("editDistanceUTF8 exceeds computation budget".into()));
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

pub(super) fn parse(parser: &mut Parser) -> Result<ConstantValue, ParseError> {
    parse_at_depth(parser, 0)
}

fn parse_at_depth(parser: &mut Parser, depth: usize) -> Result<ConstantValue, ParseError> {
    if depth >= MAX_DEPTH {
        return Err(ParseError("string functions nest at most 16 levels".into()));
    }
    let name = parser.word()?.to_ascii_uppercase();
    require(parser, Token::LParen, "string function requires '('")?;
    if super::date_scalar::is_function(&name) {
        return super::date_scalar::parse_after_open(parser, &name, depth);
    }
    let value = match name.as_str() {

        "IPV4STRINGTONUMORNULL" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "IPv4StringToNumOrNull requires one argument",
            )?;
            match input {
                Some(input) => input
                    .parse::<std::net::Ipv4Addr>()
                    .ok()
                    .map_or(ConstantValue::Null, |address| {
                        ConstantValue::Integer(i64::from(u32::from(address)))
                    }),
                None => ConstantValue::Null,
            }
        }
        "IPV4STRINGTONUM" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "IPv4StringToNum requires one argument",
            )?;
            match input {
                Some(input) => {
                    let address = input.parse::<std::net::Ipv4Addr>().map_err(|_| {
                        ParseError("IPv4StringToNum requires a valid IPv4 address".into())
                    })?;
                    ConstantValue::Integer(i64::from(u32::from(address)))
                }
                None => ConstantValue::Null,
            }
        }
        "IPV4NUMTOSTRING" => {
            let value = integer_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "IPv4NumToString requires one argument",
            )?;
            match value {
                Some(value) => {
                    let value = u32::try_from(value).map_err(|_| {
                        ParseError("IPv4NumToString requires a UInt32 integer".into())
                    })?;
                    ConstantValue::Text(std::net::Ipv4Addr::from(value).to_string())
                }
                None => ConstantValue::Null,
            }
        }
        "JSONEXTRACTBOOL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "JSONExtractBool requires a path")?;
            let path = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "JSONExtractBool requires two arguments",
            )?;
            match (input, path) {
                (Some(input), Some(path)) => {
                    let json = crate::json::parse_json(&input)
                        .map_err(|error| ParseError(format!("invalid JSON: {error}")))?;
                    let is_true = json.at_path(&path).is_some_and(|value| value.raw() == "true");
                    ConstantValue::Integer(i64::from(is_true))
                }
                _ => ConstantValue::Null,
            }
        }
        "CUTURLPARAMETER" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "cutURLParameter requires a parameter name",
            )?;
            let parameter = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "cutURLParameter requires two arguments",
            )?;
            match (input, parameter) {
                (Some(input), Some(parameter)) => {
                    let (url, fragment) = input
                        .split_once('#')
                        .map_or((input.as_str(), None), |(url, fragment)| {
                            (url, Some(fragment))
                        });
                    if let Some((prefix, query)) = url.split_once('?') {
                        let kept: Vec<&str> = query
                            .split('&')
                            .filter(|pair| pair.split('=').next() != Some(parameter.as_str()))
                            .collect();
                        let mut output = String::with_capacity(input.len());
                        output.push_str(prefix);
                        if !kept.is_empty() {
                            output.push('?');
                            output.push_str(&kept.join("&"));
                        }
                        if let Some(fragment) = fragment {
                            output.push('#');
                            output.push_str(fragment);
                        }
                        ConstantValue::Text(output)
                    } else {
                        ConstantValue::Text(input)
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "DOMAINWITHOUTWWW" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "domainWithoutWWW requires one argument",
            )?;
            match input {
                Some(input) => {
                    let authority = input
                        .split_once("://")
                        .map_or(input.as_str(), |(_, rest)| rest)
                        .split(['/', '?', '#'])
                        .next()
                        .unwrap_or("");
                    let host_port = authority.rsplit('@').next().unwrap_or("");
                    let host = if let Some(bracketed) = host_port.strip_prefix('[') {
                        bracketed.split_once(']').map_or("", |(host, _)| host)
                    } else {
                        host_port.split(':').next().unwrap_or("")
                    };
                    ConstantValue::Text(
                        host.strip_prefix("www.").unwrap_or(host).to_owned(),
                    )
                }
                None => ConstantValue::Null,
            }
        }
        "CUTWWW" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "cutWWW requires one argument")?;
            match input {
                Some(input) => {
                    let authority_start = input.find("://").map_or(0, |index| index + 3);
                    let authority_end = input[authority_start..]
                        .find(['/', '?', '#'])
                        .map_or(input.len(), |index| authority_start + index);
                    let host_start = input[authority_start..authority_end]
                        .rfind('@')
                        .map_or(authority_start, |index| authority_start + index + 1);
                    if input[host_start..authority_end].starts_with("www.") {
                        let mut output = String::with_capacity(input.len() - 4);
                        output.push_str(&input[..host_start]);
                        output.push_str(&input[host_start + 4..]);
                        ConstantValue::Text(output)
                    } else {
                        ConstantValue::Text(input)
                    }
                }
                None => ConstantValue::Null,
            }
        }
        "IPV4STRINGTONUMORDEFAULT" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "IPv4StringToNumOrDefault requires one argument",
            )?;
            match input {
                Some(input) => {
                    let address = input.parse::<std::net::Ipv4Addr>().ok();
                    ConstantValue::Integer(address.map_or(0, |ip| i64::from(u32::from(ip))))
                }
                None => ConstantValue::Null,
            }
        }
        "IPV4NUMTOSTRINGCLASSC" => {
            let value = integer_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "IPv4NumToStringClassC requires one argument",
            )?;
            match value {
                Some(value) => {
                    let numeric = u32::try_from(value).map_err(|_| {
                        ParseError("IPv4NumToStringClassC requires UInt32".into())
                    })?;
                    let [a, b, c, _] = numeric.to_be_bytes();
                    ConstantValue::Text(format!("{a}.{b}.{c}.xxx"))
                }
                None => ConstantValue::Null,
            }
        }
        "MACSTRINGTONUM" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "MACStringToNum requires one argument")?;
            match input {
                Some(input) => {
                    let value = parse_mac_address(&input).unwrap_or(0);
                    ConstantValue::Integer(value as i64)
                }
                None => ConstantValue::Null,
            }
        }
        "MACNUMTOSTRING" => {
            let value = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "MACNumToString requires one argument")?;
            match value {
                Some(value) => {
                    let value = u64::try_from(value).map_err(|_| {
                        ParseError("MACNumToString requires a nonnegative integer".into())
                    })?;
                    if value > 0xFFFF_FFFF_FFFF {
                        return Err(ParseError("MACNumToString exceeds 48-bit MAC range".into()));
                    }
                    let octets = value.to_be_bytes();
                    ConstantValue::Text(format!(
                        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                        octets[2], octets[3], octets[4], octets[5], octets[6], octets[7],
                    ))
                }
                None => ConstantValue::Null,
            }
        }
        "MACSTRINGTOOUI" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "MACStringToOUI requires one argument")?;
            match input {
                Some(input) => {
                    let oui = parse_mac_address(&input).map_or(0_u64, |mac| mac >> 24);
                    ConstantValue::Integer(oui as i64)
                }
                None => ConstantValue::Null,
            }
        }
        "ISIPADDRESSINRANGE" => {
            let address = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "isIPAddressInRange requires a CIDR range")?;
            let range = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "isIPAddressInRange requires two arguments")?;
            match (address, range) {
                (Some(address), Some(range)) => {
                    let address = address.parse::<std::net::IpAddr>()
                        .map_err(|_| ParseError("invalid IP address".into()))?;
                    let (network, prefix) = range.split_once('/')
                        .ok_or_else(|| ParseError("invalid CIDR range".into()))?;
                    let network = network.parse::<std::net::IpAddr>()
                        .map_err(|_| ParseError("invalid CIDR network address".into()))?;
                    let prefix = prefix.parse::<u32>()
                        .map_err(|_| ParseError("invalid CIDR prefix length".into()))?;
                    let contains = match (address, network) {
                        (std::net::IpAddr::V4(address), std::net::IpAddr::V4(network))
                            if prefix <= 32 => {
                            let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
                            u32::from(address) & mask == u32::from(network) & mask
                        }
                        (std::net::IpAddr::V6(address), std::net::IpAddr::V6(network))
                            if prefix <= 128 => {
                            let mask = if prefix == 0 { 0 } else { u128::MAX << (128 - prefix) };
                            u128::from(address) & mask == u128::from(network) & mask
                        }
                        _ => return Err(ParseError("incompatible CIDR address or prefix".into())),
                    };
                    ConstantValue::Integer(i64::from(contains))
                }
                _ => ConstantValue::Null,
            }
        }
        "PORT" => {
            let input = text_arg(parser, depth + 1)?;
            let fallback = if parser.take(Token::Comma) {
                integer_arg(parser, depth + 1)?
            } else {
                Some(0)
            };
            require(parser, Token::RParen, "port requires one or two arguments")?;
            match (input, fallback) {
                (Some(input), Some(fallback)) => {
                    let fallback = u16::try_from(fallback)
                        .map_err(|_| ParseError("port default must be UInt16".into()))?;
                    let authority = input
                        .split_once("://")
                        .map_or(input.as_str(), |(_, rest)| rest)
                        .split(['/', '?', '#'])
                        .next()
                        .unwrap_or("");
                    let host_port = authority.rsplit('@').next().unwrap_or("");
                    let number = if host_port.starts_with('[') {
                        host_port
                            .split_once("]:")
                            .and_then(|(_, port)| port.parse::<u16>().ok())
                    } else {
                        host_port
                            .rsplit_once(':')
                            .and_then(|(_, port)| port.parse::<u16>().ok())
                    };
                    ConstantValue::Integer(i64::from(number.unwrap_or(fallback)))
                }
                _ => ConstantValue::Null,
            }
        }
        "FORMATREADABLEQUANTITY" => {
            let value = integer_arg(parser, depth + 1)?;
            let precision = if parser.take(Token::Comma) {
                integer_arg(parser, depth + 1)?
            } else {
                Some(2)
            };
            require(parser, Token::RParen, "FORMATREADABLEQUANTITY requires one value")?;
            match (value, precision) {
                (Some(value), Some(precision)) => {
                    let precision = usize::try_from(precision)
                        .map_err(|_| ParseError("readable number precision must be nonnegative".into()))?;
                    if precision > 8 {
                        return Err(ParseError("readable number precision exceeds 8".into()));
                    }
                    const UNITS: &[&str] = &["", " thousand", " million", " billion", " trillion", " quadrillion", " quintillion"];
                    let mut number = value as f64;
                    let mut index = 0;
                    while number.abs() >= 1000.0 && index + 1 < UNITS.len() {
                        number /= 1000.0;
                        index += 1;
                    }
                    ConstantValue::Text(format!("{number:.precision$}{}", UNITS[index]))
                }
                _ => ConstantValue::Null,
            }
        }
        "NORMALIZEIPV6" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "normalizeIPv6 requires one argument")?;
            match input {
                Some(value) => {
                    let address = value.parse::<std::net::Ipv6Addr>()
                        .map_err(|_| ParseError("invalid IPv6 address".into()))?;
                    ConstantValue::Text(address.to_string())
                }
                None => ConstantValue::Null,
            }
        }
        "ISIPV6MULTICAST" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "isIPv6Multicast requires one argument")?;
            match input {
                Some(value) => {
                    let address = value.parse::<std::net::Ipv6Addr>()
                        .map_err(|_| ParseError("invalid IPv6 address".into()))?;
                    ConstantValue::Integer(i64::from(address.is_multicast()))
                }
                None => ConstantValue::Null,
            }
        }
        "ISIPV6LOOPBACK" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "isIPv6Loopback requires one argument")?;
            match input {
                Some(value) => {
                    let address = value.parse::<std::net::Ipv6Addr>()
                        .map_err(|_| ParseError("invalid IPv6 address".into()))?;
                    ConstantValue::Integer(i64::from(address.is_loopback()))
                }
                None => ConstantValue::Null,
            }
        }
        "ISIPV6UNIQUELOCAL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "isIPv6UniqueLocal requires one argument")?;
            match input {
                Some(value) => {
                    let address = value.parse::<std::net::Ipv6Addr>()
                        .map_err(|_| ParseError("invalid IPv6 address".into()))?;
                    ConstantValue::Integer(i64::from(address.is_unique_local()))
                }
                None => ConstantValue::Null,
            }
        }
        "ISIPV4LINKLOCAL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "isIPv4LinkLocal requires one argument")?;
            match input {
                Some(value) => {
                    let address = value.parse::<std::net::Ipv4Addr>()
                        .map_err(|_| ParseError("invalid IPv4 address".into()))?;
                    ConstantValue::Integer(i64::from(address.is_link_local()))
                }
                None => ConstantValue::Null,
            }
        }
        "ISIPV4MULTICAST" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "isIPv4Multicast requires one argument")?;
            match input {
                Some(value) => {
                    let address = value.parse::<std::net::Ipv4Addr>()
                        .map_err(|_| ParseError("invalid IPv4 address".into()))?;
                    ConstantValue::Integer(i64::from(address.is_multicast()))
                }
                None => ConstantValue::Null,
            }
        }
        "ISIPV4LOOPBACK" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "isIPv4Loopback requires one argument")?;
            match input {
                Some(value) => {
                    let address = value.parse::<std::net::Ipv4Addr>()
                        .map_err(|_| ParseError("invalid IPv4 address".into()))?;
                    ConstantValue::Integer(i64::from(address.is_loopback()))
                }
                None => ConstantValue::Null,
            }
        }
        "ISIPV4PRIVATE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "isIPv4Private requires one argument")?;
            match input {
                Some(value) => {
                    let address = value.parse::<std::net::Ipv4Addr>()
                        .map_err(|_| ParseError("invalid IPv4 address".into()))?;
                    ConstantValue::Integer(i64::from(address.is_private()))
                }
                None => ConstantValue::Null,
            }
        }
        "FIRSTLINE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "firstLine requires one argument")?;
            match input {
                Some(input) => {
                    let end = input.find(['\r', '\n']).unwrap_or(input.len());
                    ConstantValue::Text(input[..end].to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "TOPLEVELDOMAIN" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "topLevelDomain requires one argument")?;
            match input {
                Some(input) => {
                    let authority = input
                        .split_once("://")
                        .map_or(input.as_str(), |(_, rest)| rest)
                        .split(['/', '?', '#'])
                        .next()
                        .unwrap_or("");
                    let host_port = authority.rsplit('@').next().unwrap_or("");
                    let host = host_port.split(':').next().unwrap_or("");
                    let labels: Vec<&str> = host.split('.').collect();
                    let valid = labels.len() >= 2
                        && host.parse::<std::net::Ipv4Addr>().is_err()
                        && labels.iter().all(|label| {
                            label.len() <= 63
                                && !label.is_empty()
                                && label.as_bytes()[0].is_ascii_alphanumeric()
                                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                                && label.bytes().all(|byte| {
                                    byte.is_ascii_alphanumeric() || byte == b'-'
                                })
                        });
                    ConstantValue::Text(
                        if valid {
                            labels.last().copied().unwrap_or("")
                        } else {
                            ""
                        }
                        .to_owned(),
                    )
                }
                None => ConstantValue::Null,
            }
        }
        "EDITDISTANCE" => {
            let left = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "editDistance requires two strings")?;
            let right = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "editDistance requires two strings")?;
            match (left, right) {
                (Some(left), Some(right)) => ConstantValue::Integer(
                    bounded_edit_distance(left.as_bytes(), right.as_bytes())?,
                ),
                _ => ConstantValue::Null,
            }
        }
        "EDITDISTANCEUTF8" => {
            let left = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "editDistanceUTF8 requires two strings")?;
            let right = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "editDistanceUTF8 requires two strings")?;
            match (left, right) {
                (Some(left), Some(right)) => ConstantValue::Integer(
                    bounded_utf8_edit_distance(&left, &right)?,
                ),
                _ => ConstantValue::Null,
            }
        }
        "FORMATREADABLESIZE" => {
            let value = integer_arg(parser, depth + 1)?;
            let precision = if parser.take(Token::Comma) {
                integer_arg(parser, depth + 1)?
            } else {
                Some(2)
            };
            require(parser, Token::RParen, "FORMATREADABLESIZE requires one value")?;
            match (value, precision) {
                (Some(value), Some(precision)) => {
                    let precision = usize::try_from(precision)
                        .map_err(|_| ParseError("readable number precision must be nonnegative".into()))?;
                    if precision > 8 {
                        return Err(ParseError("readable number precision exceeds 8".into()));
                    }
                    const UNITS: &[&str] = &[" B", " KiB", " MiB", " GiB", " TiB", " PiB", " EiB"];
                    let mut number = value as f64;
                    let mut index = 0;
                    while number.abs() >= 1024.0 && index + 1 < UNITS.len() {
                        number /= 1024.0;
                        index += 1;
                    }
                    ConstantValue::Text(format!("{number:.precision$}{}", UNITS[index]))
                }
                _ => ConstantValue::Null,
            }
        }
        "FORMATREADABLEDECIMALSIZE" => {
            let value = integer_arg(parser, depth + 1)?;
            let precision = if parser.take(Token::Comma) {
                integer_arg(parser, depth + 1)?
            } else {
                Some(2)
            };
            require(parser, Token::RParen, "FORMATREADABLEDECIMALSIZE requires one value")?;
            match (value, precision) {
                (Some(value), Some(precision)) => {
                    let precision = usize::try_from(precision)
                        .map_err(|_| ParseError("readable number precision must be nonnegative".into()))?;
                    if precision > 8 {
                        return Err(ParseError("readable number precision exceeds 8".into()));
                    }
                    const UNITS: &[&str] = &[" B", " KB", " MB", " GB", " TB", " PB", " EB"];
                    let mut number = value as f64;
                    let mut index = 0;
                    while number.abs() >= 1000.0 && index + 1 < UNITS.len() {
                        number /= 1000.0;
                        index += 1;
                    }
                    ConstantValue::Text(format!("{number:.precision$}{}", UNITS[index]))
                }
                _ => ConstantValue::Null,
            }
        }
        "TOTYPENAME" => {
            let input = constant_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "toTypeName requires one argument")?;
            let name = match input {
                ConstantValue::Integer(_) => "Int64",
                ConstantValue::Text(_) => "String",
                ConstantValue::Null => "Nullable(Nothing)",
            };
            ConstantValue::Text(name.into())
        }
        "POSITION" => {
            let haystack = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "position requires two arguments")?;
            let needle = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "position requires two arguments")?;
            match (haystack, needle) {
                (Some(haystack), Some(needle)) => ConstantValue::Integer(
                    i64::try_from(haystack.find(&needle).map_or(0, |index| index + 1))
                        .map_err(|_| ParseError("position result exceeds Int64".into()))?,
                ),
                _ => ConstantValue::Null,
            }
        }
        "MODULOORNULL" => {
            let dividend = integer_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "moduloOrNull requires two arguments")?;
            let divisor = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "moduloOrNull requires two arguments")?;
            match (dividend, divisor) {
                (Some(left), Some(right)) => left
                    .checked_rem(right)
                    .map_or(ConstantValue::Null, ConstantValue::Integer),
                _ => ConstantValue::Null,
            }
        }
        "INTDIVORNULL" => {
            let dividend = integer_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "intDivOrNull requires two arguments")?;
            let divisor = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "intDivOrNull requires two arguments")?;
            match (dividend, divisor) {
                (Some(left), Some(right)) => left
                    .checked_div(right)
                    .map_or(ConstantValue::Null, ConstantValue::Integer),
                _ => ConstantValue::Null,
            }
        }
        "LENGTHUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "lengthUTF8 requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(
                    i64::try_from(if value.is_ascii() {
                        value.len()
                    } else {
                        value.chars().count()
                    })
                    .map_err(|_| ParseError("lengthUTF8 result exceeds Int64".into()))?,
                ),
                None => ConstantValue::Null,
            }
        }
        "TOSTARTOFFIVEMINUTES" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "toStartOfFiveMinutes requires one argument")?;
            match input {
                Some(value) => {
                    let seconds = rusthouse_storage::parse_datetime(&value)
                        .ok_or_else(|| ParseError("invalid UTC DateTime argument".into()))?;
                    ConstantValue::Text(rusthouse_storage::format_datetime(seconds / 300 * 300))
                }
                None => ConstantValue::Null,
            }
        }
        "FORMATREADABLETIMEDELTA" => {
            let value = integer_arg(parser, depth + 1)?;
            let maximum_unit = if parser.take(Token::Comma) {
                Some(text_arg(parser, depth + 1)?)
            } else {
                None
            };
            let minimum_unit = if parser.take(Token::Comma) {
                Some(text_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(parser, Token::RParen, "formatReadableTimeDelta requires ')'")?;
            if value.is_none() || matches!(maximum_unit, Some(None))
                || matches!(minimum_unit, Some(None))
            {
                ConstantValue::Null
            } else {
                let maximum = maximum_unit.flatten().unwrap_or_else(|| "years".into());
                let minimum = minimum_unit.flatten().unwrap_or_else(|| "seconds".into());
                const UNITS: &[(&str, u64)] = &[
                    ("years", 31_536_000), ("months", 2_635_200),
                    ("days", 86_400), ("hours", 3_600),
                    ("minutes", 60), ("seconds", 1),
                ];
                let max_index = UNITS.iter().position(|(name, _)| *name == maximum)
                    .ok_or_else(|| ParseError("invalid maximum time unit".into()))?;
                let min_index = UNITS.iter().position(|(name, _)| *name == minimum)
                    .ok_or_else(|| ParseError("invalid minimum time unit".into()))?;
                if max_index > min_index {
                    return Err(ParseError("minimum unit exceeds maximum unit".into()));
                }
                let seconds = value.expect("validated nonnull time value");
                let mut remaining = seconds.unsigned_abs();
                let mut parts = Vec::new();
                for (index, &(name, unit_seconds)) in UNITS.iter().enumerate()
                    .skip(max_index).take(min_index - max_index + 1)
                {
                    let count = remaining / unit_seconds;
                    remaining %= unit_seconds;
                    if count > 0 || (index == min_index && parts.is_empty()) {
                        let singular = name.trim_end_matches('s');
                        parts.push(format!("{count} {singular}{}", if count == 1 { "" } else { "s" }));
                    }
                }
                let mut formatted = if parts.len() <= 1 {
                    parts.join("")
                } else {
                    let last = parts.pop().expect("at least two time parts");
                    format!("{} and {last}", parts.join(", "))
                };
                if seconds < 0 {
                    formatted.insert(0, '-');
                }
                ConstantValue::Text(formatted)
            }
        }
        "TOSTARTOFMINUTE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "toStartOfMinute requires one argument")?;
            match input {
                Some(value) => {
                    let seconds = rusthouse_storage::parse_datetime(&value).ok_or_else(|| {
                        ParseError("toStartOfMinute requires a valid UTC DateTime".into())
                    })?;
                    ConstantValue::Text(rusthouse_storage::format_datetime(seconds - seconds % 60))
                }
                None => ConstantValue::Null,
            }
        }
        "ASCII" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "ASCII requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(
                    value
                        .chars()
                        .next()
                        .map(|ch| i64::from(ch as u32))
                        .unwrap_or(0),
                ),
                None => ConstantValue::Null,
            }
        }
        "LENGTH" | "CHAR_LENGTH" | "OCTET_LENGTH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "LENGTH requires one argument")?;
            match input {
                Some(value) => {
                    let length = if name == "CHAR_LENGTH" {
                        value.chars().count()
                    } else {
                        value.len()
                    };
                    ConstantValue::Integer(
                        i64::try_from(length)
                            .map_err(|_| ParseError("string length exceeds Int64".into()))?,
                    )
                }
                None => ConstantValue::Null,
            }
        }
        "TOSTARTOFFIFTEENMINUTES" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "toStartOfFifteenMinutes requires one argument")?;
            match input {
                Some(value) => {
                    let seconds = rusthouse_storage::parse_datetime(&value)
                        .ok_or_else(|| ParseError("invalid UTC DateTime argument".into()))?;
                    ConstantValue::Text(rusthouse_storage::format_datetime(seconds / 900 * 900))
                }
                None => ConstantValue::Null,
            }
        }
        "CUTQUERYSTRING" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "cutQueryString requires one argument",
            )?;
            match input {
                Some(input) => {
                    let (before_fragment, fragment) = match input.split_once('#') {
                        Some((head, tail)) => (head, Some(tail)),
                        None => (input.as_str(), None),
                    };
                    let prefix = before_fragment
                        .split_once('?')
                        .map_or(before_fragment, |(head, _)| head);
                    let mut output = prefix.to_owned();
                    if let Some(fragment) = fragment {
                        output.push('#');
                        output.push_str(fragment);
                    }
                    ConstantValue::Text(output)
                }
                None => ConstantValue::Null,
            }
        }
        "DECODEXMLCOMPONENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "decodeXMLComponent requires one string")?;
            match input {
                Some(input) => {
                    let mut output = String::with_capacity(input.len());
                    let mut remaining = input.as_str();
                    while let Some(offset) = remaining.find('&') {
                        output.push_str(&remaining[..offset]);
                        remaining = &remaining[offset..];
                        let Some(end) = remaining.find(';') else {
                            break;
                        };
                        if end > 12 {
                            output.push('&');
                            remaining = &remaining[1..];
                            continue;
                        }
                        let entity = &remaining[1..end];
                        let decoded = match entity {
                            "amp" => Some('&'),
                            "lt" => Some('<'),
                            "gt" => Some('>'),
                            "quot" => Some('"'),
                            "apos" => Some(char::from(39)),
                            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                                u32::from_str_radix(&entity[2..], 16)
                                    .ok()
                                    .and_then(char::from_u32)
                            }
                            _ if entity.starts_with('#') => {
                                entity[1..].parse::<u32>().ok().and_then(char::from_u32)
                            }
                            _ => None,
                        };
                        match decoded {
                            Some(character) => {
                                output.push(character);
                                remaining = &remaining[end + 1..];
                            }
                            None => {
                                output.push('&');
                                remaining = &remaining[1..];
                            }
                        }
                        if output.len() > MAX_TEXT_BYTES {
                            return Err(ParseError("decodeXMLComponent result is too large".into()));
                        }
                    }
                    output.push_str(remaining);
                    if output.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("decodeXMLComponent result is too large".into()));
                    }
                    ConstantValue::Text(output)
                }
                None => ConstantValue::Null,
            }
        }
        "ENCODEXMLCOMPONENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "encodeXMLComponent requires one string")?;
            match input {
                Some(input) => {
                    let mut result = String::new();
                    for character in input.chars() {
                        let escaped = match character {
                            '&' => "&amp;",
                            '<' => "&lt;",
                            '>' => "&gt;",
                            '"' => "&quot;",
                            '\'' => "&apos;",
                            _ => "",
                        };
                        let increase = if escaped.is_empty() {
                            character.len_utf8()
                        } else {
                            escaped.len()
                        };
                        if result.len() + increase > MAX_TEXT_BYTES {
                            return Err(ParseError("encodeXMLComponent result is too large".into()));
                        }
                        if escaped.is_empty() {
                            result.push(character);
                        } else {
                            result.push_str(escaped);
                        }
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "HEX_ENCODE" | "HEX" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "HEX_ENCODE requires one argument")?;
            match input {
                Some(value) => {
                    let length = value
                        .len()
                        .checked_mul(2)
                        .ok_or_else(|| ParseError("HEX_ENCODE result is too large".into()))?;
                    if length > MAX_TEXT_BYTES {
                        return Err(ParseError("HEX_ENCODE result is too large".into()));
                    }
                    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
                    let mut output = String::with_capacity(length);
                    for byte in value.bytes() {
                        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
                        output.push(char::from(DIGITS[usize::from(byte & 15)]));
                    }
                    ConstantValue::Text(output)
                }
                None => ConstantValue::Null,
            }
        }
        "ISIPV6STRING" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "isIPv6String requires one argument")?;
            match input {
                Some(value) => {
                    let is_ipv6 = value.parse::<std::net::Ipv6Addr>().is_ok();
                    ConstantValue::Integer(i64::from(is_ipv6))
                }
                None => ConstantValue::Null,
            }
        }
        "UTF16_LENGTH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "UTF16_LENGTH requires one argument")?;
            match input {
                Some(value) => {
                    let length = i64::try_from(value.encode_utf16().count())
                        .map_err(|_| ParseError("UTF16_LENGTH result exceeds Int64".into()))?;
                    ConstantValue::Integer(length)
                }
                None => ConstantValue::Null,
            }
        }
        "TOUNIXTIMESTAMP" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "toUnixTimestamp requires one argument")?;
            match input {
                Some(value) => {
                    let seconds = rusthouse_storage::parse_datetime(&value)
                        .ok_or_else(|| ParseError("invalid UTC DateTime argument".into()))?;
                    ConstantValue::Integer(i64::from(seconds))
                }
                None => ConstantValue::Null,
            }
        }
        "STRINGBYTESUNIQ" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "stringBytesUniq requires one argument",
            )?;
            match input {
                Some(value) => {
                    // Four machine words replace 256 boolean slots and their
                    // full-length counting pass for short SQL strings.
                    let mut bitmap = [0u64; 4];
                    for byte in value.bytes() {
                        bitmap[usize::from(byte) / 64] |= 1u64 << (byte % 64);
                    }
                    ConstantValue::Integer(
                        bitmap.iter().map(|word| i64::from(word.count_ones())).sum(),
                    )
                }
                None => ConstantValue::Null,
            }
        }
        "BITMASKTOLIST" => {
            let input = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "bitmaskToList requires one argument")?;
            match input {
                Some(input) => {
                    let mut bits = input as u64;
                    let mut result = String::new();
                    while bits != 0 {
                        let bit = bits.trailing_zeros();
                        let magnitude = 1u64 << bit;
                        if !result.is_empty() {
                            result.push(',');
                        }
                        result.push_str(&magnitude.to_string());
                        bits &= bits - 1;
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "BIT_LENGTH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "BIT_LENGTH requires one argument")?;
            match input {
                Some(value) => {
                    let bits = value
                        .len()
                        .checked_mul(8)
                        .and_then(|length| i64::try_from(length).ok())
                        .ok_or_else(|| ParseError("BIT_LENGTH result exceeds Int64".into()))?;
                    ConstantValue::Integer(bits)
                }
                None => ConstantValue::Null,
            }
        }
        "LOWER" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "LOWER requires one argument")?;
            match input {
                Some(value) => {
                    let result = value.to_ascii_lowercase();
                    if result.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("LOWER result is too large".into()));
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "BYTEHAMMINGDISTANCE" => {
            let left = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "byteHammingDistance requires two strings")?;
            let right = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after byteHammingDistance")?;
            match (left, right) {
                (Some(left), Some(right)) => {
                    if left.len() != right.len() {
                        return Err(ParseError("byteHammingDistance requires equal byte lengths".into()));
                    }
                    let mismatches = left.bytes().zip(right.bytes())
                        .filter(|(a, b)| a != b).count();
                    ConstantValue::Integer(i64::try_from(mismatches).map_err(|_| {
                        ParseError("byteHammingDistance count exceeds Int64".into())
                    })?)
                }
                _ => ConstantValue::Null,
            }
        }
        "HASSUBSEQUENCE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "hasSubsequence requires a needle")?;
            let needle = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after hasSubsequence")?;
            match (input, needle) {
                (Some(input), Some(needle)) => {
                    let mut required = needle.bytes();
                    let mut next = required.next();
                    for byte in input.bytes() {
                        if next == Some(byte) {
                            next = required.next();
                        }
                    }
                    ConstantValue::Integer(i64::from(next.is_none()))
                }
                _ => ConstantValue::Null,
            }
        }
        "CHR" => {
            let input = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "CHR requires one argument")?;
            match input {
                Some(value) => {
                    let character = u32::try_from(value)
                        .ok()
                        .filter(|code| *code != 0)
                        .and_then(char::from_u32)
                        .ok_or_else(|| {
                            ParseError("CHR requires a nonzero Unicode scalar".into())
                        })?;
                    ConstantValue::Text(character.to_string())
                }
                None => ConstantValue::Null,
            }
        }
        "UPPERUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "UPPERUTF8 requires one argument")?;
            match input {
                Some(mut input) if input.is_ascii() => {
                    input.make_ascii_uppercase();
                    ConstantValue::Text(input)
                }
                Some(input) => {
                    let output = input.to_uppercase();
                    if output.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("upperUTF8 result is too large".into()));
                    }
                    ConstantValue::Text(output)
                }
                None => ConstantValue::Null,
            }
        }
        "UPPER" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "UPPER requires one argument")?;
            match input {
                Some(value) => {
                    let result = value.to_ascii_uppercase();
                    if result.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("UPPER result is too large".into()));
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "TOSTARTOFTENMINUTES" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "toStartOfTenMinutes requires one argument")?;
            match input {
                Some(value) => {
                    let seconds = rusthouse_storage::parse_datetime(&value).ok_or_else(|| {
                        ParseError("toStartOfTenMinutes requires a valid UTC DateTime".into())
                    })?;
                    ConstantValue::Text(rusthouse_storage::format_datetime(seconds - seconds % 600))
                }
                None => ConstantValue::Null,
            }
        }
        "INITCAP" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "INITCAP requires one argument")?;
            match input {
                Some(value) => {
                    let mut result = String::with_capacity(value.len());
                    let mut word_start = true;
                    for ch in value.chars() {
                        let transformed = if word_start {
                            ch.to_uppercase().to_string()
                        } else {
                            ch.to_lowercase().to_string()
                        };
                        if result.len() + transformed.len() > MAX_TEXT_BYTES {
                            return Err(ParseError("INITCAP result is too large".into()));
                        }
                        result.push_str(&transformed);
                        word_start = !ch.is_alphanumeric();
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "ISIPV4STRING" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "isIPv4String requires one argument")?;
            match input {
                Some(value) => {
                    let is_ipv4 = value.parse::<std::net::Ipv4Addr>().is_ok();
                    ConstantValue::Integer(i64::from(is_ipv4))
                }
                None => ConstantValue::Null,
            }
        }
        "IS_ASCII" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "IS_ASCII requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(i64::from(value.is_ascii())),
                None => ConstantValue::Null,
            }
        }
        "IS_EMPTY" | "EMPTY" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "IS_EMPTY requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(i64::from(value.is_empty())),
                None => ConstantValue::Null,
            }
        }
        "TOMODIFIEDJULIANDAY" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "toModifiedJulianDay requires one argument",
            )?;
            match input {
                Some(value) => {
                    let days = rusthouse_storage::parse_date(&value).ok_or_else(|| {
                        ParseError("toModifiedJulianDay requires a valid UTC Date".into())
                    })?;
                    ConstantValue::Integer(i64::from(days) + 40_587)
                }
                None => ConstantValue::Null,
            }
        }
        "IS_INT64" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "IS_INT64 requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(i64::from(value.parse::<i64>().is_ok())),
                None => ConstantValue::Null,
            }
        }
        "REVERSE" | "REVERSEUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "REVERSE requires one argument")?;
            match input {
                Some(value) if value.is_ascii() => {
                    let mut bytes = value.into_bytes();
                    bytes.reverse();
                    ConstantValue::Text(
                        String::from_utf8(bytes).expect("reversed ASCII bytes remain valid UTF-8"),
                    )
                }
                Some(value) => ConstantValue::Text(value.chars().rev().collect()),
                None => ConstantValue::Null,
            }
        }
        "SUBTRACTMINUTES" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "subtractMinutes requires an offset")?;
            let amount = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "subtractMinutes requires two arguments")?;
            match (input, amount) {
                (Some(value), Some(amount)) => {
                    let seconds = rusthouse_storage::parse_datetime(&value)
                        .ok_or_else(|| ParseError("invalid UTC DateTime argument".into()))?;
                    let delta = amount
                        .checked_mul(60)
                        .ok_or_else(|| ParseError("subtractMinutes offset overflow".into()))?;
                    let shifted = i64::from(seconds)
                        .checked_sub(delta)
                        .and_then(|value| u32::try_from(value).ok())
                        .ok_or_else(|| ParseError("subtractMinutes result outside UTC DateTime range".into()))?;
                    ConstantValue::Text(rusthouse_storage::format_datetime(shifted))
                }
                _ => ConstantValue::Null,
            }
        }
        "CUTFRAGMENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "cutFragment requires one argument")?;
            match input {
                Some(input) => ConstantValue::Text(
                    input
                        .split_once('#')
                        .map_or(input.as_str(), |(prefix, _)| prefix)
                        .to_owned(),
                ),
                None => ConstantValue::Null,
            }
        }
        "SUBTRACTHOURS" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "subtractHours requires an offset")?;
            let amount = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "subtractHours requires two arguments")?;
            match (input, amount) {
                (Some(value), Some(amount)) => {
                    let seconds = rusthouse_storage::parse_datetime(&value)
                        .ok_or_else(|| ParseError("invalid UTC DateTime argument".into()))?;
                    let delta = amount
                        .checked_mul(3600)
                        .ok_or_else(|| ParseError("subtractHours offset overflow".into()))?;
                    let shifted = i64::from(seconds)
                        .checked_sub(delta)
                        .and_then(|value| u32::try_from(value).ok())
                        .ok_or_else(|| ParseError("subtractHours result outside UTC DateTime range".into()))?;
                    ConstantValue::Text(rusthouse_storage::format_datetime(shifted))
                }
                _ => ConstantValue::Null,
            }
        }
        "URL_ENCODE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "URL_ENCODE requires one argument")?;
            match input {
                Some(input) => {
                    fn unreserved(byte: u8) -> bool {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
                    }
                    let mut length = 0usize;
                    for byte in input.bytes() {
                        length = length
                            .checked_add(if unreserved(byte) { 1 } else { 3 })
                            .ok_or_else(|| ParseError("URL_ENCODE result is too large".into()))?;
                        if length > MAX_TEXT_BYTES {
                            return Err(ParseError("URL_ENCODE result is too large".into()));
                        }
                    }
                    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
                    let mut result = String::with_capacity(length);
                    for byte in input.bytes() {
                        if unreserved(byte) {
                            result.push(char::from(byte));
                        } else {
                            result.push('%');
                            result.push(char::from(DIGITS[usize::from(byte >> 4)]));
                            result.push(char::from(DIGITS[usize::from(byte & 15)]));
                        }
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "ENCODEURLFORMCOMPONENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "encodeURLFormComponent requires one argument")?;
            match input {
                Some(input) => {
                    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
                    let mut output = String::new();
                    for byte in input.bytes() {
                        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                            output.push(char::from(byte));
                        } else if byte == b' ' {
                            output.push('+');
                        } else {
                            output.push('%');
                            output.push(char::from(DIGITS[usize::from(byte >> 4)]));
                            output.push(char::from(DIGITS[usize::from(byte & 15)]));
                        }
                        if output.len() > MAX_TEXT_BYTES {
                            return Err(ParseError("encodeURLFormComponent result is too large".into()));
                        }
                    }
                    ConstantValue::Text(output)
                }
                None => ConstantValue::Null,
            }
        }
        "CUTQUERYSTRINGANDFRAGMENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "cutQueryStringAndFragment requires one argument",
            )?;
            match input {
                Some(input) => {
                    let end = input.find(['?', '#']).unwrap_or(input.len());
                    ConstantValue::Text(input[..end].to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "TOLASTDAYOFWEEK" => {
            let input = text_arg(parser, depth + 1)?;
            let mode = if parser.take(Token::Comma) {
                integer_arg(parser, depth + 1)?
            } else {
                Some(0)
            };
            require(parser, Token::RParen, "toLastDayOfWeek requires one or two arguments")?;
            match (input, mode) {
                (Some(value), Some(mode)) => {
                    if !matches!(mode, 0 | 1) {
                        return Err(ParseError("toLastDayOfWeek only supports week modes 0 and 1".into()));
                    }
                    let days = rusthouse_storage::parse_date(&value).map(u32::from)
                        .or_else(|| rusthouse_storage::parse_datetime(&value).map(|seconds| seconds / 86_400))
                        .ok_or_else(|| ParseError("toLastDayOfWeek requires a valid UTC Date or DateTime".into()))?;
                    let weekday = (days + if mode == 1 { 3 } else { 4 }) % 7;
                    let days = days.checked_add(6 - weekday).ok_or_else(|| {
                        ParseError("toLastDayOfWeek result overflows Date".into())
                    })?;
                    let days = u16::try_from(days).map_err(|_| {
                        ParseError("toLastDayOfWeek result exceeds the Date range".into())
                    })?;
                    ConstantValue::Text(rusthouse_storage::format_date(days))
                }
                _ => ConstantValue::Null,
            }
        }
        "COALESCE" => {
            let mut first = None;
            let mut count = 0;
            loop {
                if count == 16 {
                    return Err(ParseError("COALESCE supports at most 16 arguments".into()));
                }
                let value = text_arg(parser, depth + 1)?;
                if first.is_none() {
                    first = value;
                }
                count += 1;
                if !parser.take(Token::Comma) {
                    break;
                }
            }
            require(
                parser,
                Token::RParen,
                "expected ')' after COALESCE arguments",
            )?;
            first.map_or(ConstantValue::Null, ConstantValue::Text)
        }
        "CONCAT_WS" | "CONCATWITHSEPARATOR" => {
            let separator = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "CONCAT_WS requires at least one value",
            )?;
            let mut result = String::new();
            let mut written = false;
            let mut count = 0;
            loop {
                if count == 16 {
                    return Err(ParseError("CONCAT_WS supports at most 16 values".into()));
                }
                let value = text_arg(parser, depth + 1)?;
                if let (Some(separator), Some(value)) = (separator.as_ref(), value) {
                    let separator_len = if written { separator.len() } else { 0 };
                    let total = result
                        .len()
                        .checked_add(separator_len)
                        .and_then(|size| size.checked_add(value.len()))
                        .ok_or_else(|| ParseError("CONCAT_WS result is too large".into()))?;
                    if total > MAX_TEXT_BYTES {
                        return Err(ParseError("CONCAT_WS result is too large".into()));
                    }
                    if written {
                        result.push_str(separator);
                    }
                    result.push_str(&value);
                    written = true;
                }
                count += 1;
                if !parser.take(Token::Comma) {
                    break;
                }
            }
            require(
                parser,
                Token::RParen,
                "expected ')' after CONCAT_WS arguments",
            )?;
            if separator.is_some() {
                ConstantValue::Text(result)
            } else {
                ConstantValue::Null
            }
        }
        "FROMUNIXTIMESTAMP64MILLI" => {
            let input = integer_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "fromUnixTimestamp64Milli requires one argument",
            )?;
            match input {
                Some(input) => {
                    let unit = 10_i64.pow(3);
                    let seconds = u32::try_from(input.div_euclid(unit)).map_err(|_| {
                        ParseError("fromUnixTimestamp64Milli requires a supported UTC DateTime".into())
                    })?;
                    let fraction = input.rem_euclid(unit);
                    let datetime = rusthouse_storage::format_datetime(seconds);
                    ConstantValue::Text(format!(
                        "{datetime}.{fraction:0width$}",
                        width = 3
                    ))
                }
                None => ConstantValue::Null,
            }
        }
        "QUOTE_LITERAL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "QUOTE_LITERAL requires one argument")?;
            match input {
                Some(value) => {
                    let quotes = value.bytes().filter(|byte| *byte == b'\'').count();
                    let length = value
                        .len()
                        .checked_add(quotes)
                        .and_then(|n| n.checked_add(2))
                        .ok_or_else(|| ParseError("QUOTE_LITERAL result is too large".into()))?;
                    if length > MAX_TEXT_BYTES {
                        return Err(ParseError("QUOTE_LITERAL result is too large".into()));
                    }
                    let mut output = String::with_capacity(length);
                    output.push('\'');
                    for ch in value.chars() {
                        if ch == '\'' {
                            output.push('\'');
                        }
                        output.push(ch);
                    }
                    output.push('\'');
                    ConstantValue::Text(output)
                }
                None => ConstantValue::Null,
            }
        }
        "CONCAT" | "CONCATASSUMEINJECTIVE" => {
            let mut result = String::new();
            let mut count = 0;
            let mut is_null = false;
            loop {
                if count == 16 {
                    return Err(ParseError("CONCAT supports at most 16 arguments".into()));
                }
                match text_arg(parser, depth + 1)? {
                    Some(value) => {
                        let total = result
                            .len()
                            .checked_add(value.len())
                            .ok_or_else(|| ParseError("CONCAT result is too large".into()))?;
                        if total > MAX_TEXT_BYTES {
                            return Err(ParseError("CONCAT result is too large".into()));
                        }
                        result.push_str(&value);
                    }
                    None => is_null = true,
                }
                count += 1;
                if !parser.take(Token::Comma) {
                    break;
                }
            }
            if count < 2 {
                return Err(ParseError("CONCAT requires at least two arguments".into()));
            }
            require(parser, Token::RParen, "expected ')' after CONCAT arguments")?;
            if is_null {
                ConstantValue::Null
            } else {
                ConstantValue::Text(result)
            }
        }
        "DATETRUNC" => {
            let unit = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "dateTrunc requires a timestamp")?;
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "dateTrunc requires two arguments")?;
            match (unit, input) {
                (Some(unit), Some(input)) => {
                    let step: u32 = match unit.to_ascii_lowercase().as_str() {
                        "second" | "seconds" => 1,
                        "minute" | "minutes" => 60,
                        "hour" | "hours" => 3_600,
                        "day" | "days" => 86_400,
                        _ => return Err(ParseError("unsupported dateTrunc unit".into())),
                    };
                    let seconds = rusthouse_storage::parse_datetime(&input)
                        .ok_or_else(|| ParseError("invalid UTC DateTime argument".into()))?;
                    ConstantValue::Text(rusthouse_storage::format_datetime(seconds / step * step))
                }
                _ => ConstantValue::Null,
            }
        }
        "QUERYSTRING" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "queryString requires one argument")?;
            match input {
                Some(input) => {
                    let before_fragment = input
                        .split_once('#')
                        .map_or(input.as_str(), |(head, _)| head);
                    let query = before_fragment
                        .split_once('?')
                        .map_or("", |(_, query)| query);
                    ConstantValue::Text(query.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "DATEADD" | "DATESUB" => {
            let unit = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "dateAdd requires an offset")?;
            let amount = integer_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "dateAdd requires a timestamp")?;
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "dateAdd requires three arguments")?;
            match (unit, amount, input) {
                (Some(unit), Some(amount), Some(input)) => {
                    let step: i64 = match unit.to_ascii_lowercase().as_str() {
                        "second" | "seconds" => 1,
                        "minute" | "minutes" => 60,
                        "hour" | "hours" => 3_600,
                        "day" | "days" => 86_400,
                        "week" | "weeks" => 604_800,
                        _ => return Err(ParseError("unsupported dateAdd unit".into())),
                    };
                    let seconds = rusthouse_storage::parse_datetime(&input)
                        .ok_or_else(|| ParseError("invalid UTC DateTime argument".into()))?;
                    let offset = amount
                        .checked_mul(step)
                        .ok_or_else(|| ParseError("dateAdd offset overflow".into()))?;
                    let shifted = if name == "DATESUB" {
                        i64::from(seconds).checked_sub(offset)
                    } else {
                        i64::from(seconds).checked_add(offset)
                    }
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| ParseError("dateAdd result outside UTC DateTime range".into()))?;
                    ConstantValue::Text(rusthouse_storage::format_datetime(shifted))
                }
                _ => ConstantValue::Null,
            }
        }
        "SUBSTRING_INDEX" | "SUBSTRINGINDEXUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "SUBSTRING_INDEX requires a delimiter")?;
            let delimiter = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "SUBSTRING_INDEX requires a field count",
            )?;
            let count = integer_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after SUBSTRING_INDEX arguments",
            )?;
            match (input, delimiter, count) {
                (Some(input), Some(delimiter), Some(count)) => {
                    if delimiter.is_empty() || count == 0 {
                        ConstantValue::Text(String::new())
                    } else {
                        let field = usize::try_from(count.unsigned_abs())
                            .unwrap_or(usize::MAX)
                            .saturating_sub(1);
                        let result = if count > 0 {
                            input
                                .match_indices(delimiter.as_str())
                                .nth(field)
                                .map(|(index, _)| &input[..index])
                                .unwrap_or(&input)
                        } else {
                            input
                                .rmatch_indices(delimiter.as_str())
                                .nth(field)
                                .map(|(index, _)| &input[index + delimiter.len()..])
                                .unwrap_or(&input)
                        };
                        ConstantValue::Text(result.to_owned())
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "ADDMONTHS" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "addMonths requires a month count")?;
            let amount = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "addMonths requires two arguments")?;
            match (input, amount) {
                (Some(value), Some(amount)) => {
                    let (calendar, clock) = if let Some(days) = rusthouse_storage::parse_date(&value) {
                        (rusthouse_storage::format_date(days), None)
                    } else if let Some(seconds) = rusthouse_storage::parse_datetime(&value) {
                        let datetime = rusthouse_storage::format_datetime(seconds);
                        (datetime[..10].to_owned(), Some(datetime[10..].to_owned()))
                    } else {
                        return Err(ParseError("invalid UTC Date or DateTime argument".into()));
                    };
                    let year = calendar[..4]
                        .parse::<i64>()
                        .map_err(|_| ParseError("invalid calendar year".into()))?;
                    let month = calendar[5..7]
                        .parse::<i64>()
                        .map_err(|_| ParseError("invalid calendar month".into()))?;
                    let day = calendar[8..10]
                        .parse::<u32>()
                        .map_err(|_| ParseError("invalid calendar day".into()))?;
                    let offset = amount;
                    let target = (year * 12 + month - 1)
                        .checked_add(offset)
                        .ok_or_else(|| ParseError("addMonths offset overflow".into()))?;
                    let target_year = target.div_euclid(12);
                    let target_month = target.rem_euclid(12) + 1;
                    if !(1970..=2149).contains(&target_year) {
                        return Err(ParseError("addMonths result outside Date range".into()));
                    }
                    let mut candidate = None;
                    for target_day in (1..=day).rev() {
                        let date = format!("{target_year:04}-{target_month:02}-{target_day:02}");
                        if let Some(days) = rusthouse_storage::parse_date(&date) {
                            candidate = Some((date, days));
                            break;
                        }
                    }
                    let (date, days) = candidate
                        .ok_or_else(|| ParseError("addMonths result outside Date range".into()))?;
                    if let Some(clock) = clock {
                        let datetime = format!("{date}{clock}");
                        let seconds = rusthouse_storage::parse_datetime(&datetime)
                            .ok_or_else(|| ParseError("addMonths result outside DateTime range".into()))?;
                        ConstantValue::Text(rusthouse_storage::format_datetime(seconds))
                    } else {
                        ConstantValue::Text(rusthouse_storage::format_date(days))
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "QUERYSTRINGANDFRAGMENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "queryStringAndFragment requires one argument",
            )?;
            match input {
                Some(input) => {
                    let before_fragment = input
                        .split_once('#')
                        .map_or(input.as_str(), |(head, _)| head);
                    let query = before_fragment
                        .find('?')
                        .map_or("", |position| &input[position + 1..]);
                    ConstantValue::Text(query.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "SUBTRACTMONTHS" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "subtractMonths requires a month count")?;
            let amount = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "subtractMonths requires two arguments")?;
            match (input, amount) {
                (Some(value), Some(amount)) => {
                    let (calendar, clock) = if let Some(days) = rusthouse_storage::parse_date(&value) {
                        (rusthouse_storage::format_date(days), None)
                    } else if let Some(seconds) = rusthouse_storage::parse_datetime(&value) {
                        let datetime = rusthouse_storage::format_datetime(seconds);
                        (datetime[..10].to_owned(), Some(datetime[10..].to_owned()))
                    } else {
                        return Err(ParseError("invalid UTC Date or DateTime argument".into()));
                    };
                    let year = calendar[..4]
                        .parse::<i64>()
                        .map_err(|_| ParseError("invalid calendar year".into()))?;
                    let month = calendar[5..7]
                        .parse::<i64>()
                        .map_err(|_| ParseError("invalid calendar month".into()))?;
                    let day = calendar[8..10]
                        .parse::<u32>()
                        .map_err(|_| ParseError("invalid calendar day".into()))?;
                    let offset = amount.checked_neg()
                        .ok_or_else(|| ParseError("subtractMonths offset overflow".into()))?;
                    let target = (year * 12 + month - 1)
                        .checked_add(offset)
                        .ok_or_else(|| ParseError("subtractMonths offset overflow".into()))?;
                    let target_year = target.div_euclid(12);
                    let target_month = target.rem_euclid(12) + 1;
                    if !(1970..=2149).contains(&target_year) {
                        return Err(ParseError("subtractMonths result outside Date range".into()));
                    }
                    let mut candidate = None;
                    for target_day in (1..=day).rev() {
                        let date = format!("{target_year:04}-{target_month:02}-{target_day:02}");
                        if let Some(days) = rusthouse_storage::parse_date(&date) {
                            candidate = Some((date, days));
                            break;
                        }
                    }
                    let (date, days) = candidate
                        .ok_or_else(|| ParseError("subtractMonths result outside Date range".into()))?;
                    if let Some(clock) = clock {
                        let datetime = format!("{date}{clock}");
                        let seconds = rusthouse_storage::parse_datetime(&datetime)
                            .ok_or_else(|| ParseError("subtractMonths result outside DateTime range".into()))?;
                        ConstantValue::Text(rusthouse_storage::format_datetime(seconds))
                    } else {
                        ConstantValue::Text(rusthouse_storage::format_date(days))
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "SPLIT_PART" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "SPLIT_PART requires a delimiter")?;
            let delimiter = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "SPLIT_PART requires a field index")?;
            let index = integer_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after SPLIT_PART arguments",
            )?;
            match (input, delimiter, index) {
                (Some(input), Some(delimiter), Some(index)) => {
                    if delimiter.is_empty() {
                        return Err(ParseError("SPLIT_PART delimiter cannot be empty".into()));
                    }
                    if index < 1 {
                        return Err(ParseError("SPLIT_PART index must be positive".into()));
                    }
                    let index = usize::try_from(index - 1).unwrap_or(usize::MAX);
                    ConstantValue::Text(
                        input
                            .split(delimiter.as_str())
                            .nth(index)
                            .unwrap_or("")
                            .to_owned(),
                    )
                }
                _ => ConstantValue::Null,
            }
        }
        "HASSUBSEQUENCECASEINSENSITIVE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "hasSubsequenceCaseInsensitive requires a needle",
            )?;
            let needle = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after hasSubsequenceCaseInsensitive",
            )?;
            match (input, needle) {
                (Some(input), Some(needle)) => {
                    let mut required = needle.bytes().map(|byte| byte.to_ascii_lowercase());
                    let mut next = required.next();
                    for byte in input.bytes() {
                        if next == Some(byte.to_ascii_lowercase()) {
                            next = required.next();
                        }
                    }
                    ConstantValue::Integer(i64::from(next.is_none()))
                }
                _ => ConstantValue::Null,
            }
        }
        "OVERLAY" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "OVERLAY requires replacement text")?;
            let replacement = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "OVERLAY requires a start position")?;
            let start = integer_arg(parser, depth + 1)?;
            let length = if parser.take(Token::Comma) {
                Some(integer_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(
                parser,
                Token::RParen,
                "expected ')' after OVERLAY arguments",
            )?;
            match (input, replacement, start) {
                (Some(input), Some(replacement), Some(start)) => {
                    if start < 1 {
                        return Err(ParseError("OVERLAY positions are 1-based".into()));
                    }
                    let remove = match length {
                        None => replacement.chars().count(),
                        Some(Some(length)) if length >= 0 => {
                            usize::try_from(length).unwrap_or(usize::MAX)
                        }
                        Some(Some(_)) => {
                            return Err(ParseError("OVERLAY length must be nonnegative".into()));
                        }
                        Some(None) => return Ok(ConstantValue::Null),
                    };
                    let start = usize::try_from(start - 1).unwrap_or(usize::MAX);
                    let prefix = input
                        .char_indices()
                        .nth(start)
                        .map_or(input.len(), |(index, _)| index);
                    let suffix = input
                        .char_indices()
                        .nth(start.saturating_add(remove))
                        .map_or(input.len(), |(index, _)| index);
                    let size = prefix
                        .checked_add(replacement.len())
                        .and_then(|size| size.checked_add(input.len() - suffix))
                        .ok_or_else(|| ParseError("OVERLAY result is too large".into()))?;
                    if size > MAX_TEXT_BYTES {
                        return Err(ParseError("OVERLAY result is too large".into()));
                    }
                    let mut result = String::with_capacity(size);
                    result.push_str(&input[..prefix]);
                    result.push_str(&replacement);
                    result.push_str(&input[suffix..]);
                    ConstantValue::Text(result)
                }
                _ => ConstantValue::Null,
            }
        }
        "FRAGMENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "fragment requires one argument")?;
            match input {
                Some(input) => ConstantValue::Text(
                    input
                        .split_once('#')
                        .map_or("", |(_, fragment)| fragment)
                        .to_owned(),
                ),
                None => ConstantValue::Null,
            }
        }
        "SUBSTRING" | "SUBSTRINGUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "SUBSTRING requires a start position")?;
            let position = integer_arg(parser, depth + 1)?;
            let length = if parser.take(Token::Comma) {
                Some(integer_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(
                parser,
                Token::RParen,
                "expected ')' after SUBSTRING arguments",
            )?;
            match (input, position, length) {
                (Some(value), Some(position), length) => {
                    if position < 1 {
                        return Err(ParseError("SUBSTRING positions are 1-based".into()));
                    }
                    let start = usize::try_from(position - 1).unwrap_or(usize::MAX);
                    let count = match length {
                        None => usize::MAX,
                        Some(Some(count)) if count >= 0 => {
                            usize::try_from(count).unwrap_or(usize::MAX)
                        }
                        Some(Some(_)) => {
                            return Err(ParseError("SUBSTRING length must be nonnegative".into()));
                        }
                        Some(None) => return Ok(ConstantValue::Null),
                    };
                    ConstantValue::Text(value.chars().skip(start).take(count).collect())
                }
                _ => ConstantValue::Null,
            }
        }
        "UNBIN" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "unbin requires one argument")?;
            match input {
                Some(input) => {
                    let bits = input.as_bytes();
                    if bits.len() % 8 != 0 {
                        return Err(ParseError("unbin requires complete byte groups".into()));
                    }
                    let mut decoded = Vec::with_capacity(bits.len() / 8);
                    for group in bits.chunks_exact(8) {
                        let mut value = 0u8;
                        for &bit in group {
                            value = (value << 1) | match bit {
                                b'0' => 0,
                                b'1' => 1,
                                _ => return Err(ParseError("unbin requires binary digits".into())),
                            };
                        }
                        decoded.push(value);
                    }
                    ConstantValue::Text(String::from_utf8(decoded)
                        .map_err(|_| ParseError("unbin result is not UTF-8".into()))?)
                }
                None => ConstantValue::Null,
            }
        }
        "HEX_DECODE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "HEX_DECODE requires one argument")?;
            match input {
                Some(value) => {
                    if value.len() % 2 != 0 || !value.is_ascii() {
                        return Err(ParseError(
                            "HEX_DECODE requires even-length ASCII hex".into(),
                        ));
                    }
                    let mut bytes = Vec::with_capacity(value.len() / 2);
                    for pair in value.as_bytes().chunks_exact(2) {
                        let hex = std::str::from_utf8(pair)
                            .map_err(|_| ParseError("invalid HEX_DECODE input".into()))?;
                        let byte = u8::from_str_radix(hex, 16)
                            .map_err(|_| ParseError("invalid HEX_DECODE input".into()))?;
                        bytes.push(byte);
                    }
                    let decoded = String::from_utf8(bytes)
                        .map_err(|_| ParseError("HEX_DECODE result is not UTF-8".into()))?;
                    ConstantValue::Text(decoded)
                }
                None => ConstantValue::Null,
            }
        }
        "TOSTRING" => {
            let value = match parser.tokens.get(parser.index) {
                Some(Token::String(_)) => text_arg(parser, depth + 1)?,
                Some(Token::Word(name)) if is_function(name) => {
                    match parse_at_depth(parser, depth + 1)? {
                        ConstantValue::Text(value) => Some(value),
                        ConstantValue::Integer(value) => Some(value.to_string()),
                        ConstantValue::Null => None,
                    }
                }
                _ => integer_arg(parser, depth + 1)?.map(|value| value.to_string()),
            };
            require(parser, Token::RParen, "toString requires one argument")?;
            value.map_or(ConstantValue::Null, ConstantValue::Text)
        }
        "TO_INT64" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "TO_INT64 requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(
                    value
                        .parse::<i64>()
                        .map_err(|_| ParseError("TO_INT64 requires an Int64 string".into()))?,
                ),
                None => ConstantValue::Null,
            }
        }
        "LPAD" | "LEFTPADUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "LPAD requires a width")?;
            let width = integer_arg(parser, depth + 1)?;
            let padding = if parser.take(Token::Comma) {
                text_arg(parser, depth + 1)?
            } else {
                Some(" ".to_owned())
            };
            require(parser, Token::RParen, "expected ')' after LPAD arguments")?;
            match (input, width, padding) {
                (Some(input), Some(width), Some(padding)) => {
                    let width = usize::try_from(width)
                        .map_err(|_| ParseError("LPAD width must be nonnegative".into()))?;
                    if width > MAX_TEXT_BYTES {
                        return Err(ParseError("LPAD result is too large".into()));
                    }
                    let tail: String = input.chars().take(width).collect();
                    if tail.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("LPAD result is too large".into()));
                    }
                    let missing = width - tail.chars().count();
                    if missing > 0 && padding.is_empty() {
                        return Err(ParseError("LPAD padding cannot be empty".into()));
                    }
                    let mut result = String::new();
                    for character in padding.chars().cycle().take(missing) {
                        if result.len() + character.len_utf8() + tail.len() > MAX_TEXT_BYTES {
                            return Err(ParseError("LPAD result is too large".into()));
                        }
                        result.push(character);
                    }
                    result.push_str(&tail);
                    ConstantValue::Text(result)
                }
                _ => ConstantValue::Null,
            }
        }
        "FROMUNIXTIMESTAMP64NANO" => {
            let input = integer_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "fromUnixTimestamp64Nano requires one argument",
            )?;
            match input {
                Some(input) => {
                    let unit = 10_i64.pow(9);
                    let seconds = u32::try_from(input.div_euclid(unit)).map_err(|_| {
                        ParseError("fromUnixTimestamp64Nano requires a supported UTC DateTime".into())
                    })?;
                    let fraction = input.rem_euclid(unit);
                    let datetime = rusthouse_storage::format_datetime(seconds);
                    ConstantValue::Text(format!(
                        "{datetime}.{fraction:0width$}",
                        width = 9
                    ))
                }
                None => ConstantValue::Null,
            }
        }
        "PROTOCOL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "protocol requires one argument")?;
            match input {
                Some(input) => {
                    let scheme = input.split_once(':').map_or("", |(scheme, _)| scheme);
                    let valid = scheme
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphabetic)
                        && scheme.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
                        });
                    ConstantValue::Text(if valid { scheme } else { "" }.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "LTRIM" | "TRIMLEFT" => {
            let input = text_arg(parser, depth + 1)?;
            let characters = if parser.take(Token::Comma) {
                Some(text_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(parser, Token::RParen, "expected ')' after LTRIM arguments")?;
            match (input, characters) {
                (Some(input), None) => {
                    ConstantValue::Text(input.trim_start_matches(' ').to_owned())
                }
                (Some(input), Some(Some(characters))) => ConstantValue::Text(
                    input
                        .trim_start_matches(|ch| characters.contains(ch))
                        .to_owned(),
                ),
                _ => ConstantValue::Null,
            }
        }
        "FROMUNIXTIMESTAMP64MICRO" => {
            let input = integer_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "fromUnixTimestamp64Micro requires one argument",
            )?;
            match input {
                Some(input) => {
                    let unit = 10_i64.pow(6);
                    let seconds = u32::try_from(input.div_euclid(unit)).map_err(|_| {
                        ParseError("fromUnixTimestamp64Micro requires a supported UTC DateTime".into())
                    })?;
                    let fraction = input.rem_euclid(unit);
                    let datetime = rusthouse_storage::format_datetime(seconds);
                    ConstantValue::Text(format!(
                        "{datetime}.{fraction:0width$}",
                        width = 6
                    ))
                }
                None => ConstantValue::Null,
            }
        }
        "BASENAME" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "basename requires one argument")?;
            match input {
                Some(input) => {
                    ConstantValue::Text(input.rsplit(['/', '\\']).next().unwrap_or("").to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "TRIM_WHITESPACE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "TRIM_WHITESPACE requires one argument",
            )?;
            match input {
                Some(value) => ConstantValue::Text(value.trim().to_owned()),
                None => ConstantValue::Null,
            }
        }
        "TRIM" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "TRIM requires one argument")?;
            match input {
                Some(value) => ConstantValue::Text(value.trim_matches(' ').to_owned()),
                None => ConstantValue::Null,
            }
        }
        "COUNTSUBSTRINGSCASEINSENSITIVE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "countSubstringsCaseInsensitive requires a needle",
            )?;
            let needle = text_arg(parser, depth + 1)?;
            let start = if parser.take(Token::Comma) {
                integer_arg(parser, depth + 1)?
            } else {
                Some(1)
            };
            require(
                parser,
                Token::RParen,
                "expected ')' after countSubstringsCaseInsensitive",
            )?;
            match (input, needle, start) {
                (Some(input), Some(needle), Some(start)) => {
                    if start < 0 {
                        return Err(ParseError(
                            "countSubstringsCaseInsensitive start must be nonnegative".into(),
                        ));
                    }
                    let offset = usize::try_from(start.max(1) - 1).unwrap_or(usize::MAX);
                    let bytes = input.as_bytes().get(offset..).unwrap_or_default();
                    let needle = needle.as_bytes();
                    let mut count = 0_i64;
                    if !needle.is_empty() && needle.len() <= bytes.len() {
                        let mut cursor = 0;
                        while cursor <= bytes.len() - needle.len() {
                            if bytes[cursor..cursor + needle.len()].eq_ignore_ascii_case(needle) {
                                count += 1;
                                cursor += needle.len();
                            } else {
                                cursor += 1;
                            }
                        }
                    }
                    ConstantValue::Integer(count)
                }
                _ => ConstantValue::Null,
            }
        }
        "COUNT_SUBSTRING" | "COUNTSUBSTRINGS" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "COUNT_SUBSTRING requires a substring")?;
            let needle = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after COUNT_SUBSTRING arguments",
            )?;
            match (input, needle) {
                (Some(input), Some(needle)) => {
                    if needle.is_empty() {
                        return Err(ParseError("COUNT_SUBSTRING needle cannot be empty".into()));
                    }
                    ConstantValue::Integer(
                        i64::try_from(input.matches(&needle).count()).map_err(|_| {
                            ParseError("COUNT_SUBSTRING count exceeds Int64".into())
                        })?,
                    )
                }
                _ => ConstantValue::Null,
            }
        }
        "PATH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "path requires one argument")?;
            match input {
                Some(input) => {
                    let suffix = if let Some((_, after_scheme)) = input.split_once("://") {
                        let start = after_scheme
                            .find(['/', '?', '#'])
                            .unwrap_or(after_scheme.len());
                        &after_scheme[start..]
                    } else {
                        input.as_str()
                    };
                    let end = suffix.find(['?', '#']).unwrap_or(suffix.len());
                    let path = &suffix[..end];
                    ConstantValue::Text(if path.starts_with('/') { path } else { "" }.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "IFNULL" => {
            let left = constant_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "ifNull requires a fallback")?;
            let fallback = constant_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after ifNull")?;
            if left == ConstantValue::Null { fallback } else { left }
        }
        "NULLIF" => {
            let left = constant_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "NULLIF requires a second argument")?;
            let right = constant_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after NULLIF arguments")?;
            if left == right { ConstantValue::Null } else { left }
        }
        "POSITIONCASEINSENSITIVE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "positionCaseInsensitive requires a needle",
            )?;
            let needle = text_arg(parser, depth + 1)?;
            let start = if parser.take(Token::Comma) {
                integer_arg(parser, depth + 1)?
            } else {
                Some(1)
            };
            require(
                parser,
                Token::RParen,
                "expected ')' after positionCaseInsensitive",
            )?;
            match (input, needle, start) {
                (Some(input), Some(needle), Some(start)) => {
                    if start < 0 {
                        return Err(ParseError(
                            "positionCaseInsensitive start must be nonnegative".into(),
                        ));
                    }
                    let offset = usize::try_from(start.max(1) - 1).unwrap_or(usize::MAX);
                    let position = if offset > input.len() {
                        0
                    } else if needle.is_empty() {
                        offset + 1
                    } else {
                        input.as_bytes()[offset..]
                            .windows(needle.len())
                            .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
                            .map_or(0, |index| offset + index + 1)
                    };
                    ConstantValue::Integer(i64::try_from(position).map_err(|_| {
                        ParseError("positionCaseInsensitive result exceeds Int64".into())
                    })?)
                }
                _ => ConstantValue::Null,
            }
        }
        "TOSTARTOFISOYEAR" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "toStartOfISOYear requires one argument")?;
            match input {
                Some(value) => {
                    let days = rusthouse_storage::parse_date(&value).map(u32::from)
                        .or_else(|| rusthouse_storage::parse_datetime(&value).map(|seconds| seconds / 86_400))
                        .ok_or_else(|| ParseError("toStartOfISOYear requires a valid UTC Date or DateTime".into()))?;
                    let weekday = (days + 3) % 7;
                    let thursday = i64::from(days) + 3 - i64::from(weekday);
                    let thursday = u16::try_from(thursday).map_err(|_| {
                        ParseError("toStartOfISOYear is outside the Date range".into())
                    })?;
                    let calendar_day = rusthouse_storage::format_date(thursday);
                    let year = &calendar_day[..4];
                    let jan4 = rusthouse_storage::parse_date(&format!("{year}-01-04"))
                        .ok_or_else(|| ParseError("invalid ISO calendar year".into()))?;
                    let iso_start = i64::from(jan4) - i64::from((u32::from(jan4) + 3) % 7);
                    let iso_start = u16::try_from(iso_start).map_err(|_| {
                        ParseError("toStartOfISOYear result is outside the Date range".into())
                    })?;
                    ConstantValue::Text(rusthouse_storage::format_date(iso_start))
                }
                None => ConstantValue::Null,
            }
        }
        "STRPOS" | "POSITIONUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "STRPOS requires a substring")?;
            let needle = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after STRPOS arguments")?;
            match (input, needle) {
                (Some(input), Some(needle)) => {
                    let index = input
                        .find(needle.as_str())
                        .map(|byte_index| input[..byte_index].chars().count() + 1)
                        .unwrap_or(0);
                    ConstantValue::Integer(
                        i64::try_from(index)
                            .map_err(|_| ParseError("STRPOS result exceeds Int64".into()))?,
                    )
                }
                _ => ConstantValue::Null,
            }
        }
        "RTRIM" => {
            let input = text_arg(parser, depth + 1)?;
            let characters = if parser.take(Token::Comma) {
                Some(text_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(parser, Token::RParen, "expected ')' after RTRIM arguments")?;
            match (input, characters) {
                (Some(input), None) => ConstantValue::Text(input.trim_end_matches(' ').to_owned()),
                (Some(input), Some(Some(characters))) => ConstantValue::Text(
                    input
                        .trim_end_matches(|ch| characters.contains(ch))
                        .to_owned(),
                ),
                _ => ConstantValue::Null,
            }
        }
        "LAST_INDEX_OF" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "LAST_INDEX_OF requires a substring")?;
            let needle = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after LAST_INDEX_OF arguments",
            )?;
            match (input, needle) {
                (Some(input), Some(needle)) => {
                    let index = input
                        .rfind(needle.as_str())
                        .map(|byte_index| input[..byte_index].chars().count() + 1)
                        .unwrap_or(0);
                    ConstantValue::Integer(
                        i64::try_from(index)
                            .map_err(|_| ParseError("LAST_INDEX_OF exceeds Int64".into()))?,
                    )
                }
                _ => ConstantValue::Null,
            }
        }
        "COLLAPSE_WHITESPACE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "COLLAPSE_WHITESPACE requires one argument",
            )?;
            match input {
                Some(value) => {
                    let mut result = String::new();
                    let mut pending_space = false;
                    for character in value.chars() {
                        if character.is_whitespace() {
                            pending_space = !result.is_empty();
                        } else {
                            if pending_space {
                                result.push(' ');
                                pending_space = false;
                            }
                            result.push(character);
                        }
                        if result.len() > MAX_TEXT_BYTES {
                            return Err(ParseError(
                                "COLLAPSE_WHITESPACE result is too large".into(),
                            ));
                        }
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "BTRIM" | "TRIMBOTH" => {
            let input = text_arg(parser, depth + 1)?;
            let characters = if parser.take(Token::Comma) {
                Some(text_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(parser, Token::RParen, "expected ')' after BTRIM arguments")?;
            match (input, characters) {
                (Some(input), None) => ConstantValue::Text(input.trim_matches(' ').to_owned()),
                (Some(input), Some(Some(characters))) => ConstantValue::Text(
                    input
                        .trim_matches(|character| characters.contains(character))
                        .to_owned(),
                ),
                _ => ConstantValue::Null,
            }
        }
        "TRANSLATE" | "TRANSLATEUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "TRANSLATE requires source characters")?;
            let from = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "TRANSLATE requires replacement characters",
            )?;
            let to = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after TRANSLATE arguments",
            )?;
            match (input, from, to) {
                (Some(input), Some(from), Some(to)) => {
                    if name == "TRANSLATEUTF8" && from.chars().count() != to.chars().count() {
                        return Err(ParseError("translateUTF8 requires matching character counts".into()));
                    }
                    let mut replacements = std::collections::HashMap::new();
                    let mut to_chars = to.chars();
                    for ch in from.chars() {
                        let replacement = to_chars.next();
                        replacements.entry(ch).or_insert(replacement);
                    }
                    let mut result = String::with_capacity(input.len());
                    for ch in input.chars() {
                        if let Some(replacement) = replacements.get(&ch) {
                            if let Some(mapped) = replacement {
                                if result.len() + mapped.len_utf8() > MAX_TEXT_BYTES {
                                    return Err(ParseError("TRANSLATE result is too large".into()));
                                }
                                result.push(*mapped);
                            }
                        } else {
                            if result.len() + ch.len_utf8() > MAX_TEXT_BYTES {
                                return Err(ParseError("TRANSLATE result is too large".into()));
                            }
                            result.push(ch);
                        }
                    }
                    ConstantValue::Text(result)
                }
                _ => ConstantValue::Null,
            }
        }
        "PATHFULL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "pathFull requires one argument")?;
            match input {
                Some(input) => {
                    let suffix = if let Some((_, after_scheme)) = input.split_once("://") {
                        let start = after_scheme
                            .find(['/', '?', '#'])
                            .unwrap_or(after_scheme.len());
                        &after_scheme[start..]
                    } else {
                        input.as_str()
                    };
                    ConstantValue::Text(suffix.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "REPLACE" | "REPLACEALL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "REPLACE requires a search string")?;
            let needle = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "REPLACE requires a replacement string",
            )?;
            let replacement = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after REPLACE arguments",
            )?;
            match (input, needle, replacement) {
                (Some(input), Some(needle), Some(replacement)) => {
                    if needle.is_empty() {
                        ConstantValue::Text(input)
                    } else {
                        let matches = input.match_indices(&needle).count();
                        let removed = matches * needle.len();
                        let inserted = matches
                            .checked_mul(replacement.len())
                            .ok_or_else(|| ParseError("REPLACE result is too large".into()))?;
                        let length = (input.len() - removed)
                            .checked_add(inserted)
                            .ok_or_else(|| ParseError("REPLACE result is too large".into()))?;
                        if length > MAX_TEXT_BYTES {
                            return Err(ParseError("REPLACE result is too large".into()));
                        }
                        ConstantValue::Text(input.replace(&needle, &replacement))
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "TOMONTHNUMSINCEEPOCH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "toMonthNumSinceEpoch requires one argument")?;
            match input {
                Some(value) => {
                    let days = rusthouse_storage::parse_date(&value).map(u32::from)
                        .or_else(|| rusthouse_storage::parse_datetime(&value).map(|seconds| seconds / 86_400))
                        .ok_or_else(|| ParseError("toMonthNumSinceEpoch requires a valid UTC Date or DateTime".into()))?;
                    let date = rusthouse_storage::format_date(u16::try_from(days).map_err(|_| {
                        ParseError("toMonthNumSinceEpoch date is out of range".into())
                    })?);
                    let year = date[..4].parse::<i64>().map_err(|_| {
                        ParseError("invalid calendar year".into())
                    })?;
                    let month = date[5..7].parse::<i64>().map_err(|_| {
                        ParseError("invalid calendar month".into())
                    })?;
                    ConstantValue::Integer((year - 1970) * 12 + month - 1)
                }
                None => ConstantValue::Null,
            }
        }
        "HASTOKEN" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "hasToken requires a token")?;
            let token = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after hasToken")?;
            match (input, token) {
                (Some(input), Some(token)) => {
                    if token.is_empty()
                        || !token
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    {
                        return Err(ParseError("hasToken requires a single ASCII token".into()));
                    }
                    let found = input
                        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                        .any(|word| word == token);
                    ConstantValue::Integer(i64::from(found))
                }
                _ => ConstantValue::Null,
            }
        }
        "CONTAINS" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "CONTAINS requires a substring")?;
            let needle = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after CONTAINS arguments",
            )?;
            match (input, needle) {
                (Some(input), Some(needle)) => {
                    ConstantValue::Integer(i64::from(input.contains(needle.as_str())))
                }
                _ => ConstantValue::Null,
            }
        }
        "STARTS_WITH" | "STARTSWITH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "STARTS_WITH requires a prefix")?;
            let prefix = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after STARTS_WITH arguments",
            )?;
            match (input, prefix) {
                (Some(input), Some(prefix)) => {
                    ConstantValue::Integer(i64::from(input.starts_with(prefix.as_str())))
                }
                _ => ConstantValue::Null,
            }
        }
        "REPLACE_FIRST" | "REPLACEONE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "REPLACE_FIRST requires a search string",
            )?;
            let needle = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "REPLACE_FIRST requires a replacement")?;
            let replacement = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after REPLACE_FIRST arguments",
            )?;
            match (input, needle, replacement) {
                (Some(input), Some(needle), Some(replacement)) => {
                    if needle.is_empty() || !input.contains(&needle) {
                        ConstantValue::Text(input)
                    } else {
                        let length = input.len() - needle.len();
                        if length.saturating_add(replacement.len()) > MAX_TEXT_BYTES {
                            return Err(ParseError("REPLACE_FIRST result is too large".into()));
                        }
                        ConstantValue::Text(input.replacen(&needle, &replacement, 1))
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "STRIP_PREFIX" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "STRIP_PREFIX requires a prefix")?;
            let prefix = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after STRIP_PREFIX arguments",
            )?;
            match (input, prefix) {
                (Some(input), Some(prefix)) => ConstantValue::Text(
                    input
                        .strip_prefix(prefix.as_str())
                        .unwrap_or(input.as_str())
                        .to_owned(),
                ),
                _ => ConstantValue::Null,
            }
        }
        "LEFT" | "LEFTUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "LEFT requires a character count")?;
            let count = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after LEFT arguments")?;
            match (input, count) {
                (Some(value), Some(count)) => {
                    if count < 0 {
                        return Err(ParseError("LEFT count must be nonnegative".into()));
                    }
                    let count = usize::try_from(count).unwrap_or(usize::MAX);
                    if count >= value.len() {
                        ConstantValue::Text(value)
                    } else if value.as_bytes()[..count].is_ascii() {
                        let mut result = value;
                        result.truncate(count);
                        ConstantValue::Text(result)
                    } else {
                        ConstantValue::Text(value.chars().take(count).collect())
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "REPLACE_LAST" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "REPLACE_LAST requires a search string",
            )?;
            let needle = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "REPLACE_LAST requires a replacement")?;
            let replacement = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after REPLACE_LAST arguments",
            )?;
            match (input, needle, replacement) {
                (Some(input), Some(needle), Some(replacement)) => {
                    if needle.is_empty() {
                        ConstantValue::Text(input)
                    } else if let Some(index) = input.rfind(&needle) {
                        let length = (input.len() - needle.len())
                            .checked_add(replacement.len())
                            .ok_or_else(|| ParseError("REPLACE_LAST result is too large".into()))?;
                        if length > MAX_TEXT_BYTES {
                            return Err(ParseError("REPLACE_LAST result is too large".into()));
                        }
                        let mut result = String::with_capacity(length);
                        result.push_str(&input[..index]);
                        result.push_str(&replacement);
                        result.push_str(&input[index + needle.len()..]);
                        ConstantValue::Text(result)
                    } else {
                        ConstantValue::Text(input)
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "HASTOKENCASEINSENSITIVE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "hasTokenCaseInsensitive requires a token",
            )?;
            let token = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after hasTokenCaseInsensitive",
            )?;
            match (input, token) {
                (Some(input), Some(token)) => {
                    if token.is_empty()
                        || !token
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    {
                        return Err(ParseError(
                            "hasTokenCaseInsensitive requires a single ASCII token".into(),
                        ));
                    }
                    let found = input
                        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                        .any(|word| word.eq_ignore_ascii_case(&token));
                    ConstantValue::Integer(i64::from(found))
                }
                _ => ConstantValue::Null,
            }
        }
        "ENDS_WITH" | "ENDSWITH" | "ENDSWITHUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "ENDS_WITH requires a suffix")?;
            let suffix = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after ENDS_WITH arguments",
            )?;
            match (input, suffix) {
                (Some(input), Some(suffix)) => {
                    ConstantValue::Integer(i64::from(input.ends_with(suffix.as_str())))
                }
                _ => ConstantValue::Null,
            }
        }
        "URL_DECODE" | "DECODEURLCOMPONENT" | "DECODEURLFORMCOMPONENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "URL_DECODE requires one argument")?;
            match input {
                Some(input) => {
                    let source = input.as_bytes();
                    let mut result = Vec::with_capacity(source.len());
                    let mut index = 0;
                    while index < source.len() {
                        if source[index] == b'%' {
                            if index + 2 >= source.len() {
                                return Err(ParseError("URL_DECODE has incomplete escape".into()));
                            }
                            let high = char::from(source[index + 1]).to_digit(16);
                            let low = char::from(source[index + 2]).to_digit(16);
                            let byte = match (high, low) {
                                (Some(high), Some(low)) => (high * 16 + low) as u8,
                                _ => {
                                    return Err(ParseError("URL_DECODE has invalid escape".into()));
                                }
                            };
                            result.push(byte);
                            index += 3;
                        } else {
                            result.push(if source[index] == b'+'
                                && name == "DECODEURLFORMCOMPONENT"
                            {
                                b' '
                            } else {
                                source[index]
                            });
                            index += 1;
                        }
                    }
                    ConstantValue::Text(
                        String::from_utf8(result)
                            .map_err(|_| ParseError("URL_DECODE result is not UTF-8".into()))?,
                    )
                }
                None => ConstantValue::Null,
            }
        }
        "BASE64_ENCODE" | "BASE64ENCODE" | "BASE64URLENCODE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "BASE64_ENCODE requires one argument")?;
            match input {
                Some(value) => {
                    let output_len = value
                        .len()
                        .checked_add(2)
                        .and_then(|length| length.checked_div(3))
                        .and_then(|groups| groups.checked_mul(4))
                        .ok_or_else(|| ParseError("BASE64_ENCODE result is too large".into()))?;
                    if output_len > MAX_TEXT_BYTES {
                        return Err(ParseError("BASE64_ENCODE result is too large".into()));
                    }
                    let alphabet: &[u8; 64] = if name == "BASE64URLENCODE" {
                        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
                    } else {
                        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
                    };
                    let mut result = String::with_capacity(output_len);
                    for chunk in value.as_bytes().chunks(3) {
                        let mut bits = u32::from(chunk[0]) << 16;
                        if chunk.len() > 1 {
                            bits |= u32::from(chunk[1]) << 8;
                        }
                        if chunk.len() > 2 {
                            bits |= u32::from(chunk[2]);
                        }
                        result.push(char::from(alphabet[((bits >> 18) & 63) as usize]));
                        result.push(char::from(alphabet[((bits >> 12) & 63) as usize]));
                        if chunk.len() > 1 {
                            result.push(char::from(alphabet[((bits >> 6) & 63) as usize]));
                        } else {
                            result.push('=');
                        }
                        if chunk.len() > 2 {
                            result.push(char::from(alphabet[(bits & 63) as usize]));
                        } else {
                            result.push('=');
                        }
                    }
                    if name == "BASE64URLENCODE" {
                        while result.ends_with('=') {
                            result.pop();
                        }
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "STRIP_SUFFIX" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "STRIP_SUFFIX requires a suffix")?;
            let suffix = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after STRIP_SUFFIX arguments",
            )?;
            match (input, suffix) {
                (Some(input), Some(suffix)) => ConstantValue::Text(
                    input
                        .strip_suffix(suffix.as_str())
                        .unwrap_or(input.as_str())
                        .to_owned(),
                ),
                _ => ConstantValue::Null,
            }
        }
        "DOMAIN" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "domain requires one argument")?;
            match input {
                Some(input) => {
                    let authority = input
                        .split_once("://")
                        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""))
                        .unwrap_or("");
                    let host_port = authority.rsplit('@').next().unwrap_or("");
                    let host = if let Some(without_bracket) = host_port.strip_prefix('[') {
                        without_bracket
                            .split_once(']')
                            .map(|(host, _)| host)
                            .unwrap_or("")
                    } else {
                        host_port.split(':').next().unwrap_or("")
                    };
                    ConstantValue::Text(host.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "NETLOC" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "netloc requires one argument")?;
            match input {
                Some(input) => {
                    let authority = input.split_once("://").map_or("", |(_, rest)| {
                        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
                        &rest[..end]
                    });
                    ConstantValue::Text(authority.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "RIGHT" | "RIGHTUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "RIGHT requires a character count")?;
            let count = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after RIGHT arguments")?;
            match (input, count) {
                (Some(value), Some(count)) => {
                    if count < 0 {
                        return Err(ParseError("RIGHT count must be nonnegative".into()));
                    }
                    let count = usize::try_from(count).unwrap_or(usize::MAX);
                    if count >= value.len() {
                        ConstantValue::Text(value)
                    } else {
                        let start = value.len() - count;
                        if value.as_bytes()[start..].is_ascii() {
                            let mut result = value;
                            result.replace_range(..start, "");
                            ConstantValue::Text(result)
                        } else {
                            let skip = value.chars().count().saturating_sub(count);
                            ConstantValue::Text(value.chars().skip(skip).collect())
                        }
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "SPACE" => {
            let count = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "SPACE requires one argument")?;
            match count {
                Some(count) => {
                    let width = usize::try_from(count)
                        .map_err(|_| ParseError("SPACE count must be nonnegative".into()))?;
                    if width > MAX_TEXT_BYTES {
                        return Err(ParseError("SPACE result is too large".into()));
                    }
                    ConstantValue::Text(" ".repeat(width))
                }
                None => ConstantValue::Null,
            }
        }
        "TOUINT8ORDEFAULT" | "TOUINT16ORDEFAULT" | "TOUINT32ORDEFAULT" => {
            let input = text_arg(parser, depth + 1)?;
            let fallback = if parser.take(Token::Comma) {
                integer_arg(parser, depth + 1)?
            } else {
                Some(0)
            };
            require(parser, Token::RParen, "unsigned OrDefault requires one or two arguments")?;
            match (input, fallback) {
                (Some(input), Some(fallback)) => {
                    let (parsed, fallback) = match name.as_str() {
                        "TOUINT8ORDEFAULT" => (
                            input.parse::<u128>().ok().map(|n| i64::from(n as u8)),
                            i64::from(u8::try_from(fallback)
                                .map_err(|_| ParseError("UInt8 default is out of range".into()))?),
                        ),
                        "TOUINT16ORDEFAULT" => (
                            input.parse::<u128>().ok().map(|n| i64::from(n as u16)),
                            i64::from(u16::try_from(fallback)
                                .map_err(|_| ParseError("UInt16 default is out of range".into()))?),
                        ),
                        "TOUINT32ORDEFAULT" => (
                            input.parse::<u128>().ok().map(|n| i64::from(n as u32)),
                            i64::from(u32::try_from(fallback)
                                .map_err(|_| ParseError("UInt32 default is out of range".into()))?),
                        ),
                        _ => unreachable!(),
                    };
                    ConstantValue::Integer(parsed.unwrap_or(fallback))
                }
                _ => ConstantValue::Null,
            }
        }
        "TOINT8ORDEFAULT" | "TOINT16ORDEFAULT" | "TOINT32ORDEFAULT" | "TOINT64ORDEFAULT" => {
            let input = text_arg(parser, depth + 1)?;
            let fallback = if parser.take(Token::Comma) {
                integer_arg(parser, depth + 1)?
            } else {
                Some(0)
            };
            require(parser, Token::RParen, "signed OrDefault requires one or two arguments")?;
            match (input, fallback) {
                (Some(input), Some(fallback)) => {
                    let (parsed, fallback) = match name.as_str() {
                        "TOINT8ORDEFAULT" => (
                            input.parse::<i128>().ok().map(|n| i64::from(n as i8)),
                            i64::from(i8::try_from(fallback)
                                .map_err(|_| ParseError("Int8 default is out of range".into()))?),
                        ),
                        "TOINT16ORDEFAULT" => (
                            input.parse::<i128>().ok().map(|n| i64::from(n as i16)),
                            i64::from(i16::try_from(fallback)
                                .map_err(|_| ParseError("Int16 default is out of range".into()))?),
                        ),
                        "TOINT32ORDEFAULT" => (
                            input.parse::<i128>().ok().map(|n| i64::from(n as i32)),
                            i64::from(i32::try_from(fallback)
                                .map_err(|_| ParseError("Int32 default is out of range".into()))?),
                        ),
                        "TOINT64ORDEFAULT" => (input.parse::<i64>().ok(), fallback),
                        _ => unreachable!(),
                    };
                    ConstantValue::Integer(parsed.unwrap_or(fallback))
                }
                _ => ConstantValue::Null,
            }
        }
        "PARSEREADABLESIZE" | "PARSEREADABLESIZEORZERO" | "PARSEREADABLESIZEORNULL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "parseReadableSize requires one string")?;
            match input {
                None => ConstantValue::Null,
                Some(input) => match parse_readable_size_bytes(&input) {
                    Some(bytes) => ConstantValue::Integer(bytes),
                    None if name == "PARSEREADABLESIZEORZERO" => ConstantValue::Integer(0),
                    None if name == "PARSEREADABLESIZEORNULL" => ConstantValue::Null,
                    None => return Err(ParseError("invalid readable byte size".into())),
                },
            }
        }
        "TOINT8ORZERO" | "TOINT16ORZERO" | "TOINT32ORZERO" | "TOINT64ORZERO" => {
            let text = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "integer OrZero cast requires one argument")?;
            match text {
                None => ConstantValue::Null,
                Some(text) => {
                    let parsed = match name.as_str() {
                    "TOINT8ORZERO" => text.parse::<i128>().ok().map(|n| i64::from(n as i8)),
                    "TOINT16ORZERO" => text.parse::<i128>().ok().map(|n| i64::from(n as i16)),
                    "TOINT32ORZERO" => text.parse::<i128>().ok().map(|n| i64::from(n as i32)),
                    "TOINT64ORZERO" => text.parse::<i64>().ok(),
                        _ => unreachable!("only registered casts reach this branch"),
                    };
                    ConstantValue::Integer(parsed.unwrap_or(0))
                }
            }
        }
        "TOUINT8ORZERO" | "TOUINT16ORZERO" | "TOUINT32ORZERO" => {
            let text = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "integer OrZero cast requires one argument")?;
            match text {
                None => ConstantValue::Null,
                Some(text) => {
                    let parsed = match name.as_str() {
                    "TOUINT8ORZERO" => text.parse::<u128>().ok().map(|n| i64::from(n as u8)),
                    "TOUINT16ORZERO" => text.parse::<u128>().ok().map(|n| i64::from(n as u16)),
                    "TOUINT32ORZERO" => text.parse::<u128>().ok().map(|n| i64::from(n as u32)),
                        _ => unreachable!("only registered casts reach this branch"),
                    };
                    ConstantValue::Integer(parsed.unwrap_or(0))
                }
            }
        }
        "TOINT8ORNULL" | "TOINT16ORNULL" | "TOINT32ORNULL" => {
            let text = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "integer OrNull cast requires one argument")?;
            match text {
                None => ConstantValue::Null,
                Some(text) => {
                    let parsed = match name.as_str() {
                    "TOINT8ORNULL" => text.parse::<i128>().ok().map(|n| i64::from(n as i8)),
                    "TOINT16ORNULL" => text.parse::<i128>().ok().map(|n| i64::from(n as i16)),
                    "TOINT32ORNULL" => text.parse::<i128>().ok().map(|n| i64::from(n as i32)),
                        _ => unreachable!("only registered casts reach this branch"),
                    };
                    parsed.map_or(ConstantValue::Null, ConstantValue::Integer)
                }
            }
        }
        "TOUINT8ORNULL" | "TOUINT16ORNULL" | "TOUINT32ORNULL" => {
            let text = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "integer OrNull cast requires one argument")?;
            match text {
                None => ConstantValue::Null,
                Some(text) => {
                    let parsed = match name.as_str() {
                    "TOUINT8ORNULL" => text.parse::<u128>().ok().map(|n| i64::from(n as u8)),
                    "TOUINT16ORNULL" => text.parse::<u128>().ok().map(|n| i64::from(n as u16)),
                    "TOUINT32ORNULL" => text.parse::<u128>().ok().map(|n| i64::from(n as u32)),
                        _ => unreachable!("only registered casts reach this branch"),
                    };
                    parsed.map_or(ConstantValue::Null, ConstantValue::Integer)
                }
            }
        }
        "TRY_TO_INT64" | "TOINT64ORNULL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "TRY_TO_INT64 requires one argument")?;
            match input {
                Some(value) => value
                    .parse::<i64>()
                    .map_or(ConstantValue::Null, ConstantValue::Integer),
                None => ConstantValue::Null,
            }
        }
        "APPENDTRAILINGCHARIFABSENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "appendTrailingCharIfAbsent requires a character",
            )?;
            let ending = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after appendTrailingCharIfAbsent",
            )?;
            match (input, ending) {
                (Some(mut input), Some(ending)) => {
                    if ending.chars().count() != 1 {
                        return Err(ParseError(
                            "appendTrailingCharIfAbsent requires exactly one character".into(),
                        ));
                    }
                    if !input.is_empty() && !input.ends_with(&ending) {
                        if input.len().saturating_add(ending.len()) > MAX_TEXT_BYTES {
                            return Err(ParseError(
                                "appendTrailingCharIfAbsent result is too large".into(),
                            ));
                        }
                        input.push_str(&ending);
                    }
                    ConstantValue::Text(input)
                }
                _ => ConstantValue::Null,
            }
        }
        "RPAD" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "RPAD requires a width")?;
            let width = integer_arg(parser, depth + 1)?;
            let padding = if parser.take(Token::Comma) {
                text_arg(parser, depth + 1)?
            } else {
                Some(" ".to_owned())
            };
            require(parser, Token::RParen, "expected ')' after RPAD arguments")?;
            match (input, width, padding) {
                (Some(input), Some(width), Some(padding)) => {
                    let width = usize::try_from(width)
                        .map_err(|_| ParseError("RPAD width must be nonnegative".into()))?;
                    if width > MAX_TEXT_BYTES {
                        return Err(ParseError("RPAD result is too large".into()));
                    }
                    let mut result: String = input.chars().take(width).collect();
                    if result.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("RPAD result is too large".into()));
                    }
                    let missing = width - result.chars().count();
                    if missing > 0 && padding.is_empty() {
                        return Err(ParseError("RPAD padding cannot be empty".into()));
                    }
                    for character in padding.chars().cycle().take(missing) {
                        if result.len() + character.len_utf8() > MAX_TEXT_BYTES {
                            return Err(ParseError("RPAD result is too large".into()));
                        }
                        result.push(character);
                    }
                    ConstantValue::Text(result)
                }
                _ => ConstantValue::Null,
            }
        }
        "BASE64_DECODE" | "BASE64DECODE" | "TRYBASE64DECODE" | "BASE64URLDECODE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "BASE64_DECODE requires one argument")?;
            match input {
                Some(value) => {
                    let decoded = (|| -> Result<ConstantValue, ParseError> {
                    fn digit(value: u8) -> Option<u8> {
                        match value {
                            b'A'..=b'Z' => Some(value - b'A'),
                            b'a'..=b'z' => Some(value - b'a' + 26),
                            b'0'..=b'9' => Some(value - b'0' + 52),
                            b'+' => Some(62),
                            b'/' => Some(63),
                            _ => None,
                        }
                    }
                    let mut normalized = Vec::new();
                    let source: &[u8] = if name == "BASE64URLDECODE" {
                        normalized.reserve(value.len().saturating_add(3));
                        for byte in value.bytes() {
                            normalized.push(match byte {
                                b'-' => b'+',
                                b'_' => b'/',
                                b'+' | b'/' => {
                                    return Err(ParseError("invalid BASE64URLDECODE alphabet".into()));
                                }
                                byte => byte,
                            });
                        }
                        match normalized.len() % 4 {
                            0 => {}
                            2 => normalized.extend_from_slice(b"=="),
                            3 => normalized.push(b'='),
                            _ => return Err(ParseError("invalid BASE64URLDECODE length".into())),
                        }
                        normalized.as_slice()
                    } else {
                        value.as_bytes()
                    };
                    if source.len() % 4 != 0 {
                        return Err(ParseError("BASE64_DECODE requires four-byte groups".into()));
                    }
                    let mut bytes = Vec::with_capacity(source.len() / 4 * 3);
                    for (index, group) in source.chunks_exact(4).enumerate() {
                        let last = index + 1 == source.len() / 4;
                        let a = digit(group[0])
                            .ok_or_else(|| ParseError("invalid BASE64_DECODE character".into()))?;
                        let b = digit(group[1])
                            .ok_or_else(|| ParseError("invalid BASE64_DECODE character".into()))?;
                        let (c, d, width) = if group[2] == b'=' {
                            if group[3] != b'=' || !last || b & 15 != 0 {
                                return Err(ParseError("invalid BASE64_DECODE padding".into()));
                            }
                            (0, 0, 1)
                        } else if group[3] == b'=' {
                            let c = digit(group[2]).ok_or_else(|| {
                                ParseError("invalid BASE64_DECODE character".into())
                            })?;
                            if !last || c & 3 != 0 {
                                return Err(ParseError("invalid BASE64_DECODE padding".into()));
                            }
                            (c, 0, 2)
                        } else {
                            let c = digit(group[2]).ok_or_else(|| {
                                ParseError("invalid BASE64_DECODE character".into())
                            })?;
                            let d = digit(group[3]).ok_or_else(|| {
                                ParseError("invalid BASE64_DECODE character".into())
                            })?;
                            (c, d, 3)
                        };
                        let bits = (u32::from(a) << 18)
                            | (u32::from(b) << 12)
                            | (u32::from(c) << 6)
                            | u32::from(d);
                        bytes.push((bits >> 16) as u8);
                        if width > 1 {
                            bytes.push((bits >> 8) as u8);
                        }
                        if width > 2 {
                            bytes.push(bits as u8);
                        }
                    }
                    Ok(ConstantValue::Text(
                        String::from_utf8(bytes)
                            .map_err(|_| ParseError("BASE64_DECODE result is not UTF-8".into()))?,
                    ))
                    })();
                    match decoded {
                        Ok(value) => value,
                        Err(_) if name == "TRYBASE64DECODE" => {
                            ConstantValue::Text(String::new())
                        }
                        Err(error) => return Err(error),
                    }
                }
                None => ConstantValue::Null,
            }
        }
        "EXTRACTURLPARAMETER" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "extractURLParameter requires a parameter name",
            )?;
            let name = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after extractURLParameter",
            )?;
            match (input, name) {
                (Some(input), Some(name)) => {
                    let before_fragment = input
                        .split_once('#')
                        .map_or(input.as_str(), |(head, _)| head);
                    let query = before_fragment
                        .split_once('?')
                        .map_or("", |(_, query)| query);
                    let value = query
                        .split('&')
                        .find_map(|part| {
                            if part == name {
                                Some("")
                            } else {
                                part.split_once('=')
                                    .and_then(|(key, value)| (key == name).then_some(value))
                            }
                        })
                        .unwrap_or("");
                    ConstantValue::Text(value.to_owned())
                }
                _ => ConstantValue::Null,
            }
        }
        "ISVALIDJSON" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "isValidJSON requires one argument")?;
            match input {
                Some(input) => {
                    ConstantValue::Integer(i64::from(crate::json::parse_json(&input).is_ok()))
                }
                None => ConstantValue::Null,
            }
        }
        "JSONHAS" | "JSONEXTRACTSTRING" | "JSONEXTRACTRAW" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "JSON function requires a key")?;
            let path = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "JSON function requires two arguments",
            )?;
            match (input, path) {
                (Some(input), Some(path)) => {
                    let json = crate::json::parse_json(&input)
                        .map_err(|error| ParseError(format!("invalid JSON: {error}")))?;
                    let found = json.at_path(&path);
                    match name.as_str() {
                        "JSONHAS" => ConstantValue::Integer(i64::from(found.is_some())),
                        "JSONEXTRACTSTRING" => ConstantValue::Text(
                            found
                                .and_then(|value| value.as_string())
                                .unwrap_or("")
                                .to_owned(),
                        ),
                        _ => ConstantValue::Text(
                            found.map(|value| value.raw()).unwrap_or("").to_owned(),
                        ),
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "JSONLENGTH" => {
            let input = text_arg(parser, depth + 1)?;
            let path = if parser.take(Token::Comma) {
                Some(text_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(parser, Token::RParen, "JSONLength requires one or two arguments")?;
            match (input, path) {
                (None, _) | (_, Some(None)) => ConstantValue::Null,
                (Some(input), path) => {
                    let json = crate::json::parse_json(&input)
                        .map_err(|error| ParseError(format!("invalid JSON: {error}")))?;
                    let value = match path {
                        Some(Some(path)) => json.at_path(&path),
                        None => Some(&json),
                        Some(None) => unreachable!(),
                    };
                    let length = value.map_or(0, |node| node.container_len());
                    ConstantValue::Integer(
                        i64::try_from(length).map_err(|_| ParseError("JSONLength exceeds Int64".into()))?
                    )
                }
            }
        }
        "JSONEXTRACTINT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "JSONExtractInt requires a path")?;
            let path = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "JSONExtractInt requires two arguments")?;
            match (input, path) {
                (Some(input), Some(path)) => {
                    let json = crate::json::parse_json(&input)
                        .map_err(|error| ParseError(format!("invalid JSON: {error}")))?;
                    let value = json
                        .at_path(&path)
                        .and_then(|value| value.raw().parse::<i64>().ok())
                        .unwrap_or(0);
                    ConstantValue::Integer(value)
                }
                _ => ConstantValue::Null,
            }
        }
        "JSON_QUOTE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "JSON_QUOTE requires one argument")?;
            match input {
                Some(value) => {
                    let mut result = String::with_capacity(value.len().saturating_add(2));
                    result.push('"');
                    for character in value.chars() {
                        match character {
                            '"' => result.push_str("\\\""),
                            '\\' => result.push_str("\\\\"),
                            '\n' => result.push_str("\\n"),
                            '\r' => result.push_str("\\r"),
                            '\t' => result.push_str("\\t"),
                            '\u{0008}' => result.push_str("\\b"),
                            '\u{000C}' => result.push_str("\\f"),
                            character if character < '\u{0020}' => {
                                const HEX: &[u8; 16] = b"0123456789abcdef";
                                let byte = character as u8;
                                result.push_str("\\u00");
                                result.push(char::from(HEX[usize::from(byte >> 4)]));
                                result.push(char::from(HEX[usize::from(byte & 0x0f)]));
                            }
                            character => result.push(character),
                        }
                        if result.len() >= MAX_TEXT_BYTES {
                            return Err(ParseError("JSON_QUOTE result is too large".into()));
                        }
                    }
                    result.push('"');
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "TOSTRINGCUTTOZERO" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "toStringCutToZero requires one argument")?;
            match input {
                Some(input) => {
                    let before_nul = input.split('\0').next().unwrap_or("");
                    ConstantValue::Text(before_nul.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "REPEAT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "REPEAT requires a count")?;
            let count = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after REPEAT arguments")?;
            match (input, count) {
                (Some(value), Some(count)) => {
                    if count < 0 {
                        return Err(ParseError("REPEAT count must be nonnegative".into()));
                    }
                    if value.is_empty() {
                        ConstantValue::Text(String::new())
                    } else {
                        let count = usize::try_from(count).unwrap_or(usize::MAX);
                        let length = value
                            .len()
                            .checked_mul(count)
                            .ok_or_else(|| ParseError("REPEAT result is too large".into()))?;
                        if length > MAX_TEXT_BYTES {
                            return Err(ParseError("REPEAT result is too large".into()));
                        }
                        ConstantValue::Text(value.repeat(count))
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "LOWERUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "lowerUTF8 requires one argument")?;
            match input {
                Some(mut value) if value.is_ascii() => {
                    value.make_ascii_lowercase();
                    ConstantValue::Text(value)
                }
                Some(value) => {
                    let result = value.to_lowercase();
                    if result.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("lowerUTF8 result is too large".into()));
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "TOYEAR" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "toYear requires one argument")?;
            match input {
                Some(value) => {
                    if rusthouse_storage::parse_date(&value).is_none() {
                        return Err(ParseError("invalid Date argument".into()));
                    }
                    ConstantValue::Integer(value[..4].parse::<i64>().unwrap())
                }
                None => ConstantValue::Null,
            }
        }
        "NOTEMPTY" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "notEmpty requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(i64::from(!value.is_empty())),
                None => ConstantValue::Null,
            }
        }
        _ => return Err(ParseError(format!("unsupported string function: {name}"))),
    };
    Ok(value)
}

fn constant_arg(parser: &mut Parser, depth: usize) -> Result<ConstantValue, ParseError> {
    match parser.tokens.get(parser.index) {
        Some(Token::String(value)) => {
            let result = ConstantValue::Text(value.clone());
            parser.index += 1;
            Ok(result)
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL") => {
            parser.index += 1;
            Ok(ConstantValue::Null)
        }
        Some(Token::Word(word)) if is_function(word) => parse_at_depth(parser, depth),
        _ => Ok(ConstantValue::Integer(super::scalar::parse(parser)?)),
    }
}

fn require(parser: &mut Parser, token: Token, message: &str) -> Result<(), ParseError> {
    if parser.take(token) {
        Ok(())
    } else {
        Err(ParseError(message.into()))
    }
}

pub(super) fn integer_arg(parser: &mut Parser, depth: usize) -> Result<Option<i64>, ParseError> {
    if matches!(
        parser.tokens.get(parser.index),
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL")
    ) {
        parser.index += 1;
        return Ok(None);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if is_function(word)) {
        return match parse_at_depth(parser, depth)? {
            ConstantValue::Integer(value) => Ok(Some(value)),
            ConstantValue::Null => Ok(None),
            ConstantValue::Text(_) => Err(ParseError("expected an integer expression".into())),
        };
    }
    Ok(Some(super::scalar::parse(parser)?))
}

pub(super) fn text_arg(parser: &mut Parser, depth: usize) -> Result<Option<String>, ParseError> {
    match parser.tokens.get(parser.index) {
        Some(Token::String(value)) => {
            let result = value.clone();
            parser.index += 1;
            Ok(Some(result))
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL") => {
            parser.index += 1;
            Ok(None)
        }
        Some(Token::Word(word)) if is_function(word) => match parse_at_depth(parser, depth)? {
            ConstantValue::Text(value) => Ok(Some(value)),
            ConstantValue::Null => Ok(None),
            ConstantValue::Integer(_) => Err(ParseError("expected a string expression".into())),
        },
        _ => Err(ParseError(
            "expected a string literal or string function".into(),
        )),
    }
}

#[cfg(test)]
mod clickhouse_readable_time_delta_tests {
    use crate::{parse_select, ConstantValue, Projection};
    #[test]
    fn formats_second_intervals_and_unit_limits() {
        for (sql, expected) in [
            ("SELECT formatReadableTimeDelta(0)", "0 seconds"),
            ("SELECT formatReadableTimeDelta(12345)", "3 hours, 25 minutes and 45 seconds"),
            ("SELECT formatReadableTimeDelta(100, 'minutes')", "1 minute and 40 seconds"),
            ("SELECT formatReadableTimeDelta(12345, 'minutes')", "205 minutes and 45 seconds"),
            ("SELECT formatReadableTimeDelta(-60)", "-1 minute"),
            ("SELECT formatReadableTimeDelta(3600, 'minutes', 'minutes')", "60 minutes"),
            ("SELECT formatReadableTimeDelta(5456456)", "2 months, 2 days, 3 hours, 40 minutes and 56 seconds"),
        ] {
            assert!(matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1 && values[0].value == ConstantValue::Text(expected.into())), "{sql}");
        }
        assert!(parse_select("SELECT formatReadableTimeDelta(60, 'invalid')").is_err());
        assert!(parse_select("SELECT formatReadableTimeDelta(60, 'minutes', 'hours')").is_err());
    }
}

#[cfg(test)]
mod string_bytes_uniq_bitmap_tests {
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn counts_distinct_bytes_including_utf8_and_duplicates() {
        for (sql, expected) in [
            ("SELECT stringBytesUniq('')", 0),
            ("SELECT stringBytesUniq('aaaa')", 1),
            ("SELECT stringBytesUniq('abcabc')", 3),
            ("SELECT stringBytesUniq('éé')", 2),
            ("SELECT stringBytesUniq('aé')", 3),
        ] {
            let parsed = parse_select(sql).unwrap();
            assert!(
                matches!(parsed.projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Integer(expected)),
                "{sql}",
            );
        }
        let parsed = parse_select("SELECT stringBytesUniq(NULL)").unwrap();
        assert!(matches!(parsed.projection, Projection::Constants(values)
            if values[0].value == ConstantValue::Null));
    }
}

#[cfg(test)]
mod ch_position_byte_tests {
    use crate::{parse_select, ConstantValue, Projection};
    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            Projection::Literal(value) => ConstantValue::Integer(value),
            other => panic!("unexpected projection: {other:?}"),
        }
    }
    #[test]
    fn supported_values_and_invalid_arity() {
        for (sql, expected) in [
            ("SELECT position('éa', 'a')", ConstantValue::Integer(3)),
            ("SELECT position('hello', 'll')", ConstantValue::Integer(3)),
            ("SELECT position('abc', 'Z')", ConstantValue::Integer(0)),
            ("SELECT position(NULL, 'a')", ConstantValue::Null),
        ] {
            assert_eq!(value(sql), expected, "{sql}");
        }
        for sql in ["SELECT position('abc')", "SELECT position('abc','a','x')"] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }
}

#[cfg(test)]
mod clickhouse_decode_url_form_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn decodes_plus_and_percent_escaped_form_components() {
        for (sql, expected) in [
            ("SELECT decodeURLFormComponent('hello+world')", "hello world"),
            ("SELECT decodeURLFormComponent('x%2By+z')", "x+y z"),
            ("SELECT decodeURLFormComponent('a%20b')", "a b"),
            ("SELECT decodeURLFormComponent('')", ""),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Text(expected.into())),
                "{sql}",
            );
        }
        for sql in [
            "SELECT decodeURLFormComponent('%GG')",
            "SELECT decodeURLFormComponent('%')",
            "SELECT decodeURLFormComponent('x', 'y')",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }
}

#[cfg(test)]
mod clickhouse_ipv4_to_num_or_null_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn valid_ipv4_is_numeric_and_invalid_input_is_null() {
        for (sql, expected) in [
            ("SELECT IPv4StringToNumOrNull('192.168.0.1')", 3232235521),
            ("SELECT IPv4StringToNumOrNull('0.0.0.0')", 0),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Integer(expected)),
                "{sql}",
            );
        }
        for sql in [
            "SELECT IPv4StringToNumOrNull('invalid')",
            "SELECT IPv4StringToNumOrNull(NULL)",
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1 && values[0].value == ConstantValue::Null),
                "{sql}",
            );
        }
        assert!(parse_select("SELECT IPv4StringToNumOrNull('x', 'y')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_ipv4_string_to_num_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn converts_ipv4_to_network_order_integer() {
        for (sql, expected) in [
            ("SELECT IPv4StringToNum('0.0.0.0')", 0),
            ("SELECT IPv4StringToNum('192.168.0.1')", 3232235521),
            ("SELECT IPv4StringToNum('255.255.255.255')", 4294967295),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Integer(expected)),
                "{sql}",
            );
        }
        for sql in [
            "SELECT IPv4StringToNum('256.1.2.3')",
            "SELECT IPv4StringToNum('1.2.3')",
            "SELECT IPv4StringToNum('1.2.3.4', 'x')",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }
}

#[cfg(test)]
mod clickhouse_ipv4_num_to_string_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn formats_network_order_ipv4_integers() {
        for (sql, expected) in [
            ("SELECT IPv4NumToString(0)", "0.0.0.0"),
            ("SELECT IPv4NumToString(3232235521)", "192.168.0.1"),
            ("SELECT IPv4NumToString(4294967295)", "255.255.255.255"),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Text(expected.into())),
                "{sql}",
            );
        }
        for sql in [
            "SELECT IPv4NumToString(-1)",
            "SELECT IPv4NumToString(4294967296)",
            "SELECT IPv4NumToString(1, 2)",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }
}

#[cfg(test)]
mod clickhouse_json_extract_bool_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn reads_boolean_paths_and_defaults_nonbooleans_to_zero() {
        for (sql, expected) in [
            (r#"SELECT JSONExtractBool('{"x":true}', 'x')"#, 1),
            (r#"SELECT JSONExtractBool('{"x":false}', 'x')"#, 0),
            (r#"SELECT JSONExtractBool('{"x":1}', 'x')"#, 0),
            (r#"SELECT JSONExtractBool('{"x":{"y":true}}', 'x.y')"#, 1),
            (r#"SELECT JSONExtractBool('{"x":true}', 'missing')"#, 0),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Integer(expected)),
                "{sql}",
            );
        }
        assert!(parse_select("SELECT JSONExtractBool('not-json', 'x')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_cut_url_parameter_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn removes_named_query_parameters_and_preserves_fragments() {
        for (sql, expected) in [
            ("SELECT cutURLParameter('https://a/?x=1&y=2', 'x')",
                "https://a/?y=2"),
            ("SELECT cutURLParameter('https://a/?x=1&x=2#frag', 'x')",
                "https://a/#frag"),
            ("SELECT cutURLParameter('https://a/?x=1', 'missing')",
                "https://a/?x=1"),
            ("SELECT cutURLParameter('https://a/path#frag', 'x')",
                "https://a/path#frag"),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Text(expected.into())),
                "{sql}",
            );
        }
        assert!(parse_select("SELECT cutURLParameter('x')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_domain_without_www_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn extracts_domain_without_at_most_one_www_prefix() {
        for (sql, expected) in [
            ("SELECT domainWithoutWWW('https://www.example.org/p')", "example.org"),
            ("SELECT domainWithoutWWW('www.example.org:80/p')", "example.org"),
            ("SELECT domainWithoutWWW('https://www.www.example.org')", "www.example.org"),
            ("SELECT domainWithoutWWW('https://example.org')", "example.org"),
            ("SELECT domainWithoutWWW('not-a-url')", "not-a-url"),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Text(expected.into())),
                "{sql}",
            );
        }
        assert!(parse_select("SELECT domainWithoutWWW('x', 'y')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_cut_www_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn removes_one_www_prefix_without_changing_other_url_components() {
        for (sql, expected) in [
            ("SELECT cutWWW('https://www.example.org/a?b=1')",
                "https://example.org/a?b=1"),
            ("SELECT cutWWW('www.example.org/path')", "example.org/path"),
            ("SELECT cutWWW('https://www.www.example.org')",
                "https://www.example.org"),
            ("SELECT cutWWW('https://example.org')", "https://example.org"),
            ("SELECT cutWWW('https://u@www.example.org:80/x')",
                "https://u@example.org:80/x"),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Text(expected.into())),
                "{sql}",
            );
        }
        assert!(parse_select("SELECT cutWWW('a', 'b')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_ipv4_string_default_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn invalid_ipv4_strings_return_zero() {
        for (sql, expected) in [
            ("SELECT IPv4StringToNumOrDefault('127.0.0.1')", 2130706433),
            ("SELECT IPv4StringToNumOrDefault('invalid')", 0),
            ("SELECT IPv4StringToNumOrDefault('256.0.0.1')", 0),
            ("SELECT IPv4StringToNumOrDefault('0.0.0.0')", 0),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Integer(expected)),
                "{sql}",
            );
        }
        assert!(parse_select("SELECT IPv4StringToNumOrDefault('a', 0)").is_err());
    }
}

#[cfg(test)]
mod clickhouse_ipv4_class_c_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn redacts_low_octet_of_network_order_ipv4() {
        for (sql, expected) in [
            ("SELECT IPv4NumToStringClassC(3232235521)", "192.168.0.xxx"),
            ("SELECT IPv4NumToStringClassC(0)", "0.0.0.xxx"),
            ("SELECT IPv4NumToStringClassC(4294967295)", "255.255.255.xxx"),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Text(expected.into())),
                "{sql}",
            );
        }
        assert!(parse_select("SELECT IPv4NumToStringClassC(-1)").is_err());
        assert!(parse_select("SELECT IPv4NumToStringClassC(1,2)").is_err());
    }
}

#[cfg(test)]
mod clickhouse_mac_string_to_num_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn parses_six_octets_and_defaults_invalid_formats_to_zero() {
        for (sql, expected) in [
            ("SELECT MACStringToNum('01:02:03:04:05:06')", 1108152157446),
            ("SELECT MACStringToNum('ff:ff:ff:ff:ff:ff')", 281474976710655),
            ("SELECT MACStringToNum('bad')", 0),
            ("SELECT MACStringToNum('01:02:03:04:05')", 0),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Integer(expected)),
                "{sql}",
            );
        }
        assert!(parse_select("SELECT MACStringToNum('x','y')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_mac_num_to_string_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn uses_big_endian_six_octet_mac_format() {
        for (sql, expected) in [
            ("SELECT MACNumToString(1108152157446)", "01:02:03:04:05:06"),
            ("SELECT MACNumToString(0)", "00:00:00:00:00:00"),
            ("SELECT MACNumToString(281474976710655)", "FF:FF:FF:FF:FF:FF"),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Text(expected.into())),
                "{sql}",
            );
        }
        assert!(parse_select("SELECT MACNumToString(-1)").is_err());
        assert!(parse_select("SELECT MACNumToString(281474976710656)").is_err());
    }
}

#[cfg(test)]
mod clickhouse_mac_string_to_oui_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn extracts_vendor_oui_prefix_of_valid_mac() {
        for (sql, expected) in [
            ("SELECT MACStringToOUI('01:02:03:AA:BB:CC')", 66051),
            ("SELECT MACStringToOUI('ff:ff:ff:ff:ff:ff')", 16777215),
            ("SELECT MACStringToOUI('00:00:00:12:34:56')", 0),
            ("SELECT MACStringToOUI('invalid')", 0),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Integer(expected)),
                "{sql}",
            );
        }
        assert!(parse_select("SELECT MACStringToOUI('x', 'y')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_ip_address_in_range_tests {
    use crate::{parse_select, ConstantValue, Projection};

    #[test]
    fn matches_ipv4_and_ipv6_with_prefix_boundaries() {
        for (sql, expected) in [
            ("SELECT isIPAddressInRange('192.168.1.10', '192.168.0.0/16')", 1),
            ("SELECT isIPAddressInRange('192.167.1.10', '192.168.0.0/16')", 0),
            ("SELECT isIPAddressInRange('1.2.3.4', '0.0.0.0/0')", 1),
            ("SELECT isIPAddressInRange('2001:db8::1', '2001:db8::/32')", 1),
            ("SELECT isIPAddressInRange('2001:db9::1', '2001:db8::/32')", 0),
            ("SELECT isIPAddressInRange('::1', '::1/128')", 1),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1
                        && values[0].value == ConstantValue::Integer(expected)),
                "{sql}",
            );
        }
        for sql in [
            "SELECT isIPAddressInRange('1.2.3.4', '1.2.3.4/33')",
            "SELECT isIPAddressInRange('1.2.3.4', '::1/128')",
            "SELECT isIPAddressInRange('x', '1.2.3.4/32')",
            "SELECT isIPAddressInRange('1.2.3.4')",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }
}

#[cfg(test)]
mod mac_oui_parser_reuse_tests {
    use crate::{parse_select, Projection};

    #[test]
    fn mac_oui_reuses_strict_six_byte_parsing() {
        for (sql, expected) in [
            ("SELECT MACStringToOUI('AA:BB:CC:DD:EE:FF')", 0xAA_BB_CC),
            ("SELECT MACStringToOUI('00:00:00:00:00:01')", 0),
            ("SELECT MACStringToOUI('GG:BB:CC:DD:EE:FF')", 0),
            ("SELECT MACStringToOUI('AA:BB:CC')", 0),
        ] {
            assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
        }
        assert!(parse_select("SELECT MACStringToOUI('AA:BB:CC:DD:EE:FF', 1)").is_err());
    }
}
