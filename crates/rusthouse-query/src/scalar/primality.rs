use super::*;

/// Binary exponentiation with exact UInt128 intermediates for 64-bit residues.
pub(super) fn pow_mod(mut base: u64, mut exponent: u64, modulus: u64) -> u64 {
    // Most scalar inputs fit in 32 bits. Multiplying two residues of a
    // 32-bit modulus cannot overflow u64, so avoid costly u128 division
    // for every bit of the Miller-Rabin exponent.
    if modulus <= u32::MAX as u64 {
        base %= modulus;
        let mut result = 1u64;
        while exponent > 0 {
            if exponent & 1 != 0 {
                result = result * base % modulus;
            }
            exponent >>= 1;
            if exponent != 0 {
                base = base * base % modulus;
            }
        }
        return result;
    }

    let mut result = 1u64;
    while exponent > 0 {
        if exponent & 1 != 0 {
            result = ((u128::from(result) * u128::from(base)) % u128::from(modulus)) as u64;
        }
        exponent >>= 1;
        if exponent != 0 {
            base = ((u128::from(base) * u128::from(base)) % u128::from(modulus)) as u64;
        }
    }
    result
}

/// Deterministic Miller-Rabin for every UInt64 value, using seven proven
/// witnesses. The scalar SQL argument is limited to signed Int64.
pub(super) fn prime_u64(value: u64) -> bool {
    const SMALL_PRIMES: [u64; 12] = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];
    if value < 2 {
        return false;
    }
    for prime in SMALL_PRIMES {
        if value == prime {
            return true;
        }
        if value % prime == 0 {
            return false;
        }
    }
    let shifts = (value - 1).trailing_zeros();
    let odd = (value - 1) >> shifts;
    for base in [2u64, 325, 9375, 28178, 450775, 9780504, 1795265022] {
        let residue = base % value;
        if residue == 0 {
            continue;
        }
        let mut power = pow_mod(residue, odd, value);
        if power == 1 || power == value - 1 {
            continue;
        }
        let mut witnessed_nontrivial_root = false;
        for _ in 1..shifts {
            power = ((u128::from(power) * u128::from(power)) % u128::from(value)) as u64;
            if power == value - 1 {
                witnessed_nontrivial_root = true;
                break;
            }
        }
        if !witnessed_nontrivial_root {
            return false;
        }
    }
    true
}
