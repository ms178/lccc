//! Shared ADR/ADRP immediate-field contract. ADR uses bytes; ADRP uses pages.
//! Validate before masking: a wrapped signed-21 field is a different address.

pub(super) fn adr_imm_bits(displacement: i64) -> Result<u32, String> {
    if !(-(1 << 20)..(1 << 20)).contains(&displacement) {
        return Err(format!(
            "ADR/ADRP displacement {displacement} out of signed 21-bit range"
        ));
    }
    let bits = displacement as u32;
    Ok(((bits & 3) << 29) | (((bits >> 2) & 0x7ffff) << 5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_signed_21_bit_value_round_trips_without_touching_opcode_or_rd() {
        for displacement in -(1 << 20)..(1 << 20) {
            let bits = adr_imm_bits(displacement).unwrap();
            assert_eq!(bits & 0x9f00001f, 0);
            let raw = ((bits >> 29) & 3) | (((bits >> 5) & 0x7ffff) << 2);
            let decoded = ((raw as i32) << 11) >> 11;
            assert_eq!(i64::from(decoded), displacement);
        }
    }

    #[test]
    fn reject_outside_endpoints_and_integer_extremes() {
        for displacement in [i64::MIN, -1048577, 1048576, i64::MAX] {
            assert!(adr_imm_bits(displacement).is_err());
        }
    }
}
