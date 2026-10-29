//! Bounded constant-expression evaluation for scalar SELECT without FROM.
//! Calculation happens in the parser: the executor still sees a single Literal.
use super::{ParseError, Parser, Token};

const MAX_DEPTH: usize = 16;

fn is_extrema(name: &str) -> bool {
    name.eq_ignore_ascii_case("GREATEST") || name.eq_ignore_ascii_case("LEAST")
}

fn is_power(name: &str) -> bool {
    name.eq_ignore_ascii_case("POWER") || name.eq_ignore_ascii_case("POW")
}

fn is_bitwise(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "BITAND"
            | "BITOR"
            | "BITXOR"
            | "BITNAND"
            | "BITNOR"
            | "BITXNOR"
            | "BITSHIFTLEFT"
            | "BITSHIFTRIGHT"
            | "BITROTATELEFT"
            | "BITROTATERIGHT"
    )
}

pub(super) fn is_function(name: &str) -> bool {
    if name.eq_ignore_ascii_case("DIGIT_SUM") {
        return true;
    }
    name.eq_ignore_ascii_case("ISZERO")
        || is_saturating(name)
        || name.eq_ignore_ascii_case("BITHAMMINGDISTANCE")
        || name.eq_ignore_ascii_case("ABS")
        || name.eq_ignore_ascii_case("NEGATE")
        || name.eq_ignore_ascii_case("BIT_PARITY")
        || name.eq_ignore_ascii_case("BIT_MASK")
        || name.eq_ignore_ascii_case("BIT_WIDTH")
        || name.eq_ignore_ascii_case("DIV")
        || name.eq_ignore_ascii_case("INTDIV")
        || name.eq_ignore_ascii_case("MOD")
        || name.eq_ignore_ascii_case("MODULO")
        || name.eq_ignore_ascii_case("ABS_DIFF")
        || name.eq_ignore_ascii_case("MULTIPLY")
        || name.eq_ignore_ascii_case("SIGN")
        || name.eq_ignore_ascii_case("TOBOOL")
        || name.eq_ignore_ascii_case("DIGIT_COUNT")
        || name.eq_ignore_ascii_case("HAMMING_DISTANCE")
        || name.eq_ignore_ascii_case("BITHAMMINGDISTANCE")
        || is_extrema(name)
        || name.eq_ignore_ascii_case("IS_POWER_OF_TWO")
        || name.eq_ignore_ascii_case("BIT_REVERSE")
        || is_power(name)
        || name.eq_ignore_ascii_case("NEXT_POWER_OF_TWO")
        || name.eq_ignore_ascii_case("BIT_EXTRACT")
        || is_bitwise(name)
        || name.eq_ignore_ascii_case("MOD_POW")
        || name.eq_ignore_ascii_case("GCD")
        || name.eq_ignore_ascii_case("REVERSE_DIGITS")
        || name.eq_ignore_ascii_case("LCM")
        || name.eq_ignore_ascii_case("FIBONACCI")
        || name.eq_ignore_ascii_case("IS_DIVISIBLE")
        || name.eq_ignore_ascii_case("IN_RANGE")
        || name.eq_ignore_ascii_case("DIV_EUCLID")
        || name.eq_ignore_ascii_case("REM_EUCLID")
        || name.eq_ignore_ascii_case("MUL_DIV")
        || name.eq_ignore_ascii_case("WIDTH_BUCKET")
        || name.eq_ignore_ascii_case("BIT_SET")
        || name.eq_ignore_ascii_case("BIT_CLEAR")
        || name.eq_ignore_ascii_case("BIT_TOGGLE")
        || name.eq_ignore_ascii_case("LEADING_ZEROS")
        || name.eq_ignore_ascii_case("TRAILING_ZEROS")
        || name.eq_ignore_ascii_case("DIGITAL_ROOT")
        || name.eq_ignore_ascii_case("POW10")
        || name.eq_ignore_ascii_case("TRIANGULAR")
        || name.eq_ignore_ascii_case("IS_PRIME")
        || name.eq_ignore_ascii_case("PERMUTATIONS")
        || name.eq_ignore_ascii_case("BINOMIAL")
        || name.eq_ignore_ascii_case("CEIL_TO_MULTIPLE")
        || name.eq_ignore_ascii_case("CLAMP")
        || name.eq_ignore_ascii_case("ROUND_DIV")
        || name.eq_ignore_ascii_case("CEIL_DIV")
        || name.eq_ignore_ascii_case("FLOOR_DIV")
        || name.eq_ignore_ascii_case("BITCOUNT")
        || name.eq_ignore_ascii_case("BITNOT")
        || name.eq_ignore_ascii_case("BITTEST")
        || name.eq_ignore_ascii_case("ISQRT")
        || name.eq_ignore_ascii_case("FACTORIAL")
        || name.eq_ignore_ascii_case("INTLOG")
        || name.eq_ignore_ascii_case("INTLOG2")
        || name.eq_ignore_ascii_case("FLOOR_TO_MULTIPLE")
        || name.eq_ignore_ascii_case("ROUND_TO_MULTIPLE")
        || name.eq_ignore_ascii_case("PMOD")
        || name.eq_ignore_ascii_case("TRAILING_ONES")
        || name.eq_ignore_ascii_case("TOINT8")
}

pub(super) fn parse(parser: &mut Parser) -> Result<i64, ParseError> {
    expression(parser, 0)
}

fn expression(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let mut value = term(parser, depth)?;
    loop {
        if parser.take(Token::Plus) {
            value = value
                .checked_add(term(parser, depth)?)
                .ok_or_else(overflow)?;
        } else if parser.take(Token::Minus) {
            value = value
                .checked_sub(term(parser, depth)?)
                .ok_or_else(overflow)?;
        } else {
            break;
        }
    }
    Ok(value)
}

fn term(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let mut value = factor(parser, depth)?;
    loop {
        if parser.take(Token::Star) {
            value = value
                .checked_mul(factor(parser, depth)?)
                .ok_or_else(overflow)?;
        } else if parser.take(Token::Slash) {
            value = value
                .checked_div(factor(parser, depth)?)
                .ok_or(ParseError("scalar division by zero or overflow".into()))?;
        } else if parser.take(Token::Percent) {
            let divisor = factor(parser, depth)?;
            value = if divisor == -1 {
                0
            } else {
                value.checked_rem(divisor)
                    .ok_or(ParseError("scalar modulo by zero or overflow".into()))?
            };
        } else {
            break;
        }
    }
    Ok(value)
}

fn require_token(parser: &mut Parser, token: Token, message: &str) -> Result<(), ParseError> {
    if parser.take(token) {
        Ok(())
    } else {
        Err(ParseError(message.into()))
    }
}

fn is_saturating(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "SATURATING_ADD" | "SATURATING_SUB" | "SATURATING_MUL"
    )
}

fn parse_saturating(parser: &mut Parser, depth: usize, name: &str) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "saturating arithmetic requires '('")?;
    let left = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::Comma,
        "saturating arithmetic requires two arguments",
    )?;
    let right = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "expected ')' after saturating arguments",
    )?;
    Ok(match name.to_ascii_uppercase().as_str() {
        "SATURATING_ADD" => left.saturating_add(right),
        "SATURATING_SUB" => left.saturating_sub(right),
        "SATURATING_MUL" => left.saturating_mul(right),
        _ => unreachable!("only saturating functions reach this branch"),
    })
}

fn parse_negate(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "negate requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "expected ')' after negate argument")?;
    value.checked_neg().ok_or_else(overflow)
}

fn parse_power(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "POWER requires '('")?;
    let base = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "POWER requires two arguments")?;
    let exponent = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "POWER requires ')' after exponent")?;
    let exponent = u32::try_from(exponent)
        .map_err(|_| ParseError("POWER exponent must be non-negative and fit UInt32".into()))?;
    base.checked_pow(exponent).ok_or_else(overflow)
}

/// SQL-style truncating integer division and remainder.
fn parse_div_mod(parser: &mut Parser, depth: usize, divide: bool) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "DIV/MOD requires '('")?;
    let left = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "DIV/MOD requires two arguments")?;
    let right = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "DIV/MOD requires ')'")?;
    if divide {
        left.checked_div(right)
            .ok_or_else(|| ParseError("DIV divisor is zero or result overflows Int64".into()))
    } else {
        if right == -1 {
            return Ok(0); // Int64::MIN % -1 is mathematically zero.
        }
        left.checked_rem(right)
            .ok_or_else(|| ParseError("MOD divisor is zero or result overflows Int64".into()))
    }
}

fn parse_pmod(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "PMOD requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "PMOD requires two arguments")?;
    let divisor = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "expected ')' after PMOD arguments")?;
    if divisor <= 0 {
        return Err(ParseError("PMOD divisor must be positive".into()));
    }
    Ok(value.rem_euclid(divisor))
}

fn parse_multiply_function(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "multiply requires '('")?;
    let left = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "multiply requires two arguments")?;
    let right = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "expected ')' after multiply arguments")?;
    left.checked_mul(right).ok_or_else(overflow)
}

fn parse_abs_diff(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "ABS_DIFF requires '('")?;
    let left = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "ABS_DIFF requires two arguments")?;
    let right = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "ABS_DIFF requires ')' after arguments",
    )?;
    let difference = (i128::from(left) - i128::from(right)).abs();
    i64::try_from(difference).map_err(|_| overflow())
}

fn parse_extrema(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let name = parser.word()?;
    if !parser.take(Token::LParen) {
        return Err(ParseError(format!("{name} requires '('")));
    }
    let mut value = expression(parser, depth + 1)?;
    if !parser.take(Token::Comma) {
        return Err(ParseError(format!(
            "{name} requires at least two arguments"
        )));
    }
    let mut count = 1usize;
    loop {
        if count == 16 {
            return Err(ParseError(format!("{name} supports at most 16 arguments")));
        }
        let candidate = expression(parser, depth + 1)?;
        value = if name.eq_ignore_ascii_case("GREATEST") {
            value.max(candidate)
        } else {
            value.min(candidate)
        };
        count += 1;
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if !parser.take(Token::RParen) {
        return Err(ParseError(format!("expected ')' after {name} arguments")));
    }
    Ok(value)
}

fn parse_bit_width(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "BIT_WIDTH requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "expected ')' after BIT_WIDTH argument",
    )?;
    Ok(i64::from(u64::BITS - value.unsigned_abs().leading_zeros()))
}

/// Parity of all 64 bits in the signed two's-complement representation.
fn parse_bit_parity(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "BIT_PARITY requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "BIT_PARITY requires ')'")?;
    Ok(i64::from(value.count_ones() & 1))
}

/// Set the lowest 0 to 64 bits in a signed Int64 bit pattern.
fn parse_bit_mask(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "BIT_MASK requires '('")?;
    let width = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "BIT_MASK requires ')'")?;
    let width = u32::try_from(width)
        .map_err(|_| ParseError("BIT_MASK width must be between 0 and 64".into()))?;
    if width > 64 {
        return Err(ParseError("BIT_MASK width must be between 0 and 64".into()));
    }
    Ok(if width == 64 {
        -1
    } else {
        ((1_u64 << width) - 1) as i64
    })
}

fn parse_hamming_distance(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "HAMMING_DISTANCE requires '('")?;
    let left = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::Comma,
        "HAMMING_DISTANCE requires two arguments",
    )?;
    let right = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "HAMMING_DISTANCE requires ')' after arguments",
    )?;
    Ok(i64::from((left ^ right).count_ones()))
}

fn checked_bit_shift(value: i64, amount: i64, left: bool) -> Result<i64, ParseError> {
    let shift = u32::try_from(amount)
        .map_err(|_| ParseError("bit shift count must be between 0 and 63".into()))?;
    if shift >= i64::BITS {
        return Err(ParseError(
            "bit shift count must be between 0 and 63".into(),
        ));
    }
    if left {
        i64::try_from(i128::from(value) << shift).map_err(|_| overflow())
    } else {
        Ok(value >> shift)
    }
}

fn checked_bit_rotate(value: i64, amount: i64, left: bool) -> Result<i64, ParseError> {
    let shift = u32::try_from(amount)
        .map_err(|_| ParseError("bit rotate count must be between 0 and 63".into()))?;
    if shift >= i64::BITS {
        return Err(ParseError(
            "bit rotate count must be between 0 and 63".into(),
        ));
    }
    Ok(if left {
        value.rotate_left(shift)
    } else {
        value.rotate_right(shift)
    })
}

fn parse_bit_reverse(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "BIT_REVERSE requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "BIT_REVERSE requires ')' after argument",
    )?;
    Ok(value.reverse_bits())
}

fn parse_clamp(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "CLAMP requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "CLAMP requires three arguments")?;
    let lower = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "CLAMP requires three arguments")?;
    let upper = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "expected ')' after CLAMP arguments")?;
    if lower > upper {
        return Err(ParseError("CLAMP lower bound exceeds upper bound".into()));
    }
    Ok(value.clamp(lower, upper))
}

/// Round a signed integer upward to a positive step.
fn parse_ceil_to_multiple(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "CEIL_TO_MULTIPLE requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::Comma,
        "CEIL_TO_MULTIPLE requires two arguments",
    )?;
    let multiple = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "CEIL_TO_MULTIPLE requires ')'")?;
    if multiple <= 0 {
        return Err(ParseError(
            "CEIL_TO_MULTIPLE requires a positive multiple".into(),
        ));
    }
    let value = i128::from(value);
    let multiple = i128::from(multiple);
    let mut quotient = value.div_euclid(multiple);
    if value.rem_euclid(multiple) != 0 {
        quotient += 1;
    }
    i64::try_from(quotient * multiple).map_err(|_| overflow())
}

fn parse_digit_count(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "DIGIT_COUNT requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "expected ')' after DIGIT_COUNT argument",
    )?;
    let magnitude = value.unsigned_abs();
    let digits = if magnitude == 0 {
        1
    } else {
        magnitude.ilog10() + 1
    };
    Ok(i64::from(digits))
}

fn parse_round_to_multiple(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "ROUND_TO_MULTIPLE requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::Comma,
        "ROUND_TO_MULTIPLE requires two arguments",
    )?;
    let multiple = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "expected ')' after ROUND_TO_MULTIPLE arguments",
    )?;
    if multiple <= 0 {
        return Err(ParseError(
            "ROUND_TO_MULTIPLE requires a positive multiple".into(),
        ));
    }
    let step = i128::from(multiple);
    let rounded_magnitude = ((i128::from(value).abs() + step / 2) / step) * step;
    let rounded = if value < 0 {
        -rounded_magnitude
    } else {
        rounded_magnitude
    };
    i64::try_from(rounded).map_err(|_| overflow())
}

/// Round down to the next multiple, including negative values.
fn parse_floor_to_multiple(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "FLOOR_TO_MULTIPLE requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::Comma,
        "FLOOR_TO_MULTIPLE requires two arguments",
    )?;
    let multiple = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "FLOOR_TO_MULTIPLE requires ')'")?;
    if multiple <= 0 {
        return Err(ParseError(
            "FLOOR_TO_MULTIPLE requires a positive multiple".into(),
        ));
    }
    let value = i128::from(value);
    let multiple = i128::from(multiple);
    i64::try_from(value.div_euclid(multiple) * multiple).map_err(|_| overflow())
}

fn parse_bit_extract(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "BIT_EXTRACT requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "BIT_EXTRACT requires three arguments")?;
    let offset = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "BIT_EXTRACT requires three arguments")?;
    let width = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "BIT_EXTRACT requires ')' after arguments",
    )?;
    let offset = u32::try_from(offset)
        .map_err(|_| ParseError("BIT_EXTRACT offset must be between 0 and 63".into()))?;
    let width = u32::try_from(width)
        .map_err(|_| ParseError("BIT_EXTRACT width must be between 1 and 64".into()))?;
    if offset >= 64 || width == 0 || width > 64 || offset + width > 64 {
        return Err(ParseError(
            "BIT_EXTRACT field must fit within 64 bits".into(),
        ));
    }
    let mask = if width == 64 { u64::MAX } else { (1_u64 << width) - 1 };
    Ok((((value as u64) >> offset) & mask) as i64)
}

/// Return one when a signed integer is between inclusive bounds.
fn parse_in_range(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "IN_RANGE requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "IN_RANGE requires three arguments")?;
    let lower = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "IN_RANGE requires three arguments")?;
    let upper = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "IN_RANGE requires ')'")?;
    if lower > upper {
        return Err(ParseError(
            "IN_RANGE lower bound exceeds upper bound".into(),
        ));
    }
    Ok(i64::from((lower..=upper).contains(&value)))
}

/// Multiply in Int128 before dividing, then require an Int64 result.
/// Euclidean quotient and nonnegative remainder, including negative operands.
fn parse_euclidean_division(
    parser: &mut Parser,
    depth: usize,
    quotient: bool,
) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "Euclidean division requires '('")?;
    let left = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::Comma,
        "Euclidean division needs two arguments",
    )?;
    let right = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "Euclidean division requires ')'")?;
    let result = if quotient {
        left.checked_div_euclid(right)
    } else if right == -1 {
        Some(0) // The Euclidean remainder is representable for Int64::MIN.
    } else {
        left.checked_rem_euclid(right)
    };
    result.ok_or_else(|| ParseError("Euclidean division by zero or overflow".into()))
}

fn parse_mul_div(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "MUL_DIV requires '('")?;
    let left = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "MUL_DIV requires three arguments")?;
    let right = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "MUL_DIV requires three arguments")?;
    let divisor = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "MUL_DIV requires ')'")?;
    let numerator = i128::from(left) * i128::from(right);
    let quotient = numerator
        .checked_div(i128::from(divisor))
        .ok_or_else(|| ParseError("MUL_DIV divisor cannot be zero".into()))?;
    i64::try_from(quotient).map_err(|_| overflow())
}

/// Equal-width histogram buckets for a signed range [lower, upper).
/// Return one if the signed divisor divides the dividend exactly.
fn parse_is_divisible(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "IS_DIVISIBLE requires '('")?;
    let dividend = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "IS_DIVISIBLE requires two arguments")?;
    let divisor = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "IS_DIVISIBLE requires ')'")?;
    if divisor == 0 {
        return Err(ParseError("IS_DIVISIBLE divisor cannot be zero".into()));
    }
    Ok(i64::from(i128::from(dividend) % i128::from(divisor) == 0))
}

fn parse_width_bucket(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "WIDTH_BUCKET requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "WIDTH_BUCKET requires four arguments")?;
    let lower = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "WIDTH_BUCKET requires four arguments")?;
    let upper = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "WIDTH_BUCKET requires four arguments")?;
    let count = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "WIDTH_BUCKET requires ')'")?;
    if lower >= upper {
        return Err(ParseError("WIDTH_BUCKET requires min < max".into()));
    }
    if !(1..=1_000_000).contains(&count) {
        return Err(ParseError(
            "WIDTH_BUCKET bucket count must be between 1 and 1000000".into(),
        ));
    }
    if value < lower {
        return Ok(0);
    }
    if value >= upper {
        return Ok(count + 1);
    }
    let span = i128::from(upper) - i128::from(lower);
    let offset = i128::from(value) - i128::from(lower);
    let bucket = offset * i128::from(count) / span + 1;
    i64::try_from(bucket).map_err(|_| overflow())
}

/// Modify a single bit in the signed 64-bit two's-complement representation.
fn parse_bit_update(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let name = parser.word()?;
    require_token(parser, Token::LParen, "bit update requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "bit update requires two arguments")?;
    let index = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "bit update requires ')'")?;
    let index = u32::try_from(index)
        .map_err(|_| ParseError("bit index must be between 0 and 63".into()))?;
    if index >= i64::BITS {
        return Err(ParseError("bit index must be between 0 and 63".into()));
    }
    let mask = 1_u64 << index;
    let bits = value as u64;
    let result = match name.to_ascii_uppercase().as_str() {
        "BIT_SET" => bits | mask,
        "BIT_CLEAR" => bits & !mask,
        "BIT_TOGGLE" => bits ^ mask,
        _ => unreachable!("only bit-update functions reach this parser"),
    };
    Ok(result as i64)
}

/// Count zero bits on the left or right of an Int64 bit pattern.
fn parse_bit_scans(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let name = parser.word()?;
    require_token(parser, Token::LParen, "bit scan requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "bit scan requires ')'")?;
    let result = if name.eq_ignore_ascii_case("LEADING_ZEROS") {
        value.leading_zeros()
    } else {
        value.trailing_zeros()
    };
    Ok(i64::from(result))
}

/// Decimal digital root of the unsigned magnitude; zero maps to zero.
fn parse_digital_root(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "DIGITAL_ROOT requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "DIGITAL_ROOT requires ')'")?;
    let magnitude = value.unsigned_abs();
    if magnitude == 0 {
        return Ok(0);
    }
    let root = 1 + (magnitude - 1) % 9;
    i64::try_from(root).map_err(|_| overflow())
}

/// Checked signed Int64 power of ten for nonnegative integer exponents.
fn parse_pow10(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "POW10 requires '('")?;
    let exponent = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "POW10 requires ')'")?;
    let exponent = u32::try_from(exponent)
        .map_err(|_| ParseError("POW10 exponent must be non-negative and fit UInt32".into()))?;
    10_i64.checked_pow(exponent).ok_or_else(overflow)
}

/// Compute the triangular number with a wide intermediate product.
/// Integer floor(log_base(value)), without floating-point precision loss.
fn parse_intlog(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "INTLOG requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "INTLOG requires two arguments")?;
    let base = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "INTLOG requires ')'")?;
    if value <= 0 || base < 2 {
        return Err(ParseError("INTLOG requires value > 0 and base >= 2".into()));
    }
    let mut current = value;
    let mut exponent = 0_i64;
    while current >= base {
        current /= base;
        exponent += 1;
    }
    Ok(exponent)
}

fn parse_triangular(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "TRIANGULAR requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "TRIANGULAR requires ')'")?;
    if value < 0 {
        return Err(ParseError("TRIANGULAR requires a nonnegative value".into()));
    }
    let value = i128::from(value);
    i64::try_from(value * (value + 1) / 2).map_err(|_| overflow())
}

/// Test primality with bounded 6k +/- 1 trial division.
fn parse_is_prime(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "IS_PRIME requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "IS_PRIME requires ')'")?;
    if value < 2 {
        return Ok(0);
    }
    if value > 1_000_000_000_000 {
        return Err(ParseError(
            "IS_PRIME supports values up to 1000000000000".into(),
        ));
    }
    let value = u64::try_from(value)
        .map_err(|_| ParseError("IS_PRIME requires a nonnegative integer".into()))?;
    if value % 2 == 0 {
        return Ok(i64::from(value == 2));
    }
    if value % 3 == 0 {
        return Ok(i64::from(value == 3));
    }
    let mut divisor = 5_u64;
    while divisor <= value / divisor {
        if value % divisor == 0 || value % (divisor + 2) == 0 {
            return Ok(0);
        }
        divisor += 6;
    }
    Ok(1)
}

/// Compute n choose k exactly for n up to 66.
fn parse_binomial(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "BINOMIAL requires '('")?;
    let n = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "BINOMIAL requires two arguments")?;
    let k = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "BINOMIAL requires ')'")?;
    if !(0..=66).contains(&n) || k < 0 || k > n {
        return Err(ParseError("BINOMIAL requires 0 <= k <= n <= 66".into()));
    }
    let k = k.min(n - k);
    let mut result = 1_i128;
    for index in 0..k {
        result = result * i128::from(n - index) / i128::from(index + 1);
    }
    i64::try_from(result).map_err(|_| overflow())
}

/// Number of ordered selections nPk for 0 <= k <= n <= 20.
fn parse_permutations(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "PERMUTATIONS requires '('")?;
    let n = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "PERMUTATIONS requires two arguments")?;
    let k = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "PERMUTATIONS requires ')'")?;
    if !(0..=20).contains(&n) || k < 0 || k > n {
        return Err(ParseError("PERMUTATIONS requires 0 <= k <= n <= 20".into()));
    }
    let mut result = 1_i64;
    for offset in 0..k {
        result = result.checked_mul(n - offset).ok_or_else(overflow)?;
    }
    Ok(result)
}

fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn parse_mod_pow(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "MOD_POW requires '('")?;
    let base = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "MOD_POW requires three arguments")?;
    let exponent = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "MOD_POW requires three arguments")?;
    let modulus = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "MOD_POW requires ')' after arguments",
    )?;
    if exponent < 0 || modulus <= 0 {
        return Err(ParseError(
            "MOD_POW exponent must be non-negative and modulus must be positive".into(),
        ));
    }
    let mut factor = base.rem_euclid(modulus) as u128;
    let modulus = modulus as u128;
    let mut power = exponent as u64;
    let mut result = 1_u128 % modulus;
    while power > 0 {
        if power & 1 != 0 {
            result = (result * factor) % modulus;
        }
        factor = (factor * factor) % modulus;
        power >>= 1;
    }
    i64::try_from(result).map_err(|_| overflow())
}

fn parse_gcd_lcm(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let name = parser.word()?;
    if !parser.take(Token::LParen) {
        return Err(ParseError(format!("{name} requires '('")));
    }
    let left = expression(parser, depth + 1)?;
    if !parser.take(Token::Comma) {
        return Err(ParseError(format!("{name} requires two arguments")));
    }
    let right = expression(parser, depth + 1)?;
    if !parser.take(Token::RParen) {
        return Err(ParseError(format!("expected ')' after {name} arguments")));
    }
    let left = left.unsigned_abs();
    let right = right.unsigned_abs();
    let result = if name.eq_ignore_ascii_case("GCD") {
        gcd(left, right)
    } else if left == 0 || right == 0 {
        0
    } else {
        (left / gcd(left, right))
            .checked_mul(right)
            .ok_or_else(overflow)?
    };
    i64::try_from(result).map_err(|_| overflow())
}

fn parse_rounded_division(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let name = parser.word()?;
    require_token(parser, Token::LParen, "rounded division requires '('")?;
    let dividend = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::Comma,
        "rounded division requires two arguments",
    )?;
    let divisor = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "expected ')' after rounded division arguments",
    )?;
    if divisor == 0 {
        return Err(ParseError("rounded division by zero".into()));
    }

    // i128 also handles i64::MIN / -1 without overflowing before validation.
    let dividend = i128::from(dividend);
    let divisor = i128::from(divisor);
    let quotient = dividend / divisor;
    let remainder = dividend % divisor;
    let same_sign = (dividend < 0) == (divisor < 0);
    let result = if remainder == 0 {
        quotient
    } else if name.eq_ignore_ascii_case("CEIL_DIV") && same_sign {
        quotient + 1
    } else if name.eq_ignore_ascii_case("FLOOR_DIV") && !same_sign {
        quotient - 1
    } else {
        quotient
    };
    i64::try_from(result).map_err(|_| overflow())
}

/// Round a signed quotient to nearest, with ties away from zero.
fn parse_round_div(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "ROUND_DIV requires '('")?;
    let dividend = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "ROUND_DIV requires two arguments")?;
    let divisor = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "ROUND_DIV requires ')'")?;
    if divisor == 0 {
        return Err(ParseError("ROUND_DIV divisor cannot be zero".into()));
    }
    let dividend = i128::from(dividend);
    let divisor = i128::from(divisor);
    let rounded = (dividend.abs() + divisor.abs() / 2) / divisor.abs();
    let signed = if (dividend < 0) != (divisor < 0) {
        -rounded
    } else {
        rounded
    };
    i64::try_from(signed).map_err(|_| overflow())
}

fn parse_bit_introspection(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let name = parser.word()?;
    require_token(parser, Token::LParen, "bit function requires '('")?;
    let value = expression(parser, depth + 1)?;
    let result = if name.eq_ignore_ascii_case("BITCOUNT") {
        i64::from(value.count_ones())
    } else if name.eq_ignore_ascii_case("BITNOT") {
        !value
    } else {
        require_token(parser, Token::Comma, "BITTEST requires two arguments")?;
        let index = expression(parser, depth + 1)?;
        let bit = u32::try_from(index)
            .map_err(|_| ParseError("BITTEST position must be between 0 and 63".into()))?;
        if bit >= i64::BITS {
            return Err(ParseError(
                "BITTEST position must be between 0 and 63".into(),
            ));
        }
        i64::from(((value as u64 >> bit) & 1) as u8)
    };
    require_token(parser, Token::RParen, "expected ')' after bit function")?;
    Ok(result)
}

fn parse_is_power_of_two(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "IS_POWER_OF_TWO requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "expected ')' after IS_POWER_OF_TWO argument",
    )?;
    Ok(if value > 0 && value.unsigned_abs().is_power_of_two() {
        1
    } else {
        0
    })
}

fn parse_reverse_digits(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "REVERSE_DIGITS requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "REVERSE_DIGITS requires ')' after argument",
    )?;
    let mut magnitude = value.unsigned_abs();
    let mut reversed = 0_i128;
    while magnitude > 0 {
        reversed = reversed * 10 + i128::from(magnitude % 10);
        magnitude /= 10;
    }
    let signed = if value < 0 { -reversed } else { reversed };
    i64::try_from(signed).map_err(|_| overflow())
}

fn factor(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    if depth >= MAX_DEPTH {
        return Err(ParseError(
            "scalar expressions nest at most 16 levels".into(),
        ));
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if is_saturating(name)) {
        let name = parser.word()?;
        return parse_saturating(parser, depth, &name);
    }
    if parser.keyword("NEGATE") {
        return parse_negate(parser, depth);
    }
    if parser.take(Token::Minus) {
        return factor(parser, depth + 1)?
            .checked_neg()
            .ok_or_else(overflow);
    }
    if parser.keyword("TOBOOL") {
        require_token(parser, Token::LParen, "toBool requires '('")?;
        let value = expression(parser, depth + 1)?;
        require_token(parser, Token::RParen, "toBool requires ')'")?;
        return Ok(i64::from(value != 0));
    }
    if parser.keyword("DIV") || parser.keyword("INTDIV") {
        return parse_div_mod(parser, depth, true);
    }
    if parser.keyword("MOD") || parser.keyword("MODULO") {
        return parse_div_mod(parser, depth, false);
    }
    if parser.keyword("ABS_DIFF") {
        return parse_abs_diff(parser, depth);
    }
    if parser.take(Token::Plus) {
        return factor(parser, depth + 1);
    }
    if parser.keyword("MULTIPLY") {
        return parse_multiply_function(parser, depth);
    }
    if parser.keyword("NEXT_POWER_OF_TWO") {
        return parse_next_power_of_two(parser, depth);
    }
    if parser.keyword("HAMMING_DISTANCE") || parser.keyword("BITHAMMINGDISTANCE") {
        return parse_hamming_distance(parser, depth);
    }
    if parser.keyword("POWER") || parser.keyword("POW") {
        return parse_power(parser, depth);
    }
    if parser.keyword("BIT_PARITY") {
        return parse_bit_parity(parser, depth);
    }
    if parser.keyword("BIT_MASK") {
        return parse_bit_mask(parser, depth);
    }
    if parser.keyword("BIT_WIDTH") {
        return parse_bit_width(parser, depth);
    }
    if parser.keyword("BIT_REVERSE") {
        return parse_bit_reverse(parser, depth);
    }
    if parser.keyword("CEIL_TO_MULTIPLE") {
        return parse_ceil_to_multiple(parser, depth);
    }
    if parser.keyword("CLAMP") {
        return parse_clamp(parser, depth);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("CEIL_DIV") || name.eq_ignore_ascii_case("FLOOR_DIV"))
    {
        return parse_rounded_division(parser, depth);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("BITCOUNT") || name.eq_ignore_ascii_case("BITNOT") || name.eq_ignore_ascii_case("BITTEST"))
    {
        return parse_bit_introspection(parser, depth);
    }
    if parser.keyword("ROUND_DIV") {
        return parse_round_div(parser, depth);
    }
    if parser.keyword("ROUND_TO_MULTIPLE") {
        return parse_round_to_multiple(parser, depth);
    }
    if parser.keyword("PMOD") {
        return parse_pmod(parser, depth);
    }
    if parser.keyword("FLOOR_TO_MULTIPLE") {
        return parse_floor_to_multiple(parser, depth);
    }
    if parser.keyword("BIT_EXTRACT") {
        return parse_bit_extract(parser, depth);
    }
    if parser.keyword("IS_DIVISIBLE") {
        return parse_is_divisible(parser, depth);
    }
    if parser.keyword("IN_RANGE") {
        return parse_in_range(parser, depth);
    }
    if parser.keyword("DIV_EUCLID") {
        return parse_euclidean_division(parser, depth, true);
    }
    if parser.keyword("REM_EUCLID") {
        return parse_euclidean_division(parser, depth, false);
    }
    if parser.keyword("MUL_DIV") {
        return parse_mul_div(parser, depth);
    }
    if parser.keyword("WIDTH_BUCKET") {
        return parse_width_bucket(parser, depth);
    }
    if matches!(
        parser.tokens.get(parser.index),
        Some(Token::Word(name)) if matches!(
            name.to_ascii_uppercase().as_str(),
            "BIT_SET" | "BIT_CLEAR" | "BIT_TOGGLE"
        )
    ) {
        return parse_bit_update(parser, depth);
    }
    if matches!(
        parser.tokens.get(parser.index),
        Some(Token::Word(name)) if matches!(
            name.to_ascii_uppercase().as_str(),
            "LEADING_ZEROS" | "TRAILING_ZEROS"
        )
    ) {
        return parse_bit_scans(parser, depth);
    }
    if parser.keyword("DIGITAL_ROOT") {
        return parse_digital_root(parser, depth);
    }
    if parser.keyword("POW10") {
        return parse_pow10(parser, depth);
    }
    if parser.keyword("TRIANGULAR") {
        return parse_triangular(parser, depth);
    }
    if parser.keyword("IS_PRIME") {
        return parse_is_prime(parser, depth);
    }
    if parser.keyword("PERMUTATIONS") {
        return parse_permutations(parser, depth);
    }
    if parser.keyword("BINOMIAL") {
        return parse_binomial(parser, depth);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("GCD") || name.eq_ignore_ascii_case("LCM"))
    {
        return parse_gcd_lcm(parser, depth);
    }
    if parser.keyword("INTLOG") {
        return parse_intlog(parser, depth);
    }
    if parser.keyword("DIGIT_COUNT") {
        return parse_digit_count(parser, depth);
    }
    if parser.keyword("MOD_POW") {
        return parse_mod_pow(parser, depth);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if is_extrema(name)) {
        return parse_extrema(parser, depth);
    }
    if parser.keyword("IS_POWER_OF_TWO") {
        return parse_is_power_of_two(parser, depth);
    }
    if parser.keyword("REVERSE_DIGITS") {
        return parse_reverse_digits(parser, depth);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if is_function(name)) {
        let name = parser.word()?;
        if !parser.take(Token::LParen) {
            return Err(ParseError("expected '(' after scalar function".into()));
        }
        let first = expression(parser, depth + 1)?;
        let result = if is_bitwise(&name) {
            if !parser.take(Token::Comma) {
                return Err(ParseError("bitwise functions require two arguments".into()));
            }
            let second = expression(parser, depth + 1)?;
            match name.to_ascii_uppercase().as_str() {
                "BITAND" => first & second,
                "BITOR" => first | second,
                "BITXOR" => first ^ second,
                "BITNAND" => !(first & second),
                "BITNOR" => !(first | second),
                "BITXNOR" => !(first ^ second),
                "BITSHIFTLEFT" => checked_bit_shift(first, second, true)?,
                "BITSHIFTRIGHT" => checked_bit_shift(first, second, false)?,
                "BITROTATELEFT" => checked_bit_rotate(first, second, true)?,
                "BITROTATERIGHT" => checked_bit_rotate(first, second, false)?,
                _ => unreachable!("only supported bitwise functions reach this branch"),
            }
        } else if name.eq_ignore_ascii_case("FIBONACCI") {
            if !(0..=92).contains(&first) {
                return Err(ParseError(
                    "FIBONACCI index must be between 0 and 92".into(),
                ));
            }
            let (mut current, mut next) = (0_u128, 1_u128);
            for _ in 0..first {
                (current, next) = (next, current + next);
            }
            i64::try_from(current).map_err(|_| overflow())?
        } else if name.eq_ignore_ascii_case("ABS") {
            first.checked_abs().ok_or_else(overflow)?
        } else if name.eq_ignore_ascii_case("DIGIT_SUM") {
            let mut magnitude = first.unsigned_abs();
            let mut sum = 0_i64;
            while magnitude > 0 {
                sum += (magnitude % 10) as i64;
                magnitude /= 10;
            }
            sum
        } else if name.eq_ignore_ascii_case("ISQRT") {
            if first < 0 {
                return Err(ParseError("ISQRT requires a non-negative integer".into()));
            }
            first.isqrt()
        } else if name.eq_ignore_ascii_case("FACTORIAL") {
            if !(0..=20).contains(&first) {
                return Err(ParseError(
                    "FACTORIAL argument must be between 0 and 20".into(),
                ));
            }
            (1..=first)
                .try_fold(1i64, |acc, value| acc.checked_mul(value))
                .ok_or_else(overflow)?
        } else if name.eq_ignore_ascii_case("INTLOG2") {
            if first <= 0 {
                return Err(ParseError("INTLOG2 requires a positive integer".into()));
            }
            i64::from(first.ilog2())
        } else {
            match name.to_ascii_uppercase().as_str() {
                "TRAILING_ONES" => i64::from(first.trailing_ones()),
                "TOINT8" => first as i8 as i64,
                "ISZERO" => i64::from(first == 0),
                _ => first.signum(),
            }
        };
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after scalar function".into()));
        }
        return Ok(result);
    }
    if parser.take(Token::LParen) {
        let value = expression(parser, depth + 1)?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' in scalar expression".into()));
        }
        return Ok(value);
    }
    parser
        .number()?
        .parse::<i64>()
        .map_err(|_| ParseError("SELECT literals must be signed 64-bit integers".into()))
}

fn parse_next_power_of_two(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "NEXT_POWER_OF_TWO requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "expected ')' after NEXT_POWER_OF_TWO argument",
    )?;
    let nonnegative = u64::try_from(value)
        .map_err(|_| ParseError("NEXT_POWER_OF_TWO requires nonnegative input".into()))?;
    i64::try_from(
        nonnegative
            .checked_next_power_of_two()
            .ok_or_else(overflow)?,
    )
    .map_err(|_| overflow())
}

fn overflow() -> ParseError {
    ParseError("signed 64-bit scalar expression overflow".into())
}

#[cfg(test)]
mod clickhouse_tobool_int64_tests {
    use crate::{parse_select, Projection};
    #[test]
    fn boolean_cast_is_zero_or_one_for_int64() {
        for (sql, expected) in [
            ("SELECT toBool(0)", 0),
            ("SELECT toBool(1)", 1),
            ("SELECT toBool(-1)", 1),
            ("SELECT toBool(400)", 1),
            ("SELECT toBool(1 - 1)", 0),
        ] {
            assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
        }
        for sql in ["SELECT toBool()", "SELECT toBool(1, 2)"] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }
}
