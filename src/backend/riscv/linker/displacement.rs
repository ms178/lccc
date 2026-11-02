//! Representability of PC-relative immediates before any field mutation.
//!
//! RV64 address arithmetic is modulo XLEN, then interpreted as a signed
//! displacement. Checking before the patcher's bit masks prevents truncation.

#[derive(Clone, Copy, Debug)]
pub(super) enum Encoding {
    Branch,
    Jal,
    RvcBranch,
    RvcJump,
}

impl Encoding {
    fn bits(self) -> u32 {
        match self {
            Self::Branch => 13,
            Self::Jal => 21,
            Self::RvcBranch => 9,
            Self::RvcJump => 12,
        }
    }
}

pub(super) fn checked_delta(
    encoding: Encoding,
    symbol: u64,
    addend: i64,
    place: u64,
) -> Result<u32, String> {
    let delta = symbol.wrapping_add(addend as u64).wrapping_sub(place) as i64;
    if delta & 1 != 0 {
        return Err(format!(
            "RISC-V {encoding:?} misaligned displacement {delta}"
        ));
    }
    let bits = encoding.bits();
    let limit = 1_i64 << (bits - 1);
    if delta < -limit || delta >= limit {
        return Err(format!(
            "RISC-V {encoding:?} displacement {delta} does not fit signed {bits}-bit immediate"
        ));
    }
    Ok(delta as u32)
}

/// An AUIPC plus signed low-12 immediate has an asymmetric RV64 range:
/// [-2^31 - 2^11, 2^31 - 2^11 - 1]. Checking only a signed 32-bit delta
/// both rejects valid negative offsets and accepts positive offsets whose
/// rounded HI20 changes sign. Keep XLEN wrapping, as for branch relocations.
/// No alignment restriction: PCREL data addresses may be odd, and JALR itself
/// clears bit zero (unlike JAL/branch, its immediate does not discard that bit).
pub(super) fn checked_auipc_delta(symbol: u64, addend: i64, place: u64) -> Result<i64, String> {
    let delta = symbol.wrapping_add(addend as u64).wrapping_sub(place) as i64;
    if !(-(1_i64 << 31) - 2048..(1_i64 << 31) - 2048).contains(&delta) {
        return Err(format!(
            "RISC-V AUIPC displacement {delta} does not fit rounded signed 20-bit immediate"
        ));
    }
    Ok(delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independent ISA facts, not expectations derived from the implementation.
    const ENCODINGS: [(Encoding, u32); 4] = [
        (Encoding::Branch, 13),
        (Encoding::Jal, 21),
        (Encoding::RvcBranch, 9),
        (Encoding::RvcJump, 12),
    ];

    #[test]
    fn exhaustive_representable_displacements() {
        for (encoding, bits) in ENCODINGS {
            assert_eq!(encoding.bits(), bits);
            let limit = 1_i64 << (bits - 1);
            let place = 1_u64 << 32;
            for delta in (-limit..limit).step_by(2) {
                let symbol = place.wrapping_add(delta as u64);
                let encoded = checked_delta(encoding, symbol, 0, place).unwrap();
                assert_eq!(i64::from(encoded as i32), delta);
            }
        }
    }

    #[test]
    fn reject_truncation_and_discarded_alignment_bits() {
        for (encoding, bits) in ENCODINGS {
            assert_eq!(encoding.bits(), bits);
            let limit = 1_i64 << (bits - 1);
            let place = 1_u64 << 32;
            for delta in [-limit - 4, -limit - 2, limit, limit + 2, i64::MIN] {
                let symbol = place.wrapping_add(delta as u64);
                assert!(checked_delta(encoding, symbol, 0, place).is_err());
            }
            for delta in [-limit + 1, -1, 1, limit - 1] {
                let symbol = place.wrapping_add(delta as u64);
                assert!(
                    checked_delta(encoding, symbol, 0, place)
                        .unwrap_err()
                        .contains("misaligned")
                );
            }
        }
    }

    #[test]
    fn xlen_wrap_and_signed_boundary_are_not_host_overflows() {
        for (encoding, _) in ENCODINGS {
            assert_eq!(checked_delta(encoding, 0, 0, u64::MAX - 1).unwrap(), 2);
            let backward = checked_delta(encoding, u64::MAX - 1, 0, 0).unwrap();
            assert_eq!(backward, (-2_i32) as u32);
            assert_eq!(
                checked_delta(encoding, 1 << 63, 0, (1 << 63) - 2).unwrap(),
                2
            );
            assert_eq!(checked_delta(encoding, 1 << 63, i64::MIN, 0).unwrap(), 0);
            let odd = checked_delta(encoding, 0, i64::MAX, i64::MAX as u64 - 1);
            assert!(odd.unwrap_err().contains("misaligned"));
            let forward = checked_delta(encoding, 0, i64::MAX, i64::MAX as u64 - 2).unwrap();
            assert_eq!(forward, 2);
        }
    }

    #[test]
    fn auipc_rounding_boundaries_and_all_low_bits() {
        // Independently construct offsets from signed ISA field values, then
        // check representability independently of the encoding module. The
        // encoder owns a separate production round-trip regression.
        for high in [-524288_i64, -1, 0, 1, 524287] {
            for low in -2048_i64..2048 {
                let expected = high * 4096 + low;
                let place = 1_u64 << 63;
                let symbol = place.wrapping_add(expected as u64);
                let delta = checked_auipc_delta(symbol, 0, place).unwrap();
                assert_eq!(delta, expected);
            }
        }
    }

    #[test]
    fn auipc_rejects_rounded_overflow_not_just_i32_overflow() {
        for delta in [i64::MIN, -2147485697, 2147481600, 2147483647, i64::MAX] {
            assert!(checked_auipc_delta(0, delta, 0).is_err());
        }
        for delta in [-2147485696, -2147483648, -1, 0, 1, 2147481599] {
            assert_eq!(checked_auipc_delta(0, delta, 0).unwrap(), delta);
        }
    }

    #[test]
    fn auipc_uses_xlen_arithmetic_across_address_and_addend_wrap() {
        assert_eq!(checked_auipc_delta(0, 0, u64::MAX).unwrap(), 1);
        assert_eq!(checked_auipc_delta(u64::MAX, 0, 0).unwrap(), -1);
        assert_eq!(checked_auipc_delta(1 << 63, i64::MIN, 0).unwrap(), 0);
        assert_eq!(checked_auipc_delta(1 << 63, 1, (1 << 63) - 1).unwrap(), 2);
    }
}
