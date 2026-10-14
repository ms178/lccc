//! Bounded RISC-V SET/SUB ULEB128 relocation pairs.
//!
//! The reserved field length is an input property, not a property of the merged
//! output. Encode only the final difference: absolute intermediate label values
//! can require more bytes than the final value. Validation precedes mutation.

/// Length of an existing u64 ULEB field, bounded by its input contribution.
/// Encodings wider than u64 are explicitly unsupported, never silently decoded.
pub(super) fn field_len(data: &[u8]) -> Result<usize, String> {
    for (i, &byte) in data.iter().take(10).enumerate() {
        if i == 9 && byte > 1 {
            return Err("RISC-V ULEB128 field exceeds supported 64-bit encoding".into());
        }
        if byte & 0x80 == 0 {
            return Ok(i + 1);
        }
    }
    Err("unterminated or oversized RISC-V ULEB128 field in input section".into())
}

/// Apply a pair without encoding either intermediate address. Arithmetic is
/// widened so negative differences and addend overflow cannot masquerade as fits.
/// `data` must be exactly the reserved field found in the ORIGINAL input. Its
/// current contents may have been touched by an earlier overlapping relocation.
pub(super) fn apply_pair(
    data: &mut [u8],
    set: u64,
    set_addend: i64,
    sub: u64,
    sub_addend: i64,
) -> Result<(), String> {
    let len = data.len();
    if len == 0 || len > 10 {
        return Err("invalid RISC-V ULEB128 reserved field length".into());
    }
    let difference =
        i128::from(set) + i128::from(set_addend) - i128::from(sub) - i128::from(sub_addend);
    let mut value = u64::try_from(difference)
        .map_err(|_| "RISC-V ULEB128 pair result is outside unsigned 64-bit range")?;
    if len < 10 && value >> (7 * len) != 0 {
        return Err(format!(
            "RISC-V ULEB128 pair result does not fit reserved {}-byte field",
            len
        ));
    }
    for (i, byte) in data[..len].iter_mut().enumerate() {
        *byte = (value & 0x7f) as u8 | if i + 1 < len { 0x80 } else { 0 };
        value >>= 7;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(data: &[u8]) -> u64 {
        data.iter()
            .enumerate()
            .fold(0, |value, (i, b)| value | (u64::from(b & 0x7f) << (7 * i)))
    }

    #[test]
    fn absolute_labels_need_not_fit_the_final_field() {
        let mut merged = [0, 0xa5, 0xa5];
        apply_pair(&mut merged[..1], 0x1000, 0, 0xfff, 0).unwrap();
        assert_eq!(merged, [1, 0xa5, 0xa5]);
    }

    #[test]
    fn preserve_padding_and_neighbors_for_every_reserved_length() {
        for len in 1..=10 {
            for value in [0, 1, 127, 128, 16383, 16384, u64::MAX] {
                let mut merged = [0xa5; 12];
                merged[..len].fill(0x80);
                merged[len - 1] = 0;
                let before = merged;
                let fits = len == 10 || value >> (7 * len) == 0;
                let result = apply_pair(&mut merged[..len], value, 0, 0, 0);
                assert_eq!(result.is_ok(), fits, "length={len} value={value}");
                if fits {
                    assert_eq!(field_len(&merged[..len]).unwrap(), len);
                    assert_eq!(decode(&merged[..len]), value);
                    assert_eq!(&merged[len..], &before[len..]);
                } else {
                    assert_eq!(merged, before);
                }
            }
        }
    }

    #[test]
    fn exhaustive_small_values_and_lengths() {
        for len in 1..=3 {
            for value in 0..=16384 {
                let mut field = [0x80; 3];
                field[len - 1] = 0;
                let before = field;
                let fits = value < (1 << (7 * len));
                let result = apply_pair(&mut field[..len], value + 0x10000, 7, 0x10000, 7);
                assert_eq!(result.is_ok(), fits);
                if fits {
                    assert_eq!(decode(&field[..len]), value);
                    assert_eq!(field_len(&field[..len]).unwrap(), len);
                } else {
                    assert_eq!(field, before);
                }
            }
        }
    }

    #[test]
    fn malformed_original_fields_are_rejected() {
        for field in [
            vec![],
            vec![0x80],
            vec![0x80; 10],
            [vec![0x80; 9], vec![2]].concat(),
            vec![0x80; 20],
        ] {
            assert!(field_len(&field).is_err());
        }
    }

    #[test]
    fn earlier_overlapping_write_cannot_change_reserved_length() {
        let original = [0x80, 0];
        let len = field_len(&original).unwrap();
        let mut modified = [1, 0, 0xa5];
        apply_pair(&mut modified[..len], 1, 0, 0, 0).unwrap();
        assert_eq!(modified, [0x81, 0, 0xa5]);
    }

    #[test]
    fn widened_arithmetic_rejects_underflow_and_overflow() {
        for (set, sa, sub, sb) in [
            (0, 0, 1, 0),
            (u64::MAX, 1, 0, 0),
            (0, i64::MIN, 0, i64::MAX),
        ] {
            let mut field = [0x80; 10];
            field[9] = 0;
            let before = field;
            assert!(apply_pair(&mut field, set, sa, sub, sb).is_err());
            assert_eq!(field, before);
        }
        let mut field = [0];
        apply_pair(&mut field, u64::MAX, 1, u64::MAX, 0).unwrap();
        assert_eq!(field, [1]);
    }

    #[test]
    fn deterministic_full_width_samples_preserve_fields_and_sentinels() {
        let mut state = 0x8f47_913a_21dd_700bu64;
        for len in 1..=10 {
            for _ in 0..4096 {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let value = if len == 10 {
                    state
                } else {
                    state & ((1u64 << (7 * len)) - 1)
                };
                let mut merged = [0xa5; 12];
                merged[..len].fill(0x80);
                merged[len - 1] = 0;
                apply_pair(&mut merged[..len], u64::MAX, 17, u64::MAX - value, 17).unwrap();
                assert_eq!(decode(&merged[..len]), value);
                assert_eq!(field_len(&merged[..len]).unwrap(), len);
                assert!(merged[len..].iter().all(|&byte| byte == 0xa5));
            }
        }
    }

    #[test]
    fn invalid_reserved_lengths_do_not_mutate() {
        for len in [0, 11] {
            let mut field = vec![0xa5; len];
            let before = field.clone();
            assert!(apply_pair(&mut field, 1, 0, 0, 0).is_err());
            assert_eq!(field, before);
        }
    }
}
