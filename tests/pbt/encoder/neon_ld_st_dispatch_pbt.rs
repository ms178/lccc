//! Property-based tests for `encode_neon_ld_st_dispatch`.
//!
//! `encode_neon_ld_st_dispatch(operands, is_load, num_structs)` is the public
//! router for the AArch64 NEON load/store-structure family
//! (`LD1`/`LD2`/`LD3`/`LD4`/`ST1`/`ST2`/`ST3`/`ST4`). Its entire contract is:
//!
//! ```text
//!   operands[0] is Operand::RegListIndexed  ⇒  forward to encode_neon_ld_st_single
//!   otherwise (RegList / anything else)     ⇒  forward to encode_neon_ld_st_multi
//! ```
//! with `is_load` and `num_structs` passed through verbatim.
//!
//! ## Oracle
//! The router is *differentially* validated against the very sibling encoders
//! it forwards to (`encode_neon_ld_st_single` / `encode_neon_ld_st_multi`).
//! For every generated input, the dispatcher's output must be byte-identical
//! to the encoder selected by the routing rule. This pins routing correctness
//! independently of whether the sibling encoders are themselves spec-correct.
//!
//! Additionally, independently-assembled reference words (field-by-field OR,
//! mirroring the verified references in `neon_ld_st_single_pbt` and
//! `neon_ld_st_multi_pbt`) confirm the forwarded words land in the right bits.
//!
//! ## Findings (bug witnesses — all `#[ignore]`d so `cargo test` stays green)
//! The router itself is a trivial, provably-correct forwarder. The witnesses
//! below document that the *reachable* (through this public entry point)
//! defects of its callees are still observable here:
//! 1. `witness_inherited_single_d_unallocated` — a `RegListIndexed` `.d`
//!    element is forwarded to `single`, which emits an *unallocated* opcode
//!    (the `.s` group `100/101` instead of the spec `.d`/`.h` group `010/011`).
//! 2. `witness_inherited_multi_wrong_post_index_imm` — a multiple-structures
//!    post-index with a wrong `#imm` is forwarded to `multi`, which silently
//!    accepts it (the immediate is never validated against the transfer size).

use lccc::pbt_internals::EncodeResult;
use lccc::pbt_internals::Operand;
use lccc::pbt_internals::{
    encode_neon_ld_st_dispatch, encode_neon_ld_st_multi, encode_neon_ld_st_single,
};
use proptest::prelude::*;

// --- shared helpers -------------------------------------------------------

fn word_of(res: Result<EncodeResult, String>) -> u32 {
    match res {
        Ok(EncodeResult::Word(w)) => w,
        other => panic!("expected EncodeResult::Word, got {other:?}"),
    }
}

fn reg_num_strategy() -> impl Strategy<Value = u32> {
    0u32..=31u32
}

fn num_structs_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![Just(1u32), Just(2), Just(3), Just(4)]
}

// =========================================================================
//  Single-structure operand construction (.b/.h/.s only — where the SUT's
//  `single` encoder matches the spec; `.d` is exercised by the ignored
//  witness).
// =========================================================================

fn elem_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("b"), Just("h"), Just("s")]
}

fn max_index(elem: &str, num_structs: u32) -> u32 {
    let two_or_four = num_structs == 2 || num_structs == 4;
    match elem {
        "b" => {
            if two_or_four {
                7
            } else {
                15
            }
        }
        "h" => {
            if two_or_four {
                3
            } else {
                7
            }
        }
        "s" => {
            if two_or_four {
                1
            } else {
                3
            }
        }
        _ => 0,
    }
}

/// `{ v{rt}.{elem}, v{rt+1}.{elem}, ... }[index]` with `num` consecutive regs.
fn list_indexed(rt: u32, elem: &str, num: u32, index: u32) -> Operand {
    let regs: Vec<Operand> = (0..num)
        .map(|i| Operand::RegArrangement {
            reg: format!("v{}", rt + i),
            arrangement: elem.to_string(),
        })
        .collect();
    Operand::RegListIndexed { regs, index }
}

/// Build operand vec for the single route. `post_kind`:
/// `"none"` → `[Xn]`; `"post"` → `MemPostIndex{offset}`; `"imm3"` → `[Xn]` + `Imm`.
fn single_ops(rt: u32, rn: u32, elem: &str, num: u32, index: u32, post_kind: &str) -> Vec<Operand> {
    let mut ops = vec![list_indexed(rt, elem, num, index)];
    match post_kind {
        "none" => ops.push(Operand::Mem {
            base: format!("x{rn}"),
            offset: 0,
        }),
        "post" => ops.push(Operand::MemPostIndex {
            base: format!("x{rn}"),
            offset: 1,
        }),
        "imm3" => {
            ops.push(Operand::Mem {
                base: format!("x{rn}"),
                offset: 0,
            });
            ops.push(Operand::Imm(1));
        }
        _ => unreachable!("bad post_kind {post_kind}"),
    }
    ops
}

fn post_kind_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("none"), Just("post"), Just("imm3")]
}

fn single_case() -> impl Strategy<Value = (u32, u32, &'static str, u32, u32, &'static str)> {
    (elem_strategy(), num_structs_strategy()).prop_flat_map(|(elem, num)| {
        let rt_hi = 32u32.saturating_sub(num).max(1);
        let max = max_index(elem, num);
        (
            0u32..rt_hi,
            reg_num_strategy(),
            Just(elem),
            Just(num),
            0u32..=max,
            post_kind_strategy(),
        )
    })
}

// =========================================================================
//  Multiple-structures operand construction.
// =========================================================================

fn va(n: u32, arr: &str) -> Operand {
    Operand::RegArrangement {
        reg: format!("v{n}"),
        arrangement: arr.to_string(),
    }
}

fn reg_list(rt: u32, nr: u32, arr: &str) -> Operand {
    Operand::RegList((0..nr).map(|i| va(rt + i, arr)).collect())
}

fn mem(rn: u32) -> Operand {
    Operand::Mem {
        base: format!("x{rn}"),
        offset: 0,
    }
}

fn arr_strategy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just("8b"),
        Just("16b"),
        Just("4h"),
        Just("8h"),
        Just("2s"),
        Just("4s"),
        Just("1d"),
        Just("2d"),
    ]
}

/// A spec-valid (num_structs, num_regs, rt, rn, arrangement) combination.
fn multi_case() -> impl Strategy<Value = (u32, u32, u32, u32, &'static str)> {
    prop_oneof![
        // LD1/ST1: 1-4 registers in the list.
        (1u32..=4).prop_flat_map(|nr| (
            Just(1u32),
            Just(nr),
            0u32..=(32 - nr),
            reg_num_strategy(),
            arr_strategy(),
        )),
        // LD2/3/4: list length == structure count.
        (2u32..=4).prop_flat_map(|ns| (
            Just(ns),
            Just(ns),
            0u32..=(32 - ns),
            reg_num_strategy(),
            arr_strategy(),
        )),
    ]
}

// =========================================================================
//  Properties: routing contract
// =========================================================================

proptest! {
    /// PRIMARY ORACLE (differential): whenever `operands[0]` is a
    /// `RegListIndexed`, the dispatcher must produce the *exact* word that
    /// `encode_neon_ld_st_single` produces for the same inputs. This holds for
    /// every accepted single-structure case (.b/.h/.s, all struct counts,
    /// lane indices and addressing modes).
    #[test]
    fn prop_dispatch_equals_single_when_reglistindexed(
        (rt, rn, elem, num, index, post_kind) in single_case(),
        is_load in any::<bool>(),
    ) {
        let ops = single_ops(rt, rn, elem, num, index, post_kind);
        let via_dispatch = encode_neon_ld_st_dispatch(&ops, is_load, num);
        let via_single = encode_neon_ld_st_single(&ops, is_load, num);
        // Both must succeed and produce identical words.
        prop_assert!(via_dispatch.is_ok(), "expected Ok, got {via_dispatch:?}");
        prop_assert!(via_single.is_ok(), "expected Ok, got {via_single:?}");
        prop_assert_eq!(
            word_of(via_dispatch), word_of(via_single),
            "RegListIndexed input must route to single (words must match)",
        );
    }

    /// PRIMARY ORACLE (differential): whenever `operands[0]` is NOT a
    /// `RegListIndexed`, the dispatcher must produce the *exact* word that
    /// `encode_neon_ld_st_multi` produces. Covers every accepted
    /// multiple-structures arrangement and struct count.
    #[test]
    fn prop_dispatch_equals_multi_otherwise(
        (ns, nr, rt, rn, arr) in multi_case(),
        is_load in any::<bool>(),
    ) {
        let ops = vec![reg_list(rt, nr, arr), mem(rn)];
        let via_dispatch = encode_neon_ld_st_dispatch(&ops, is_load, ns);
        let via_multi = encode_neon_ld_st_multi(&ops, is_load, ns);
        prop_assert!(via_dispatch.is_ok(), "expected Ok, got {via_dispatch:?}");
        prop_assert!(via_multi.is_ok(), "expected Ok, got {via_multi:?}");
        prop_assert_eq!(
            word_of(via_dispatch), word_of(via_multi),
            "RegList input must route to multi (words must match)",
        );
    }

    /// ROUTING KEY: the decision turns ONLY on whether `operands[0]` is the
    /// `RegListIndexed` variant. Two inputs identical except for that variant
    /// route to different encoders. Here we feed a `RegList` (no index) and
    /// confirm dispatch == multi and dispatch != single's behaviour on the
    /// same shape.
    #[test]
    fn prop_routing_key_is_first_operand_variant(
        (ns, nr, rt, rn, arr) in multi_case(),
        is_load in any::<bool>(),
    ) {
        let ops = vec![reg_list(rt, nr, arr), mem(rn)];
        // Multi route: dispatch must match multi exactly…
        prop_assert_eq!(
            word_of(encode_neon_ld_st_dispatch(&ops, is_load, ns)),
            word_of(encode_neon_ld_st_multi(&ops, is_load, ns)),
        );
        // …and must NOT silently degrade into the single path: passing the
        // same ops to single is a type error (single requires RegListIndexed),
        // so single must Err on this input, while dispatch succeeds.
        prop_assert!(
            encode_neon_ld_st_single(&ops, is_load, ns).is_err(),
            "single must reject a plain RegList operand (no index)",
        );
        prop_assert!(
            encode_neon_ld_st_dispatch(&ops, is_load, ns).is_ok(),
            "dispatch must accept the RegList input via the multi route",
        );
    }

    /// `is_load` pass-through: bit 22 (L) flips with `is_load` in BOTH routes.
    /// For the single route we use a `.s` single-element list; for the multi
    /// route a `.4s` list — both produce a loadable/storable encoding.
    #[test]
    fn prop_is_load_flips_l_bit_in_both_routes(
        rt in reg_num_strategy(),
        rn in reg_num_strategy(),
    ) {
        // single route
        let s_ops = single_ops(rt, rn, "s", 1, 0, "none");
        let s_ld = word_of(encode_neon_ld_st_dispatch(&s_ops, true, 1));
        let s_st = word_of(encode_neon_ld_st_dispatch(&s_ops, false, 1));
        prop_assert_eq!(s_ld ^ s_st, 1u32 << 22, "single route: L bit (22) must flip");

        // multi route
        let m_ops = vec![reg_list(rt, 1, "4s"), mem(rn)];
        let m_ld = word_of(encode_neon_ld_st_dispatch(&m_ops, true, 1));
        let m_st = word_of(encode_neon_ld_st_dispatch(&m_ops, false, 1));
        prop_assert_eq!(m_ld ^ m_st, 1u32 << 22, "multi route: L bit (22) must flip");
    }

    /// `num_structs` pass-through (multi route): holding everything else fixed,
    /// `ld1 {v0.8b}` (opcode 0111), `ld2 {v0,v1.8b}` (1000), `ld3` (0100) and
    /// `ld4` (0000) differ in the opcode field (bits 15-12) exactly as the
    /// spec table dictates — proving num_structs is forwarded, not dropped.
    #[test]
    fn prop_num_structs_forwarded_in_multi_route(rn in reg_num_strategy()) {
        let cases = [(1u32, 0b0111u32), (2, 0b1000), (3, 0b0100), (4, 0b0000)];
        for (ns, opc) in cases {
            let ops = vec![reg_list(0, ns, "8b"), mem(rn)];
            let w = word_of(encode_neon_ld_st_dispatch(&ops, true, ns));
            // opcode field (bits 15-12) must reflect num_structs per the ARM table.
            prop_assert_eq!((w >> 12) & 0xF, opc);
        }
    }

    /// Determinism: identical inputs always yield identical outputs in both
    /// routes.
    #[test]
    fn prop_dispatch_is_deterministic(
        (rt, rn, elem, num, index, post_kind) in single_case(),
        (ns, nr, mrt, mrn, arr) in multi_case(),
        is_load in any::<bool>(),
    ) {
        let s_ops = single_ops(rt, rn, elem, num, index, post_kind);
        prop_assert_eq!(
            word_of(encode_neon_ld_st_dispatch(&s_ops, is_load, num)),
            word_of(encode_neon_ld_st_dispatch(&s_ops, is_load, num)),
        );
        let m_ops = vec![reg_list(mrt, nr, arr), mem(mrn)];
        prop_assert_eq!(
            word_of(encode_neon_ld_st_dispatch(&m_ops, is_load, ns)),
            word_of(encode_neon_ld_st_dispatch(&m_ops, is_load, ns)),
        );
    }
}

// =========================================================================
//  Deterministic edge / contract tests
// =========================================================================

#[test]
fn empty_operands_route_to_multi_and_error() {
    // operands.first() == None ⇒ falls through to multi, which requires ≥2
    // operands, so dispatch must Err.
    assert!(
        encode_neon_ld_st_dispatch(&[], true, 1).is_err(),
        "empty operand slice must error (multi needs ≥2 operands)",
    );
}

#[test]
fn non_reglistindexed_first_operand_always_routes_to_multi() {
    // A plain RegList (no index): multi accepts it, single requires
    // RegListIndexed and rejects it. dispatch matching multi's word (and
    // succeeding) proves it took the multi route, not the single route.
    let ops = vec![reg_list(0, 1, "8b"), mem(1)];
    assert_eq!(
        word_of(encode_neon_ld_st_dispatch(&ops, true, 1)),
        word_of(encode_neon_ld_st_multi(&ops, true, 1)),
        "RegList operand[0] must route to multi",
    );
    assert!(
        encode_neon_ld_st_single(&ops, true, 1).is_err(),
        "single must reject a plain RegList (no index)",
    );
}

#[test]
fn golden_routing_matches_both_siblings() {
    // Single route: st1 {v0.s}[0], [x1]  (golden from neon_ld_st_single_pbt).
    let s_ops = single_ops(0, 1, "s", 1, 0, "none");
    assert_eq!(
        word_of(encode_neon_ld_st_dispatch(&s_ops, false, 1)),
        word_of(encode_neon_ld_st_single(&s_ops, false, 1)),
    );
    assert_eq!(
        word_of(encode_neon_ld_st_dispatch(&s_ops, false, 1)),
        0x0D008020,
    );

    // Multi route: ld1 {v0.16b}, [x1]  (golden from neon_ld_st_multi_pbt).
    let m_ops = vec![reg_list(0, 1, "16b"), mem(1)];
    assert_eq!(
        word_of(encode_neon_ld_st_dispatch(&m_ops, true, 1)),
        word_of(encode_neon_ld_st_multi(&m_ops, true, 1)),
    );
    assert_eq!(
        word_of(encode_neon_ld_st_dispatch(&m_ops, true, 1)),
        0x4C407020,
    );
}

// =========================================================================
//  Bug witnesses — #[ignore]d: default `cargo test` stays green.
//  Run with:  cargo test --lib neon_ld_st_dispatch_pbt -- --ignored
// =========================================================================

proptest! {
    /// The single route forwards `.d` elements to `encode_neon_ld_st_single`,
    /// which emits the `.s`-group opcode (100/101) with size=01 — an
    /// *unallocated* encoding. Per the ARMv8-A ARM, single-structure `.d`
    /// must use the `.h`-group opcode (010/011) with size=01. The reference
    /// below follows the spec; the dispatch output does not. Inherited from
    /// `encode_neon_ld_st_single` (see neon_ld_st_single_pbt).
    #[test]
    #[ignore = "inherited witness: single route emits unallocated .d opcode"]
    fn witness_inherited_single_d_unallocated(
        num in num_structs_strategy(),
        rt in 0u32..=30,
        rn in reg_num_strategy(),
        index in 0u32..=1,
        is_load in any::<bool>(),
    ) {
        let rt = rt.min(31u32.saturating_sub(num - 1));
        let ops = single_ops(rt, rn, "d", num, index, "none");
        let got = word_of(encode_neon_ld_st_dispatch(&ops, is_load, num));

        // Spec-correct reference: .d shares the .h opcode group, size=01.
        let l: u32 = if is_load { 1 } else { 0 };
        let r: u32 = if num == 2 || num == 4 { 1 } else { 0 };
        let opcode: u32 = if num <= 2 { 0b010 } else { 0b011 };
        let q: u32 = index & 1;
        let want = (q << 30)
            | (0b001101u32 << 24)
            | (l << 22)
            | (r << 21)
            | (opcode << 13)
            | (0b01u32 << 10)
            | (rn << 5)
            | rt;
        prop_assert_eq!(got, want, ".d via dispatch must use the spec opcode group");
    }

    /// The multi route forwards multiple-structures post-index forms to
    /// `encode_neon_ld_st_multi`, which never validates that the post-index
    /// immediate equals the transfer size. A wrong `#imm` (here always smaller
    /// than the 8-byte minimum transfer) is silently accepted. `llvm-mc`
    /// rejects it with "invalid operand for instruction". Inherited from
    /// `encode_neon_ld_st_multi` (see neon_ld_st_multi_pbt).
    #[test]
    #[ignore = "inherited witness: multi route does not validate post-index imm"]
    fn witness_inherited_multi_wrong_post_index_imm(
        (ns, nr, rt, rn, arr) in multi_case(),
        is_load in any::<bool>(),
        bad_imm in 1i64..=7, // always < smallest 8-byte transfer ⇒ never valid
    ) {
        let ops = vec![
            reg_list(rt, nr, arr),
            Operand::MemPostIndex { base: format!("x{rn}"), offset: bad_imm },
        ];
        let res = encode_neon_ld_st_dispatch(&ops, is_load, ns);
        prop_assert!(
            res.is_err(),
            "post-index #{bad_imm} (ns={ns},nr={nr},{arr}) is not a valid transfer \
             size; dispatch must Err, got {res:?}",
        );
    }
}
