mod evaluate;
mod helpers;
use self::evaluate::parse_at_depth;
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
#[path = "text_scalar_tests/clickhouse_readable_time_delta_tests.rs"]
mod clickhouse_readable_time_delta_tests;

#[cfg(test)]
#[path = "text_scalar_tests/string_bytes_uniq_bitmap_tests.rs"]
mod string_bytes_uniq_bitmap_tests;

#[cfg(test)]
#[path = "text_scalar_tests/ch_position_byte_tests.rs"]
mod ch_position_byte_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_decode_url_form_tests.rs"]
mod clickhouse_decode_url_form_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_ipv4_to_num_or_null_tests.rs"]
mod clickhouse_ipv4_to_num_or_null_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_ipv4_string_to_num_tests.rs"]
mod clickhouse_ipv4_string_to_num_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_ipv4_num_to_string_tests.rs"]
mod clickhouse_ipv4_num_to_string_tests;

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
#[path = "text_scalar_tests/clickhouse_cut_url_parameter_tests.rs"]
mod clickhouse_cut_url_parameter_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_domain_without_www_tests.rs"]
mod clickhouse_domain_without_www_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_cut_www_tests.rs"]
mod clickhouse_cut_www_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_ipv4_string_default_tests.rs"]
mod clickhouse_ipv4_string_default_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_ipv4_class_c_tests.rs"]
mod clickhouse_ipv4_class_c_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_mac_string_to_num_tests.rs"]
mod clickhouse_mac_string_to_num_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_mac_num_to_string_tests.rs"]
mod clickhouse_mac_num_to_string_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_mac_string_to_oui_tests.rs"]
mod clickhouse_mac_string_to_oui_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_ip_address_in_range_tests.rs"]
mod clickhouse_ip_address_in_range_tests;

#[cfg(test)]
#[path = "text_scalar_tests/equal_edit_distance_fast_path_tests.rs"]
mod equal_edit_distance_fast_path_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_crc32_tests.rs"]
mod clickhouse_crc32_tests;

#[cfg(test)]
#[path = "text_scalar_tests/constant_throw_if_tests.rs"]
mod constant_throw_if_tests;

#[cfg(test)]
#[path = "text_scalar_tests/constant_if_tests.rs"]
mod constant_if_tests;

#[cfg(test)]
#[path = "text_scalar_tests/cut_url_parameter_single_pass_tests.rs"]
mod cut_url_parameter_single_pass_tests;

#[cfg(test)]
#[path = "text_scalar_tests/constant_multi_if_tests.rs"]
mod constant_multi_if_tests;

#[cfg(test)]
#[path = "text_scalar_tests/mac_oui_parser_reuse_tests.rs"]
mod mac_oui_parser_reuse_tests;

#[cfg(test)]
#[path = "text_scalar_tests/table_crc32_tests.rs"]
mod table_crc32_tests;

#[cfg(test)]
#[path = "text_scalar_tests/common_affix_edit_distance_tests.rs"]
mod common_affix_edit_distance_tests;

#[cfg(test)]
#[path = "text_scalar_tests/utf8_edit_distance_pretrim_tests.rs"]
mod utf8_edit_distance_pretrim_tests;

#[cfg(test)]
#[path = "text_scalar_tests/port_ipv6_authority_tests.rs"]
mod port_ipv6_authority_tests;

#[cfg(test)]
#[path = "text_scalar_tests/utf8_prefix_suffix_inplace_tests.rs"]
mod utf8_prefix_suffix_inplace_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_to_decimal_string_parity_tests.rs"]
mod clickhouse_to_decimal_string_parity_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_has_token_or_null_parity_tests.rs"]
mod clickhouse_has_token_or_null_parity_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_ends_with_case_insensitive_utf8_parity_tests.rs"]
mod clickhouse_ends_with_case_insensitive_utf8_parity_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_starts_with_case_insensitive_utf8_parity_tests.rs"]
mod clickhouse_starts_with_case_insensitive_utf8_parity_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_has_subsequence_case_insensitive_utf8_parity_tests.rs"]
mod clickhouse_has_subsequence_case_insensitive_utf8_parity_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_has_subsequence_utf8_parity_tests.rs"]
mod clickhouse_has_subsequence_utf8_parity_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_compare_substrings_parity_tests.rs"]
mod clickhouse_compare_substrings_parity_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_reverse_by_separator_parity_tests.rs"]
mod clickhouse_reverse_by_separator_parity_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_encode_url_component_parity_tests.rs"]
mod clickhouse_encode_url_component_parity_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_position_case_insensitive_utf8_parity_tests.rs"]
mod clickhouse_position_case_insensitive_utf8_parity_tests;

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
#[path = "text_scalar_tests/clickhouse_base58_encode_tests.rs"]
mod clickhouse_base58_encode_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_base58_decode_tests.rs"]
mod clickhouse_base58_decode_tests;

#[cfg(test)]
#[path = "text_scalar_tests/clickhouse_unhex_tests.rs"]
mod clickhouse_unhex_tests;
