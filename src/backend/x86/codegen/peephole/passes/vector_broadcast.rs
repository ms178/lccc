//! Broadcast copy folding: a copied scalar splatted in place becomes one shuffle.
//!
//! The scalar-SSE lowerings broadcast a value by copying it into a scratch
//! register and unpacking that register with itself, because `unpcklpd`/`unpcklps`
//! are two-operand instructions whose destination is also a source:
//!
//! ```text
//!     movsd  bodies+48(%rip), %xmm2     ; the scalar the kernel also uses directly
//!     movapd %xmm2, %xmm0               ; copy into the fixed scratch
//!     unpcklpd %xmm0, %xmm0             ; splat
//!     mulpd  %xmm0, %xmm12
//! ```
//!
//! The copy exists only to give the splat a distinct destination. `pshufd`
//! reads its source and writes a *different* destination, so the same 128-bit
//! result costs one instruction instead of two, at the same SSE2 baseline:
//!
//! | pattern | result | replacement |
//! |---|---|---|
//! | `mov %A,%T` ; `unpcklpd %T,%T` | `{A0, A0}` (qwords) | `pshufd $0x44, %A, %T` |
//! | `mov %A,%T` ; `unpcklps %T,%T` | `{A0, A0, A1, A1}` (dwords) | `pshufd $0x50, %A, %T` |
//!
//! # Why it is legal
//!
//! 1. **`pshufd $0x44` of `%A` is the packed-double splat of `%A`.** The
//!    selector `0x44` is `(0,1,0,1)` over dwords, i.e. `{a0,a1,a0,a1}`, and the
//!    64-bit lane form `{A0,A0}` of an in-place `unpcklpd %T,%T` is exactly
//!    dwords `{a0,a1,a0,a1}` when `T0 = A0`.
//! 2. **`pshufd $0x50` is the packed-single splat of `%A`.** `0x50` is
//!    `(0,0,1,1)`, matching `unpcklps %T,%T` = `{T0,T0,T1,T1}` when `T0 = A0`
//!    and `T1 = A1`.
//! 3. **The copy must define every bit the unpck reads.** `unpcklpd` reads only
//!    `T[63:0]`, so `movsd`/`movq` (which define exactly those bits from `%A`)
//!    qualify; `movss` defines 32 bits and is rejected for both forms.
//!    `unpcklps` reads `T[63:0]` too, but its `T1` must equal `A1`, so only a
//!    full 128-bit move qualifies — `movsd` is rejected there.
//! 4. **The source register survives.** `pshufd` reads `%A` and writes `%T`;
//!    the old two-instruction form also left `%A` untouched, so a later use of
//!    `%A` (nbody reads the scalar again for the paired `mulsd`) still sees the
//!    same value.
//! 5. **`%T` is fully defined by the replacement**, so no consumer can observe
//!    a stale upper half, and `%A != %T` is required because `pshufd` would
//!    then overwrite the source the second instruction still needs.
//! 6. Neither form touches flags, and both are legacy-SSE (no VEX upper-half
//!    zeroing difference to preserve).

use super::super::types::*;

/// Move mnemonics whose xmm→xmm copy defines at least the low 64 bits from the
/// source, i.e. the source's `A0` is the copied `T0`. The boolean records
/// whether a full 128 bits are defined (required for the `unpcklps` form).
fn move_defines(line: &str) -> Option<(&str, &str, bool)> {
    let (mnemonic, rest) = line.split_once(char::is_whitespace)?;
    let (src, dst) = rest.split_once(',')?;
    let (src, dst) = (src.trim(), dst.trim());
    if !src.starts_with("%xmm") || !dst.starts_with("%xmm") || src == dst {
        return None;
    }
    // `movdqa`/`movaps` family: 128 bits. `movsd`/`movq`: low 64.
    // `movss`: low 32, which no `unpck` here can use on its own.
    let full = matches!(
        mnemonic,
        "movapd" | "movaps" | "movupd" | "movups" | "movdqa" | "movdqu" | "movdqa64" | "movdqu64"
    );
    let low64 = matches!(mnemonic, "movsd" | "movq");
    (full || low64).then_some((src, dst, full))
}

/// `unpcklp{d,s} %T, %T` -> the shuffle immediate that splats `%A` instead.
fn splat_of(line: &str, dst: &str) -> Option<&'static str> {
    let (mnemonic, rest) = line.split_once(char::is_whitespace)?;
    let (src, dest) = rest.split_once(',')?;
    if src.trim() != dst || dest.trim() != dst {
        return None;
    }
    match mnemonic {
        "unpcklpd" => Some("$0x44"),
        "unpcklps" => Some("$0x50"),
        _ => None,
    }
}

/// Fold `mov %A,%T` + in-place splat of `%T` into a single `pshufd` of `%A`.
pub(super) fn fold_broadcast_copies(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    for i in 0..store.len() {
        if infos[i].is_nop() || infos[i].pinned {
            continue;
        }
        let Some((src, dst, full)) = move_defines(infos[i].trimmed(store.get(i))) else {
            continue;
        };
        let mut j = i + 1;
        while j < store.len() && infos[j].is_nop() {
            j += 1;
        }
        if j >= store.len() || infos[j].pinned {
            continue;
        }
        let Some(imm) = splat_of(infos[j].trimmed(store.get(j)), dst) else {
            continue;
        };
        if imm == "$0x50" && !full {
            // `unpcklps` also consumes T1, which a 64-bit copy never defined.
            continue;
        }
        let replacement = format!("    pshufd {imm}, {src}, {dst}");
        let mut info = infos[i];
        replace_line(store, &mut info, i, replacement);
        infos[i] = info;
        mark_nop(&mut infos[j]);
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(asm: &str) -> String {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(fold_broadcast_copies(&mut store, &mut infos));
        store.build_result(|i| infos[i].is_nop())
    }

    #[test]
    fn packed_double_broadcast_becomes_one_shuffle() {
        let out = run(concat!(
            "f:\n",
            "    movapd %xmm2, %xmm0\n",
            "    unpcklpd %xmm0, %xmm0\n",
            "    ret\n",
        ));
        assert!(out.contains("pshufd $0x44, %xmm2, %xmm0"), "{out}");
        assert!(!out.contains("unpcklpd"), "{out}");
        assert!(!out.contains("movapd"), "{out}");
    }

    #[test]
    fn packed_single_broadcast_becomes_one_shuffle() {
        let out = run(concat!(
            "f:\n",
            "    movaps %xmm5, %xmm1\n",
            "    unpcklps %xmm1, %xmm1\n",
            "    ret\n",
        ));
        assert!(out.contains("pshufd $0x50, %xmm5, %xmm1"), "{out}");
    }

    #[test]
    fn narrow_move_still_splats_packed_double() {
        // movsd defines T[63:0] from A[63:0], and unpcklpd reads only those.
        let out = run(concat!(
            "f:\n",
            "    movsd %xmm7, %xmm3\n",
            "    unpcklpd %xmm3, %xmm3\n",
            "    ret\n",
        ));
        assert!(out.contains("pshufd $0x44, %xmm7, %xmm3"), "{out}");
    }

    #[test]
    fn packed_single_refuses_a_narrow_copy() {
        // unpcklps consumes T1, which movsd never defined from A.
        let asm = concat!(
            "f:\n",
            "    movsd %xmm7, %xmm3\n",
            "    unpcklps %xmm3, %xmm3\n",
            "    ret\n",
        )
        .to_string();
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(!fold_broadcast_copies(&mut store, &mut infos));
        assert!(
            store
                .build_result(|i| infos[i].is_nop())
                .contains("unpcklps")
        );
    }

    #[test]
    fn survives_intervening_removed_lines() {
        // A line an earlier pass already retired (`mark_nop`) sits between the
        // copy and the splat; the fold must still see the pair.
        let asm = concat!(
            "f:\n",
            "    movapd %xmm4, %xmm6\n",
            "    movl %eax, %eax\n",
            "    unpcklpd %xmm6, %xmm6\n",
            "    ret\n",
        )
        .to_string();
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        mark_nop(&mut infos[2]);
        assert!(fold_broadcast_copies(&mut store, &mut infos));
        let out = store.build_result(|i| infos[i].is_nop());
        assert!(out.contains("pshufd $0x44, %xmm4, %xmm6"), "{out}");
    }

    #[test]
    fn leaves_a_distinct_source_and_destination_alone() {
        let asm = concat!(
            "f:\n",
            "    movapd %xmm2, %xmm0\n",
            "    unpcklpd %xmm0, %xmm1\n",
            "    ret\n",
        )
        .to_string();
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(!fold_broadcast_copies(&mut store, &mut infos));
    }
}
