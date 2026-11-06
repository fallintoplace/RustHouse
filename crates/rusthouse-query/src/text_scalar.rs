mod helpers;
mod dispatch;
use dispatch::parse_at_depth;
use self::helpers::*;
use super::{ConstantValue, ParseError, Parser, Token};

const MAX_DEPTH: usize = 16;
const MAX_TEXT_BYTES: usize = 1_048_576;

pub(super) fn is_function(name: &str) -> bool {
    super::date_scalar::is_function(name)
        || matches!(
            name.to_ascii_uppercase().as_str(),
            "CRC32"
                | "MATCH"
                | "CRC32IEEE"
                | "ISIPADDRESSINRANGE"
                | "MACSTRINGTOOUI"
                | "MACNUMTOSTRING"
                | "MACSTRINGTONUM"
                | "IPV4NUMTOSTRINGCLASSC"
                | "IPV4STRINGTONUMORDEFAULT"
                | "CUTWWW"
                | "FIRSTSIGNIFICANTSUBDOMAIN"
                | "CUTTOFIRSTSIGNIFICANTSUBDOMAIN"
                | "CUTTOFIRSTSIGNIFICANTSUBDOMAINWITHWWW"
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
                | "CHARACTER_LENGTH"
                | "OCTET_LENGTH"
                | "REGEXP_LIKE"
                | "TOSTARTOFFIFTEENMINUTES"
                | "ASCII"
                | "TOSTARTOFMINUTE"
                | "UTF16_LENGTH"
                | "ISIPV6STRING"
                | "HASSUBSEQUENCE"
                | "HASSUBSEQUENCEUTF8"
                | "BYTEHAMMINGDISTANCE"
                | "CHR"
                | "TOUNIXTIMESTAMP"
                | "STRINGBYTESUNIQ"
                | "SOUNDEX"
                | "REGEXP_COUNT"
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
                | "LCASE"
                | "LOWERUTF8"
                | "REGEXP_INSTR"
                | "HEX_ENCODE"
                | "DECODEXMLCOMPONENT"
                | "ENCODEXMLCOMPONENT"
                | "HEX"
                | "IS_ASCII"
                | "ISIPV4STRING"
                | "UPPER"
                | "UCASE"
                | "UPPERUTF8"
                | "URL_DECODE"
                | "DECODEURLCOMPONENT"
                | "TRYDECODEURLCOMPONENT"
                | "IS_EMPTY"
                | "REGEXP_SUBSTR"
                | "EMPTY"
                | "REVERSE"
                | "REVERSEUTF8"
                | "REVERSEBYSEPARATOR"
                | "INITCAP"
                | "INITCAPUTF8"
                | "TOSTARTOFTENMINUTES"
                | "CONCAT"
                | "CONCATASSUMEINJECTIVE"
                | "QUOTE_LITERAL"
                | "FROMUNIXTIMESTAMP64SECOND"
                | "FROMUNIXTIMESTAMP64MILLI"
                | "COUNTMATCHES"
                | "CUTQUERYSTRINGANDFRAGMENT"
                | "THROWIF"
                | "MULTIIF"
                | "COALESCE"
                | "SUBTRACTHOURS"
                | "CONCAT_WS"
                | "CONCATWITHSEPARATOR"
                | "CUTFRAGMENT"
                | "ENCODEURLCOMPONENT"
                | "URL_ENCODE"
                | "FRAGMENT"
                | "DATETRUNC"
                | "JSON_QUOTE"
                | "COUNTMATCHESCASEINSENSITIVE"
                | "REGEXPQUOTEMETA"
                | "ISVALIDJSON"
                | "JSONTYPE"
                | "JSONARRAYLENGTH"
                | "JSONLENGTH"
                | "JSON_EXISTS"
                | "JSONHAS"
                | "JSONEXTRACTSTRING"
                | "JSONEXTRACTINT"
                | "JSONEXTRACTRAW"
                | "SUBSTRING"
                | "SUBSTRINGUTF8"
                | "EXTRACT"
                | "HASSUBSEQUENCECASEINSENSITIVE"
                | "HASSUBSEQUENCECASEINSENSITIVEUTF8"
                | "OVERLAY"
                | "FORMAT"
                | "OVERLAYUTF8"
                | "FORMATREADABLEQUANTITY"
                | "QUERYSTRING"
                | "SPLIT_PART"
                | "SUBSTRING_INDEX"
                | "SUBSTRINGINDEXUTF8"
                | "QUERYSTRINGANDFRAGMENT"
                | "FORMATREADABLETIMEDELTA"
                | "TOINT8"
                | "REGEXPEXTRACT"
                | "TOINT16"
                | "TOINT32"
                | "TOINT64"
                | "TO_INT64"
                | "TOSTRING"
                | "TRIM"
                | "LTRIM"
                | "TRIMLEFT"
                | "BASENAME"
                | "FROMUNIXTIMESTAMP64MICRO"
                | "TRIM_WHITESPACE"
                | "TRY_TO_INT64"
                | "TOUINT8"
                | "STRCMP"
                | "TOUINT16"
                | "TOUINT32"
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
                | "REPLACEREGEXPONE"
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
                | "REPLACEREGEXPALL"
                | "TOINT64ORNULL"
                | "LPAD"
                | "LEFTPAD"
                | "LEFTPADUTF8"
                | "COUNTSUBSTRINGSCASEINSENSITIVE"
                | "HEX_DECODE"
                | "UNHEX"
                | "UNBIN"
                | "REPLACE"
                | "REPLACEALL"
                | "PORT"
                | "PROTOCOL"
                | "FROMUNIXTIMESTAMP64NANO"
                | "REGEXP_REPLACE"
                | "TRANSLATE"
                | "TRANSLATEUTF8"
                | "POSITIONCASEINSENSITIVE"
                | "POSITIONCASEINSENSITIVEUTF8"
                | "STRPOS"
                | "TOSTARTOFISOYEAR"
                | "POSITIONUTF8"
                | "COUNT_SUBSTRING"
                | "COUNTSUBSTRINGS"
                | "EXTRACTURLPARAMETER"
                | "NULLIF"
                | "IF"
                | "LOCATE"
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
                | "HASTOKENORNULL"
                | "INSTR"
                | "TOMONTHNUMSINCEEPOCH"
                | "CONTAINS"
                | "STRIP_PREFIX"
                | "RIGHT"
                | "RIGHTUTF8"
                | "TO_BASE64"
                | "TO_BASE64URL"
                | "FROM_BASE64"
                | "FROM_BASE64URL"
                | "BASE64_ENCODE"
                | "BASE64URLENCODE"
                | "BASE64ENCODE"
                | "FIND_IN_SET"
                | "BASE58ENCODE"
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
                | "TO_BASE"
                | "REPEAT"
                | "BASE58DECODE"
                | "BASE64_DECODE"
                | "BASE64URLDECODE"
                | "TRYBASE64URLDECODE"
                | "TRYBASE64DECODE"
                | "BASE64DECODE"
                | "APPENDTRAILINGCHARIFABSENT"
                | "ADDMONTHS"
                | "RPAD"
                | "RIGHTPAD"
                | "NOTEMPTY"
                | "SPACE"
                | "FROM_BASE"
                | "INTDIVORNULL"
                | "MODULOORNULL"
                | "FIRSTLINE"
                | "TOPLEVELDOMAIN"
                | "COMPARESUBSTRINGS"
                | "EDITDISTANCE"
                | "DAMERAULEVENSHTEINDISTANCE"
                | "EDITDISTANCEUTF8"
                | "FORMATREADABLESIZE"
                | "FORMATREADABLEDECIMALSIZE"
                | "TODECIMALSTRING"
                | "TOTYPENAME"
                | "JSONEXTRACTFLOAT"
                | "POSITION"
                | "ENCODEURLFORMCOMPONENT"
                | "ISIPV4PRIVATE"
                | "ISIPV4LOOPBACK"
                | "ISIPV4MULTICAST"
                | "ISIPV4LINKLOCAL"
                | "ISIPV6UNIQUELOCAL"
                | "ISIPV6LOOPBACK"
                | "ISIPV6MULTICAST"
                | "BASE32DECODE"
                | "TRYBASE32DECODE"
                | "FROM_BASE32"
                | "BASE32ENCODE"
                | "IS_NUMERIC"
                | "TO_BASE32"
                | "ENDSWITHCASEINSENSITIVE"
                | "ENDSWITHCASEINSENSITIVEUTF8"
                | "STARTSWITHCASEINSENSITIVE"
                | "STARTSWITHCASEINSENSITIVEUTF8"
                | "BIN"
                | "ISNOTNULL"
                | "ISNULL"
                | "ISVALIDASCII"
                | "TRIMRIGHT"
                | "STARTSWITHUTF8"
                | "RIGHTPADUTF8"
                | "NORMALIZEIPV6"
        )
}

pub(super) fn parse(parser: &mut Parser) -> Result<ConstantValue, ParseError> {
    parse_at_depth(parser, 0)
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
            ConstantValue::Text(_) | ConstantValue::Decimal(_) => {
                Err(ParseError("expected an integer expression".into()))
            }
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
            ConstantValue::Integer(_) | ConstantValue::Decimal(_) => {
                Err(ParseError("expected a string expression".into()))
            }
        },
        _ => Err(ParseError(
            "expected a string literal or string function".into(),
        )),
    }
}

#[cfg(test)]
mod clickhouse_readable_time_delta_tests {
    use crate::{ConstantValue, Projection, parse_select};
    #[test]
    fn formats_second_intervals_and_unit_limits() {
        for (sql, expected) in [
            ("SELECT formatReadableTimeDelta(0)", "0 seconds"),
            (
                "SELECT formatReadableTimeDelta(12345)",
                "3 hours, 25 minutes and 45 seconds",
            ),
            (
                "SELECT formatReadableTimeDelta(100, 'minutes')",
                "1 minute and 40 seconds",
            ),
            (
                "SELECT formatReadableTimeDelta(12345, 'minutes')",
                "205 minutes and 45 seconds",
            ),
            ("SELECT formatReadableTimeDelta(-60)", "-1 minute"),
            (
                "SELECT formatReadableTimeDelta(3600, 'minutes', 'minutes')",
                "60 minutes",
            ),
            (
                "SELECT formatReadableTimeDelta(5456456)",
                "2 months, 2 days, 3 hours, 40 minutes and 56 seconds",
            ),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1 && values[0].value == ConstantValue::Text(expected.into())),
                "{sql}"
            );
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
    use crate::{ConstantValue, Projection, parse_select};
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
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn decodes_plus_and_percent_escaped_form_components() {
        for (sql, expected) in [
            (
                "SELECT decodeURLFormComponent('hello+world')",
                "hello world",
            ),
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
    use crate::{ConstantValue, Projection, parse_select};

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
    use crate::{ConstantValue, Projection, parse_select};

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
    use crate::{ConstantValue, Projection, parse_select};

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
    use crate::{ConstantValue, Projection, parse_select};

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
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn removes_named_query_parameters_and_preserves_fragments() {
        for (sql, expected) in [
            (
                "SELECT cutURLParameter('https://a/?x=1&y=2', 'x')",
                "https://a/?y=2",
            ),
            (
                "SELECT cutURLParameter('https://a/?x=1&x=2#frag', 'x')",
                "https://a/#frag",
            ),
            (
                "SELECT cutURLParameter('https://a/?x=1', 'missing')",
                "https://a/?x=1",
            ),
            (
                "SELECT cutURLParameter('https://a/path#frag', 'x')",
                "https://a/path#frag",
            ),
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
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn extracts_domain_without_at_most_one_www_prefix() {
        for (sql, expected) in [
            (
                "SELECT domainWithoutWWW('https://www.example.org/p')",
                "example.org",
            ),
            (
                "SELECT domainWithoutWWW('www.example.org:80/p')",
                "example.org",
            ),
            (
                "SELECT domainWithoutWWW('https://www.www.example.org')",
                "www.example.org",
            ),
            (
                "SELECT domainWithoutWWW('https://example.org')",
                "example.org",
            ),
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
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn removes_one_www_prefix_without_changing_other_url_components() {
        for (sql, expected) in [
            (
                "SELECT cutWWW('https://www.example.org/a?b=1')",
                "https://example.org/a?b=1",
            ),
            ("SELECT cutWWW('www.example.org/path')", "example.org/path"),
            (
                "SELECT cutWWW('https://www.www.example.org')",
                "https://www.example.org",
            ),
            (
                "SELECT cutWWW('https://example.org')",
                "https://example.org",
            ),
            (
                "SELECT cutWWW('https://u@www.example.org:80/x')",
                "https://u@example.org:80/x",
            ),
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
    use crate::{ConstantValue, Projection, parse_select};

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
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn redacts_low_octet_of_network_order_ipv4() {
        for (sql, expected) in [
            ("SELECT IPv4NumToStringClassC(3232235521)", "192.168.0.xxx"),
            ("SELECT IPv4NumToStringClassC(0)", "0.0.0.xxx"),
            (
                "SELECT IPv4NumToStringClassC(4294967295)",
                "255.255.255.xxx",
            ),
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
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn parses_six_octets_and_defaults_invalid_formats_to_zero() {
        for (sql, expected) in [
            ("SELECT MACStringToNum('01:02:03:04:05:06')", 1108152157446),
            (
                "SELECT MACStringToNum('ff:ff:ff:ff:ff:ff')",
                281474976710655,
            ),
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
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn uses_big_endian_six_octet_mac_format() {
        for (sql, expected) in [
            ("SELECT MACNumToString(1108152157446)", "01:02:03:04:05:06"),
            ("SELECT MACNumToString(0)", "00:00:00:00:00:00"),
            (
                "SELECT MACNumToString(281474976710655)",
                "FF:FF:FF:FF:FF:FF",
            ),
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
    use crate::{ConstantValue, Projection, parse_select};

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
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn matches_ipv4_and_ipv6_with_prefix_boundaries() {
        for (sql, expected) in [
            (
                "SELECT isIPAddressInRange('192.168.1.10', '192.168.0.0/16')",
                1,
            ),
            (
                "SELECT isIPAddressInRange('192.167.1.10', '192.168.0.0/16')",
                0,
            ),
            ("SELECT isIPAddressInRange('1.2.3.4', '0.0.0.0/0')", 1),
            (
                "SELECT isIPAddressInRange('2001:db8::1', '2001:db8::/32')",
                1,
            ),
            (
                "SELECT isIPAddressInRange('2001:db9::1', '2001:db8::/32')",
                0,
            ),
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
mod equal_edit_distance_fast_path_tests {
    use super::*;

    #[test]
    fn equal_large_inputs_avoid_quadratic_distance_budget() {
        let text = "a".repeat(4096);
        assert_eq!(
            bounded_edit_distance(text.as_bytes(), text.as_bytes()).unwrap(),
            0
        );
        assert_eq!(bounded_utf8_edit_distance(&text, &text).unwrap(), 0);
        let utf8 = "ü".repeat(2048);
        assert_eq!(bounded_utf8_edit_distance(&utf8, &utf8).unwrap(), 0);
    }

    #[test]
    fn unequal_large_inputs_still_obey_distance_budget() {
        let lhs = "a".repeat(2048);
        let rhs = "b".repeat(2048);
        assert!(bounded_edit_distance(lhs.as_bytes(), rhs.as_bytes()).is_err());
        assert!(bounded_utf8_edit_distance(&lhs, &rhs).is_err());
    }
}

#[cfg(test)]
mod clickhouse_crc32_tests {
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn crc32_matches_ieee_802_3_and_handles_null() {
        for (sql, expected) in [
            ("SELECT CRC32('')", 0),
            ("SELECT CRC32('123456789')", 3_421_780_262),
            ("SELECT CRC32('ClickHouse')", 1_538_217_360),
        ] {
            assert!(
                matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                    if values.len() == 1 && values[0].value == ConstantValue::Integer(expected)),
                "{sql}",
            );
        }
        assert!(matches!(
            parse_select("SELECT CRC32(NULL)").unwrap().projection,
            Projection::Constants(values)
                if values.len() == 1 && values[0].value == ConstantValue::Null
        ));
        assert!(parse_select("SELECT CRC32('a', 'b')").is_err());
    }
}

#[cfg(test)]
mod constant_throw_if_tests {
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn false_and_null_conditions_yield_zero() {
        for sql in [
            "SELECT throwIf(0)",
            "SELECT throwIf(NULL)",
            "SELECT throwIf(0, 'unused')",
        ] {
            assert!(
                matches!(
                    parse_select(sql).unwrap().projection,
                    Projection::Constants(ref values)
                        if values.len() == 1 && values[0].value == ConstantValue::Integer(0)
                ),
                "{sql}"
            );
        }
    }

    #[test]
    fn true_condition_returns_a_parser_error() {
        assert!(parse_select("SELECT throwIf(1, 'custom guard')").is_err());
        assert!(parse_select("SELECT throwIf(1)").is_err());
        assert!(parse_select("SELECT throwIf(0, 'ok', 3)").is_err());
    }
}

#[cfg(test)]
mod constant_if_tests {
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn respects_integer_truthiness_null_conditions_and_common_types() {
        for (sql, expected) in [
            (
                "SELECT if(1, 'yes', 'no')",
                ConstantValue::Text("yes".into()),
            ),
            ("SELECT if(0, 3, 4)", ConstantValue::Integer(4)),
            (
                "SELECT if(NULL, 'yes', 'no')",
                ConstantValue::Text("no".into()),
            ),
            ("SELECT if(-3, NULL, 4)", ConstantValue::Null),
        ] {
            assert!(
                matches!(
                    parse_select(sql).unwrap().projection,
                    Projection::Constants(ref values)
                        if values.len() == 1 && values[0].value == expected
                ),
                "{sql}"
            );
        }
    }

    #[test]
    fn rejects_type_and_arity_errors() {
        for sql in [
            "SELECT if(1, 1)",
            "SELECT if(1, 2, 'text')",
            "SELECT if(1, 2, 3, 4)",
            "SELECT if('x', 1, 2)",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }
}

#[cfg(test)]
mod cut_url_parameter_single_pass_tests {
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn preserves_order_delimiters_and_fragment_when_removing_keys() {
        for (query, expected) in [
            (
                "SELECT cutURLParameter('https://a.test/p?x=1&y=2&x=3#frag', 'x')",
                "https://a.test/p?y=2#frag",
            ),
            (
                "SELECT cutURLParameter('https://a.test/p?x=1&x=2', 'x')",
                "https://a.test/p",
            ),
            (
                "SELECT cutURLParameter('https://a.test/p?z=1#f', 'x')",
                "https://a.test/p?z=1#f",
            ),
            (
                "SELECT cutURLParameter('https://a.test/p?','x')",
                "https://a.test/p?",
            ),
        ] {
            assert!(
                matches!(
                    parse_select(query).unwrap().projection,
                    Projection::Constants(ref values)
                        if values.len() == 1
                            && values[0].value == ConstantValue::Text(expected.into())
                ),
                "{query}",
            );
        }
    }
}

#[cfg(test)]
mod constant_multi_if_tests {
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn picks_first_true_branch_and_treats_null_conditions_as_false() {
        for (sql, expected) in [
            (
                "SELECT multiIf(0, 'no', 1, 'yes', 'otherwise')",
                ConstantValue::Text("yes".into()),
            ),
            (
                "SELECT multiIf(NULL, 4, 1, 7, 9)",
                ConstantValue::Integer(7),
            ),
            ("SELECT multiIf(0, 4, 0, 7, 9)", ConstantValue::Integer(9)),
            ("SELECT multiIf(1, NULL, 2)", ConstantValue::Null),
        ] {
            assert!(
                matches!(
                    parse_select(sql).unwrap().projection,
                    Projection::Constants(ref values)
                        if values.len() == 1 && values[0].value == expected
                ),
                "{sql}"
            );
        }
    }

    #[test]
    fn rejects_invalid_arity_condition_types_and_branch_types() {
        for sql in [
            "SELECT multiIf(1, 2)",
            "SELECT multiIf(1, 2, 0, 3)",
            "SELECT multiIf('yes', 2, 3)",
            "SELECT multiIf(1, 'yes', 4)",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }
}

#[cfg(test)]
mod mac_oui_parser_reuse_tests {
    use crate::{ConstantValue, Projection, parse_select};

    #[test]
    fn mac_oui_reuses_strict_six_byte_parsing() {
        for (sql, expected) in [
            ("SELECT MACStringToOUI('AA:BB:CC:DD:EE:FF')", 0xAA_BB_CC),
            ("SELECT MACStringToOUI('00:00:00:00:00:01')", 0),
            ("SELECT MACStringToOUI('GG:BB:CC:DD:EE:FF')", 0),
            ("SELECT MACStringToOUI('AA:BB:CC')", 0),
        ] {
            assert!(
                matches!(
                    parse_select(sql).unwrap().projection,
                    Projection::Constants(ref values)
                        if values.len() == 1 && values[0].value == ConstantValue::Integer(expected)
                ),
                "{sql}",
            );
        }
        assert!(parse_select("SELECT MACStringToOUI('AA:BB:CC:DD:EE:FF', 1)").is_err());
    }
}

#[cfg(test)]
mod table_crc32_tests {
    use super::*;

    fn reference(input: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in input {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xEDB8_8320 & (0u32.wrapping_sub(crc & 1)));
            }
        }
        !crc
    }

    #[test]
    fn table_matches_bitwise_reference_for_binary_and_unaligned_buffers() {
        let mut source = vec![0u8; 8192];
        let mut state = 0x9e37_79b9u32;
        for byte in &mut source {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *byte = state as u8;
        }
        for length in [
            0, 1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 63, 127, 255, 1024, 8192,
        ] {
            assert_eq!(crc32_ieee(&source[..length]), reference(&source[..length]));
        }
        for offset in 0..16 {
            assert_eq!(
                crc32_ieee(&source[offset..offset + 4096]),
                reference(&source[offset..offset + 4096])
            );
        }
        assert_eq!(crc32_ieee(b"123456789"), 0xcbf4_3926);
    }
}

#[cfg(test)]
mod common_affix_edit_distance_tests {
    use super::*;

    #[test]
    fn long_common_prefix_and_suffix_do_not_exhaust_the_work_budget() {
        let prefix = "a".repeat(65_536);
        let suffix = "z".repeat(65_536);
        let left = format!("{prefix}X{suffix}");
        let right = format!("{prefix}Y{suffix}");
        assert_eq!(
            bounded_edit_distance(left.as_bytes(), right.as_bytes()).unwrap(),
            1
        );
        assert_eq!(
            bounded_edit_distance(right.as_bytes(), left.as_bytes()).unwrap(),
            1
        );

        let prefix = "🚀".repeat(8192);
        let suffix = "é".repeat(8192);
        let left = format!("{prefix}猫{suffix}");
        let right = format!("{prefix}犬{suffix}");
        assert_eq!(bounded_utf8_edit_distance(&left, &right).unwrap(), 1);
    }

    #[test]
    fn common_edge_trimming_preserves_insertions_and_deletions() {
        for (left, right, distance) in [
            ("abcdef", "abcXdef", 1),
            ("abcXdef", "abcdef", 1),
            ("kitten", "sitting", 3),
            ("", "three", 5),
            ("foobarbaz", "foobaz", 3),
            ("aaaa", "aaa", 1),
        ] {
            assert_eq!(
                bounded_edit_distance(left.as_bytes(), right.as_bytes()).unwrap(),
                distance
            );
            assert_eq!(bounded_utf8_edit_distance(left, right).unwrap(), distance);
        }
    }

    #[test]
    fn completely_different_large_strings_keep_the_work_limit() {
        let left = "x".repeat(2048);
        let right = "y".repeat(2048);
        assert!(bounded_edit_distance(left.as_bytes(), right.as_bytes()).is_err());
        assert!(bounded_utf8_edit_distance(&left, &right).is_err());
    }
}

#[cfg(test)]
mod utf8_edit_distance_pretrim_tests {
    use super::bounded_utf8_edit_distance;

    #[test]
    fn common_multibyte_edges_are_trimmed_without_changing_distance() {
        for (left, right, expected) in [
            ("🎉éabc界", "🎉éaxc界", 1),
            ("🎉éabc界", "🎉éab界", 1),
            ("🎉éabc界", "🎉éabcd界", 1),
            ("🙂🙂", "🙂", 1),
            ("é", "e", 1),
            ("e\u{301}", "é", 2),
            ("hello", "world", 4),
        ] {
            assert_eq!(bounded_utf8_edit_distance(left, right).unwrap(), expected);
            assert_eq!(bounded_utf8_edit_distance(right, left).unwrap(), expected);
        }
    }

    #[test]
    fn long_identical_edges_leave_only_small_edits_to_allocate() {
        let prefix = "🎉".repeat(100_000);
        let suffix = "界".repeat(100_000);
        let left = format!("{prefix}a{suffix}");
        let right = format!("{prefix}b{suffix}");
        assert_eq!(bounded_utf8_edit_distance(&left, &right).unwrap(), 1);
    }
}

#[cfg(test)]
mod port_ipv6_authority_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            Projection::Literal(number) => ConstantValue::Integer(number),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn port_requires_brackets_for_ipv6_authorities() {
        for (sql, expected) in [
            ("SELECT port('http://2001:db8::80/')", 0),
            ("SELECT port('http://2001:db8::80/', 1234)", 1234),
            ("SELECT port('http://[2001:db8::1]:8123/path')", 8123),
            ("SELECT port('http://[2001:db8::1]/path', 2345)", 2345),
            ("SELECT port('http://example.com:8123/path')", 8123),
            ("SELECT port('example.com:80:90', 1234)", 1234),
            ("SELECT port('http://user:pass@example.com:443/')", 443),
            ("SELECT port('http://[::1]:0/')", 0),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
    }
}

#[cfg(test)]
mod utf8_prefix_suffix_inplace_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn text(sql: &str) -> String {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(columns) if columns.len() == 1 => {
                let ConstantValue::Text(value) = &columns[0].value else {
                    panic!("expected text")
                };
                value.clone()
            }
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn left_utf8_counts_codepoints_not_bytes() {
        for (count, expected) in [(0, ""), (1, "é"), (2, "é🙂"), (3, "é🙂Z"), (9, "é🙂Z")]
        {
            let sql = format!("SELECT leftUTF8('é🙂Z', {count})");
            assert_eq!(text(&sql), expected);
        }
    }

    #[test]
    fn right_utf8_scans_only_the_suffix_and_preserves_unicode() {
        for (count, expected) in [
            (0, ""),
            (1, "Z"),
            (2, "🙂Z"),
            (3, "é🙂Z"),
            (4, "aé🙂Z"),
            (9, "aé🙂Z"),
        ] {
            let sql = format!("SELECT rightUTF8('aé🙂Z', {count})");
            assert_eq!(text(&sql), expected);
        }
        let input = format!("{}🙂Z", "a".repeat(10_000));
        assert_eq!(text(&format!("SELECT rightUTF8('{input}', 2)")), "🙂Z");
    }
}

#[cfg(test)]
mod clickhouse_to_decimal_string_parity_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn formats_signed_integers_at_requested_scale() {
        for (sql, expected) in [
            ("SELECT toDecimalString(5, 3)", "5.000"),
            ("SELECT toDecimalString(-7, 2)", "-7.00"),
            ("SELECT toDecimalString(0, 0)", "0"),
            ("SELECT toDecimalString(-1, 0)", "-1"),
            ("SELECT toDecimalString(12, 1)", "12.0"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        let high_precision = match value("SELECT toDecimalString(3, 77)") {
            ConstantValue::Text(value) => value,
            other => panic!("unexpected value: {other:?}"),
        };
        assert_eq!(high_precision, format!("3.{}", "0".repeat(77)));
        assert_eq!(
            value("SELECT toDecimalString(NULL, 2)"),
            ConstantValue::Null
        );
        assert_eq!(
            value("SELECT toDecimalString(4, NULL)"),
            ConstantValue::Null
        );
        assert!(parse_select("SELECT toDecimalString(4)").is_err());
        assert!(parse_select("SELECT toDecimalString(4, -1)").is_err());
        assert!(parse_select("SELECT toDecimalString(4, 78)").is_err());
    }
}

#[cfg(test)]
mod clickhouse_has_token_or_null_parity_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn matches_tokens_and_returns_null_for_invalid_needles() {
        assert_eq!(
            value("SELECT hasTokenOrNull('hello, clickhouse!', 'clickhouse')"),
            ConstantValue::Integer(1),
        );
        assert_eq!(
            value("SELECT hasTokenOrNull('hello, clickhouse!', 'click')"),
            ConstantValue::Integer(0),
        );
        assert_eq!(
            value("SELECT hasTokenOrNull('foo_bar', 'foo_bar')"),
            ConstantValue::Integer(1),
        );
        for query in [
            "SELECT hasTokenOrNull('abc', '!!!')",
            "SELECT hasTokenOrNull('abc', 'a b')",
            "SELECT hasTokenOrNull('abc', '')",
            "SELECT hasTokenOrNull(NULL, 'abc')",
            "SELECT hasTokenOrNull('abc', NULL)",
        ] {
            assert_eq!(value(query), ConstantValue::Null, "{query}");
        }
        assert!(parse_select("SELECT hasTokenOrNull('a')").is_err());
        assert!(parse_select("SELECT hasToken('a', '!!!')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_ends_with_case_insensitive_utf8_parity_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn matches_unicode_suffixes_and_empty_needle() {
        for (sql, expected) in [
            ("SELECT endsWithCaseInsensitiveUTF8('данных', 'ЫХ')", 1),
            ("SELECT endsWithCaseInsensitiveUTF8('Äpfel', 'FEL')", 1),
            (
                "SELECT endsWithCaseInsensitiveUTF8('ClickHouse', 'HOUSE')",
                1,
            ),
            (
                "SELECT endsWithCaseInsensitiveUTF8('ClickHouse', 'click')",
                0,
            ),
            ("SELECT endsWithCaseInsensitiveUTF8('Ёжик', '')", 1),
            ("SELECT endsWithCaseInsensitiveUTF8('', '')", 1),
            ("SELECT endsWithCaseInsensitiveUTF8('', 'a')", 0),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(
            value("SELECT endsWithCaseInsensitiveUTF8(NULL, 'a')"),
            ConstantValue::Null
        );
        assert!(parse_select("SELECT endsWithCaseInsensitiveUTF8('a')").is_err());
        assert!(parse_select("SELECT endsWithCaseInsensitiveUTF8('a', 'b', 'c')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_starts_with_case_insensitive_utf8_parity_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn matches_unicode_prefixes_and_empty_needle() {
        for (sql, expected) in [
            (
                "SELECT startsWithCaseInsensitiveUTF8('приставка', 'ПРИ')",
                1,
            ),
            ("SELECT startsWithCaseInsensitiveUTF8('Äpfel', 'äP')", 1),
            (
                "SELECT startsWithCaseInsensitiveUTF8('ClickHouse', 'cli')",
                1,
            ),
            (
                "SELECT startsWithCaseInsensitiveUTF8('ClickHouse', 'house')",
                0,
            ),
            ("SELECT startsWithCaseInsensitiveUTF8('Ёжик', '')", 1),
            ("SELECT startsWithCaseInsensitiveUTF8('', '')", 1),
            ("SELECT startsWithCaseInsensitiveUTF8('', 'a')", 0),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(
            value("SELECT startsWithCaseInsensitiveUTF8(NULL, 'a')"),
            ConstantValue::Null
        );
        assert!(parse_select("SELECT startsWithCaseInsensitiveUTF8('a')").is_err());
        assert!(parse_select("SELECT startsWithCaseInsensitiveUTF8('a', 'b', 'c')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_has_subsequence_case_insensitive_utf8_parity_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn matches_case_insensitive_unicode_subsequences() {
        for (sql, expected) in [
            (
                "SELECT hasSubsequenceCaseInsensitiveUTF8('не прислоняться', 'не слонЯ')",
                1,
            ),
            ("SELECT hasSubsequenceCaseInsensitiveUTF8('Äpfel', 'äe')", 1),
            ("SELECT hasSubsequenceCaseInsensitiveUTF8('ДОБРО', 'до')", 1),
            ("SELECT hasSubsequenceCaseInsensitiveUTF8('AbCd', 'dc')", 0),
            ("SELECT hasSubsequenceCaseInsensitiveUTF8('AbCd', '')", 1),
            ("SELECT hasSubsequenceCaseInsensitiveUTF8('', 'a')", 0),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(
            value("SELECT hasSubsequenceCaseInsensitiveUTF8(NULL, 'a')"),
            ConstantValue::Null
        );
        assert_eq!(
            value("SELECT hasSubsequenceCaseInsensitiveUTF8('a', NULL)"),
            ConstantValue::Null
        );
        assert!(parse_select("SELECT hasSubsequenceCaseInsensitiveUTF8('a')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_has_subsequence_utf8_parity_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn matches_ordered_unicode_codepoints() {
        for (sql, expected) in [
            (
                "SELECT hasSubsequenceUTF8('ReplicatedMergeTree', 'RepMT')",
                1,
            ),
            ("SELECT hasSubsequenceUTF8('a💫bç', 'aç')", 1),
            ("SELECT hasSubsequenceUTF8('a💫bç', 'ça')", 0),
            ("SELECT hasSubsequenceUTF8('Москва', 'Мса')", 1),
            ("SELECT hasSubsequenceUTF8('a💫b', '')", 1),
            ("SELECT hasSubsequenceUTF8('', 'a')", 0),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(
            value("SELECT hasSubsequenceUTF8(NULL, 'a')"),
            ConstantValue::Null
        );
        assert_eq!(
            value("SELECT hasSubsequenceUTF8('a', NULL)"),
            ConstantValue::Null
        );
        assert!(parse_select("SELECT hasSubsequenceUTF8('a')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_compare_substrings_parity_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn compares_byte_ranges_and_short_tails() {
        for (sql, expected) in [
            (
                "SELECT compareSubstrings('Saxony', 'Anglo-Saxon', 0, 6, 5)",
                0,
            ),
            ("SELECT compareSubstrings('abc', 'abd')", -1),
            ("SELECT compareSubstrings('abd', 'abc')", 1),
            ("SELECT compareSubstrings('abc', 'abc')", 0),
            ("SELECT compareSubstrings('abc', 'abc', 2, 2, 0)", 0),
            ("SELECT compareSubstrings('abc', 'abc', 99, 99, 7)", 0),
            ("SELECT compareSubstrings('ab', 'abc', 0, 0, 5)", -1),
            ("SELECT compareSubstrings('é', 'é', 1, 1, 1)", 0),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(
            value("SELECT compareSubstrings(NULL, 'abc')"),
            ConstantValue::Null
        );
        assert!(parse_select("SELECT compareSubstrings('abc')").is_err());
        assert!(parse_select("SELECT compareSubstrings('abc', 'def', 1)").is_err());
        assert!(parse_select("SELECT compareSubstrings('abc', 'def', -1, 0, 2)").is_err());
    }
}

#[cfg(test)]
mod clickhouse_reverse_by_separator_parity_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn reverses_dot_slash_and_multibyte_separator_tokens() {
        for (sql, expected) in [
            (
                "SELECT reverseBySeparator('www.google.com')",
                "com.google.www",
            ),
            ("SELECT reverseBySeparator('a/b/c', '/')", "c/b/a"),
            ("SELECT reverseBySeparator('x::y::z', '::')", "z::y::x"),
            ("SELECT reverseBySeparator('.a.b.')", ".b.a."),
            ("SELECT reverseBySeparator('single')", "single"),
            ("SELECT reverseBySeparator('abcde', '')", "edcba"),
            ("SELECT reverseBySeparator('a💡b💡c', '💡')", "c💡b💡a"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(
            value("SELECT reverseBySeparator(NULL)"),
            ConstantValue::Null
        );
        assert_eq!(
            value("SELECT reverseBySeparator('x', NULL)"),
            ConstantValue::Null
        );
        assert!(parse_select("SELECT reverseBySeparator()").is_err());
        assert!(parse_select("SELECT reverseBySeparator('a', '.', 'b')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_encode_url_component_parity_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn encodes_reserved_bytes_and_preserves_nulls() {
        assert_eq!(
            value("SELECT encodeURLComponent('a b/c?d=e')"),
            ConstantValue::Text("a%20b%2Fc%3Fd%3De".into()),
        );
        assert_eq!(
            value("SELECT encodeURLComponent('café')"),
            ConstantValue::Text("caf%C3%A9".into()),
        );
        assert_eq!(
            value("SELECT encodeURLComponent('')"),
            ConstantValue::Text(String::new()),
        );
        assert_eq!(
            value("SELECT encodeURLComponent(NULL)"),
            ConstantValue::Null
        );
        assert!(parse_select("SELECT encodeURLComponent()").is_err());
        assert!(parse_select("SELECT encodeURLComponent('a', 'b')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_position_case_insensitive_utf8_parity_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            Projection::Literal(number) => ConstantValue::Integer(number),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn finds_unicode_case_insensitive_positions() {
        for (sql, expected) in [
            ("SELECT positionCaseInsensitiveUTF8('Äpfel', 'äP')", 1),
            (
                "SELECT positionCaseInsensitiveUTF8('Crème brûlée', 'BRÛ')",
                7,
            ),
            ("SELECT positionCaseInsensitiveUTF8('Привет Мир', 'МИР')", 8),
            ("SELECT positionCaseInsensitiveUTF8('abc abc', 'ABC', 4)", 5),
            ("SELECT positionCaseInsensitiveUTF8('abc', 'A', 0)", 1),
            ("SELECT positionCaseInsensitiveUTF8('abc', '', 3)", 3),
            ("SELECT positionCaseInsensitiveUTF8('', '')", 1),
            ("SELECT positionCaseInsensitiveUTF8('abc', '', 9)", 0),
            ("SELECT positionCaseInsensitiveUTF8('abc', 'z')", 0),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(
            value("SELECT positionCaseInsensitiveUTF8(NULL, 'a')"),
            ConstantValue::Null
        );
        assert_eq!(
            value("SELECT positionCaseInsensitiveUTF8('abc', NULL)"),
            ConstantValue::Null
        );
        assert!(parse_select("SELECT positionCaseInsensitiveUTF8('a')").is_err());
        assert!(parse_select("SELECT positionCaseInsensitiveUTF8('abc', 'a', -1)").is_err());
        assert!(parse_select("SELECT positionCaseInsensitiveUTF8('a', 'b', 1, 2)").is_err());
    }
}

#[cfg(test)]
mod typed_json_exists_selector_tests {

    #[test]
    fn indexes_and_property_names_select_only_their_container_type() {
        for (sql, expected) in [
            (r#"SELECT JSON_EXISTS('{"0":42}', '$[0]')"#, 0),
            (r#"SELECT JSON_EXISTS('[42]', '$.0')"#, 0),
            (r#"SELECT JSON_EXISTS('{"a":{"0":42}}', '$.a[0]')"#, 0),
            (r#"SELECT JSON_EXISTS('{"a":[{"z":0}]}', '$.a[0].z')"#, 1),
            (r#"SELECT JSON_EXISTS('[{"z":0}]', '$[0].z')"#, 1),
            (r#"SELECT JSON_EXISTS('{"a":4}', '$')"#, 1),
        ] {
            let projection = crate::parse_select(sql).unwrap().projection;
            let crate::Projection::Constants(columns) = projection else {
                panic!("expected constant projection");
            };
            assert_eq!(
                columns[0].value,
                crate::ConstantValue::Integer(expected),
                "{sql}"
            );
        }
    }
}

#[cfg(test)]
mod clickhouse_json_array_length_tests {
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            Projection::Literal(number) => ConstantValue::Integer(number),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn returns_length_only_for_valid_arrays() {
        for (sql, expected) in [
            (
                r#"SELECT JSONArrayLength('[1, {"a":2}, null]')"#,
                ConstantValue::Integer(3),
            ),
            (r#"SELECT JSONArrayLength('[]')"#, ConstantValue::Integer(0)),
            (
                r#"SELECT JSONArrayLength('[[], [1, 2]]')"#,
                ConstantValue::Integer(2),
            ),
            (r#"SELECT JSONArrayLength('{}')"#, ConstantValue::Null),
            (r#"SELECT JSONArrayLength('true')"#, ConstantValue::Null),
            (r#"SELECT JSONArrayLength('invalid')"#, ConstantValue::Null),
            (r#"SELECT JSONArrayLength(NULL)"#, ConstantValue::Null),
        ] {
            assert_eq!(value(sql), expected, "{sql}");
        }
    }

    #[test]
    fn rejects_wrong_argument_count() {
        assert!(parse_select("SELECT JSONArrayLength()").is_err());
        assert!(parse_select("SELECT JSONArrayLength('[]', 'path')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_regexp_quote_meta_tests {
    use super::*;
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            Projection::Literal(number) => ConstantValue::Integer(number),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn escapes_clickhouse_regex_metacharacters() {
        assert_eq!(
            regexp_quote_meta("a.b+c|x(y)[z]^-:{*}?").unwrap(),
            r"a\.b\+c\|x\(y\)\[z\]\^\-\:\{\*}\?"
        );
        assert_eq!(regexp_quote_meta("hello é 東京").unwrap(), "hello é 東京");
        assert_eq!(regexp_quote_meta("\0").unwrap(), r"\0");
        assert_eq!(regexp_quote_meta("\\").unwrap(), r"\\");
    }

    #[test]
    fn accepts_one_nullable_constant_string() {
        assert_eq!(
            value("SELECT regexpQuoteMeta('a.b+c')"),
            ConstantValue::Text(r"a\.b\+c".into())
        );
        assert_eq!(value("SELECT regexpQuoteMeta(NULL)"), ConstantValue::Null);
        assert!(parse_select("SELECT regexpQuoteMeta()").is_err());
        assert!(parse_select("SELECT regexpQuoteMeta('a', 'b')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_base58_encode_tests {
    use super::*;
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            Projection::Literal(number) => ConstantValue::Integer(number),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn encodes_bitcoin_alphabet_with_leading_zero_bytes() {
        assert_eq!(base58_encode("").unwrap(), "");
        assert_eq!(base58_encode("Hello").unwrap(), "9Ajdvzr");
        assert_eq!(base58_encode("Encoded").unwrap(), "3dc8KtHrwM");
        assert_eq!(base58_encode("\0\0A").unwrap(), "1128");
        assert_eq!(base58_encode("\0\0").unwrap(), "11");
    }

    #[test]
    fn handles_nullable_sql_constant_and_input_bound() {
        assert_eq!(
            value("SELECT base58Encode('Hello')"),
            ConstantValue::Text("9Ajdvzr".into())
        );
        assert_eq!(value("SELECT base58Encode(NULL)"), ConstantValue::Null);
        assert!(parse_select("SELECT base58Encode()").is_err());
        assert!(parse_select("SELECT base58Encode('a', 'b')").is_err());
        assert!(base58_encode(&"a".repeat(4097)).is_err());
    }
}

#[cfg(test)]
mod clickhouse_base58_decode_tests {
    use super::*;
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            Projection::Literal(number) => ConstantValue::Integer(number),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn decodes_bitcoin_base58_and_preserves_leading_zero_bytes() {
        assert_eq!(base58_decode("").unwrap(), "");
        assert_eq!(base58_decode("9Ajdvzr").unwrap(), "Hello");
        assert_eq!(base58_decode("3dc8KtHrwM").unwrap(), "Encoded");
        assert_eq!(base58_decode("1128").unwrap(), "\0\0A");
        assert_eq!(base58_decode("11").unwrap(), "\0\0");
        assert_eq!(base58_decode("2").unwrap(), "\u{0001}");
    }

    #[test]
    fn validates_alphabet_utf8_and_argument_count() {
        for invalid in ["0", "O", "I", "l", "invalid"] {
            assert!(base58_decode(invalid).is_err(), "{invalid}");
        }
        assert!(base58_decode("5Q").is_err());
        assert!(base58_decode(&"z".repeat(4097)).is_err());
        assert_eq!(
            value("SELECT base58Decode('9Ajdvzr')"),
            ConstantValue::Text("Hello".into())
        );
        assert_eq!(value("SELECT base58Decode(NULL)"), ConstantValue::Null);
        assert!(parse_select("SELECT base58Decode()").is_err());
        assert!(parse_select("SELECT base58Decode('1', '2')").is_err());
    }
}

#[cfg(test)]
mod clickhouse_unhex_tests {
    use super::*;
    use crate::{ConstantValue, Projection, parse_select};

    fn value(sql: &str) -> ConstantValue {
        match parse_select(sql).unwrap().projection {
            Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
            Projection::Literal(number) => ConstantValue::Integer(number),
            other => panic!("unexpected projection: {other:?}"),
        }
    }

    #[test]
    fn decodes_even_and_odd_hex_digit_counts() {
        assert_eq!(unhex_text("").unwrap(), "");
        assert_eq!(unhex_text("6869").unwrap(), "hi");
        assert_eq!(unhex_text("61A").unwrap(), "a\n");
        assert_eq!(unhex_text("4").unwrap(), "\u{0004}");
        assert_eq!(unhex_text("00").unwrap(), "\0");
        assert_eq!(unhex_text("C3A9").unwrap(), "é");
    }

    #[test]
    fn rejects_malformed_or_non_utf8_text_constants() {
        assert!(unhex_text("G0").is_err());
        assert!(unhex_text("aé").is_err());
        assert!(unhex_text("FF").is_err());
        assert_eq!(
            value("SELECT unhex('6869')"),
            ConstantValue::Text("hi".into())
        );
        assert_eq!(value("SELECT unhex(NULL)"), ConstantValue::Null);
        assert!(parse_select("SELECT unhex()").is_err());
        assert!(parse_select("SELECT unhex('61', '62')").is_err());
    }
}
