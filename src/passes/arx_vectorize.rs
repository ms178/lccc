//! ARX lane vectorization (generic lane-parallel Add-Rotate-Xor ciphers).
//!
//! A scalar `for (i = 0; i < 10; i++) { 8x QR over 16 u32 words }` loop is
//! 96 loop-carried scalar ARX instructions per iteration — a textbook
//! ILP-versus-serialization workload: the four quarter rounds of each round
//! are INDEPENDENT, but the scalar code interleaves them through 16 GPRs,
//! so the machine sees one long dependence chain per round.
//!
//! The recognized class is the lane-parallel 4x4 ARX family, NOT a single
//! cipher: a 16-word loop whose body is two groups of four quarter rounds
//! `a += b; d ^= a; d = rot(d); c += d; b ^= c; b = rot(b); a += b; d ^= a;
//! d = rot(d); c += d; b ^= c; b = rot(b)` — the first group lane-aligned,
//! the second group's (b, c, d) operands lane-rotated within their role
//! groups by provable offsets.  The rotate amounts are arbitrary cipher
//! constants (peeled off the proved terms, not assumed): ChaCha20 is the
//! instance `16/12/8/7` with offsets `(1, 2, 3)`, but a variant with
//! swapped per-group schedules (BLAKE-style `8/7/16/12` second groups) or
//! different offsets matches just as well.  Ciphers outside the class —
//! rotations on a/c roles, role-cycling wirings (plain Salsa rows), or
//! non-16-word states — fail the proof and stay scalar.
//!
//! The transform packs the 16 state words into four XMM role registers
//!
//! ```text
//!     A = [x0, x1, x2, x3]      B = [x4,  x5,  x6,  x7]
//!     C = [x8, x9, x10, x11]    D = [x12, x13, x14, x15]
//! ```
//!
//! so that lane j of `(A, B, C, D)` is exactly the j-th lane-aligned
//! quarter round's `(a, b, c, d)` tuple — all four QRs of a group execute
//! as ONE 4-lane SIMD quarter round.  The second group is the same SIMD
//! round over lane-rotated roles (the proved offsets collapse to at most
//! three `pshufd` lane rotations, skipped when an offset is zero).  With
//! SSSE3+ every whole-byte rotate is a byte permutation — one `pshufb`
//! each with a loop-invariant mask (one mask per DISTINCT byte-multiple
//! amount) — and the other rotates stay 3-instruction
//! `pslld/psrld/por` sequences.  RFC 7539 ChaCha20 then lands at 38 data
//! instructions per double round (SSSE3), 46 at the plain SSE2 baseline,
//! versus 96 scalar instructions — and versus 40+2 for
//! `icx -O3 -march=x86-64-v3` (Ice Lake's own AVX2 lowering).
//!
//! # Exactness proof strategy
//!
//! The matcher does NOT trust the loop's shape.  It symbolically evaluates
//! the loop body over the 16 state-word phi symbols and then PROVES that
//! the 16 backedge terms are structurally identical (up to the
//! commutativity of add/xor) to a reference construction of the generic
//! double round over the same symbols — for SOME bijection phi ↔ x_k, SOME
//! rotate schedule, and SOME lane offsets, all discovered from the terms
//! themselves.  Structural term equality is semantic equality here because
//! every node is a lane-exact u32 operation; the emitted SIMD body
//! evaluates the very same terms, four lanes at a time.  If the proof
//! fails — any ARX permutation that is not
//! exactly the RFC 7539 double round — the loop keeps its scalar form,
//! fail closed.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::instruction::{BlockId, Instruction, Operand, Terminator, Value};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::ops::IrBinOp;
use crate::ir::reexports::{IrConst, IrFunction};
use crate::passes::loop_analysis;

/// Debug trace for analyze bail-outs (LCCC_DEBUG_ARX).
fn arx_dbg(what: &str) {
    if std::env::var("LCCC_DEBUG_ARX").is_ok() {
        eprintln!("[ARX]   reject: {what}");
    }
}
use std::rc::Rc;

// ===========================================================================
// Symbolic terms
// ===========================================================================

#[derive(Debug)]
enum TermNode {
    /// The k-th state word phi.
    Phi(usize),
    /// A 32-bit constant.
    Const(u32),
    Add(Term, Term),
    Xor(Term, Term),
    Rot(Term, u8),
}

type Term = Rc<TermNode>;

impl TermNode {
    /// Canonical sort key.  Commutative nodes normalize their children into
    /// key order at construction, so derived equality IS mathematical
    /// equality modulo (x+y)=(y+x) and (x^y)=(y^x).
    fn key(&self) -> String {
        match self {
            TermNode::Phi(k) => format!("p{k}"),
            TermNode::Const(v) => format!("c{v:08x}"),
            TermNode::Add(a, b) => format!("(+ {} {})", a.key(), b.key()),
            TermNode::Xor(a, b) => format!("(^ {} {})", a.key(), b.key()),
            TermNode::Rot(a, n) => format!("(r{n} {})", a.key()),
        }
    }
}

impl PartialEq for TermNode {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (TermNode::Phi(a), TermNode::Phi(b)) => a == b,
            (TermNode::Const(a), TermNode::Const(b)) => a == b,
            (TermNode::Add(a1, b1), TermNode::Add(a2, b2))
            | (TermNode::Xor(a1, b1), TermNode::Xor(a2, b2)) => a1 == a2 && b1 == b2,
            (TermNode::Rot(a1, n1), TermNode::Rot(a2, n2)) => n1 == n2 && a1 == a2,
            _ => false,
        }
    }
}

fn term_add(a: Term, b: Term) -> Term {
    let (x, y) = if a.key() <= b.key() { (a, b) } else { (b, a) };
    Rc::new(TermNode::Add(x, y))
}
fn term_xor(a: Term, b: Term) -> Term {
    let (x, y) = if a.key() <= b.key() { (a, b) } else { (b, a) };
    Rc::new(TermNode::Xor(x, y))
}
fn term_rot(a: Term, n: u8) -> Term {
    Rc::new(TermNode::Rot(a, n))
}

/// Rebuild a term with phi symbols renumbered by `map` (candidate index
/// -> word index).  Constants and rotate amounts pass through verbatim;
/// the commutative normalization is re-applied on the way up.
fn remap_term(t: &Term, map: &[Option<usize>]) -> Term {
    match t.as_ref() {
        TermNode::Phi(k) => match map.get(*k).copied().flatten() {
            Some(w) => Rc::new(TermNode::Phi(w)),
            None => t.clone(), // the counter's symbol: only legal if dead
        },
        TermNode::Const(_) => t.clone(),
        TermNode::Add(a, b) => term_add(remap_term(a, map), remap_term(b, map)),
        TermNode::Xor(a, b) => term_xor(remap_term(a, map), remap_term(b, map)),
        TermNode::Rot(a, n) => term_rot(remap_term(a, map), *n),
    }
}

// ===========================================================================
// Reference double round
// ===========================================================================

/// One ARX quarter round over terms, with the rotate amounts as parameters:
/// `k = [rot after (d^=a) #1, rot after (b^=c) #1, rot after (d^=a) #2,
/// rot after (b^=c) #2]`.  The op order itself (add/xor/rotate on the
/// a/b/c/d roles, rotates on b and d only) is the recognizer's class
/// definition — ChaCha, Salsa-style row/column rounds and BLAKE-like
/// variants share it; ciphers that rotate a/c or interleave differently
/// are (deliberately) not matched.
fn qr_k(t: &mut [Term; 16], a: usize, b: usize, c: usize, d: usize, k: &[u8; 4]) {
    t[a] = term_add(t[a].clone(), t[b].clone());
    t[d] = term_xor(t[d].clone(), t[a].clone());
    t[d] = term_rot(t[d].clone(), k[0]);
    t[c] = term_add(t[c].clone(), t[d].clone());
    t[b] = term_xor(t[b].clone(), t[c].clone());
    t[b] = term_rot(t[b].clone(), k[1]);
    t[a] = term_add(t[a].clone(), t[b].clone());
    t[d] = term_xor(t[d].clone(), t[a].clone());
    t[d] = term_rot(t[d].clone(), k[2]);
    t[c] = term_add(t[c].clone(), t[d].clone());
    t[b] = term_xor(t[b].clone(), t[c].clone());
    t[b] = term_rot(t[b].clone(), k[3]);
}

/// The 16 backedge terms of one ARX double round over symbols `Phi(0..16)`:
/// four lane-aligned quarter rounds with constants `r`, then four quarter
/// rounds whose (b, c, d) operands are lane-rotated by `(kb, kc, kd)`
/// within their role groups.  RFC 7539 ChaCha20 is the instance
/// `r = s = [16, 12, 8, 7], (kb, kc, kd) = (1, 2, 3)`.
fn reference_double_round(r: &[u8; 4], s: &[u8; 4], kb: u8, kc: u8, kd: u8) -> [Term; 16] {
    let mut t: [Term; 16] = core::array::from_fn(|k| Rc::new(TermNode::Phi(k)));
    for j in 0..4 {
        qr_k(&mut t, j, 4 + j, 8 + j, 12 + j, r);
    }
    for j in 0..4 {
        let b = 4 + (j + kb as usize) % 4;
        let c = 8 + (j + kc as usize) % 4;
        let d = 12 + (j + kd as usize) % 4;
        qr_k(&mut t, j, b, c, d, s);
    }
    t
}

// ===========================================================================
// Proof: quarter-round structure recovery + reference comparison
// ===========================================================================

fn prove_arx_double_round(backedge: &[Term; 16]) -> Option<ArxShape> {
    // ---- 0. Index every Rot node by its child term's key.  A quarter
    //         round's structure is recovered by CONSTRUCTING the expected
    //         child terms (with the same commutative normalization the
    //         symbolic evaluator used) and reading off the unique rotate
    //         amount that wraps them — no assumptions about the cipher's
    //         constants, and no ambiguity: a child wrapped by two
    //         different amounts is not a clean QR and fails closed. ----
    let mut rot_amounts: FxHashMap<String, Vec<u8>> = FxHashMap::default();
    {
        fn index_rots(t: &Term, out: &mut FxHashMap<String, Vec<u8>>) {
            if let TermNode::Rot(x, n) = t.as_ref() {
                out.entry(TermNode::key(x.as_ref())).or_default().push(*n);
            }
            match t.as_ref() {
                TermNode::Phi(_) | TermNode::Const(_) => {}
                TermNode::Add(a, b) | TermNode::Xor(a, b) => {
                    index_rots(a, out);
                    index_rots(b, out);
                }
                TermNode::Rot(x, _) => index_rots(x, out),
            }
        }
        for t in backedge.iter() {
            index_rots(t, &mut rot_amounts);
        }
    }
    let unique_rot = |child: &Term| -> Option<u8> {
        let amounts = rot_amounts.get(&TermNode::key(child.as_ref()))?;
        let mut uniq = amounts.clone();
        uniq.sort_unstable();
        uniq.dedup();
        if uniq.len() == 1 { Some(uniq[0]) } else { None }
    };

    // ---- 1. Lane-aligned QR candidates: `Rot(Xor(phi_d, Add(phi_a,
    //         phi_b)), n)` for ANY rotate amount n — only the first
    //         group's quarter rounds see plain phis at their first rotate
    //         (the second group's operands are first-group results). ----
    let mut columns: Vec<([usize; 3], u8)> = Vec::new(); // [a, b, d] phis, rot amount
    for root in backedge.iter() {
        scan_column_shapes(root, &mut columns);
    }
    if columns.len() != 4 {
        arx_dbg(&format!("column census: {}", columns.len()));
        return None;
    }
    // Lane-parallel: the four first-group QRs share their first rotate.
    let r1 = columns[0].1;
    if columns.iter().any(|c| c.1 != r1) {
        arx_dbg("column rotate amounts differ");
        return None;
    }
    // The 12 a/b/d inputs must be 12 DISTINCT phis (the c inputs make it
    // a bijection onto all 16).
    {
        let mut seen = [false; 16];
        for (c, _) in columns.iter() {
            for &k in c.iter() {
                if seen[k] {
                    return None;
                }
                seen[k] = true;
            }
        }
    }

    // ---- 2. Search: column assignment x c assignment x a/b orientation.
    //         The Add under a first-group QR's first rotate is stored in
    //         canonical (key-sorted) order, which does NOT preserve which
    //         operand was the semantic `a` and which the `b` — both
    //         orientations of each of the four columns must be tried.
    //         For each bijection, the two groups' rotate schedules are
    //         DERIVED structurally (key lookups) and the final structural
    //         comparison against the constructed reference is the proof —
    //         a wrong candidate simply fails it rather than miscompiling.
    let mut ref_memo: FxHashMap<([u8; 4], [u8; 4], u8, u8, u8), [Term; 16]> = FxHashMap::default();
    let mut perm = [0usize, 1, 2, 3];
    'cols: loop {
        let cphis: Vec<usize> = {
            let mut assigned = [false; 16];
            for j in 0..4 {
                let (c, _) = &columns[perm[j]];
                assigned[c[0]] = true;
                assigned[c[1]] = true;
                assigned[c[2]] = true;
            }
            (0..16).filter(|&k| !assigned[k]).collect()
        };
        debug_assert_eq!(cphis.len(), 4);
        let mut cperm = [0usize, 1, 2, 3];
        'cs: loop {
            for abflip in 0u32..16 {
                let mut phi_of_x = [0usize; 16];
                {
                    let mut ok = true;
                    {
                        let mut seen = [false; 16];
                        for j in 0..4 {
                            let (c, _) = &columns[perm[j]];
                            let (a, b) = if abflip & (1 << j) != 0 {
                                (c[1], c[0])
                            } else {
                                (c[0], c[1])
                            };
                            phi_of_x[j] = a;
                            phi_of_x[4 + j] = b;
                            phi_of_x[8 + j] = cphis[cperm[j]];
                            phi_of_x[12 + j] = c[2];
                        }
                        for &p in phi_of_x.iter() {
                            if seen[p] {
                                ok = false;
                                break;
                            }
                            seen[p] = true;
                        }
                    }
                    if !ok {
                        continue;
                    }
                }
                let p = |x: usize| -> Term { Rc::new(TermNode::Phi(phi_of_x[x])) };

                // ---- 2a. Derive the first group's schedule and the
                //      per-word first-round RESULT terms, column by
                //      column.  All four columns must agree on the
                //      schedule (lane-parallelism). ----
                let mut first_res: Vec<Term> = Vec::with_capacity(16);
                let mut r = [0u8; 4];
                let mut derived = true;
                for j in 0..4 {
                    // QR op sequence with intermediates: a1 = a+b (op1's
                    // add), a2 = a1+b1 (op3's add — a's CURRENT value, not
                    // the raw phi).
                    let a1 = term_add(p(j), p(4 + j));
                    let d1 = term_rot(term_xor(p(12 + j), a1.clone()), r1);
                    let c1 = term_add(p(8 + j), d1.clone());
                    let Some(n1) = unique_rot(&term_xor(p(4 + j), c1.clone())) else {
                        derived = false;
                        break;
                    };
                    let b1 = term_rot(term_xor(p(4 + j), c1.clone()), n1);
                    let a2 = term_add(a1, b1.clone());
                    let Some(n2) = unique_rot(&term_xor(d1.clone(), a2.clone())) else {
                        derived = false;
                        break;
                    };
                    let d2 = term_rot(term_xor(d1, a2.clone()), n2);
                    let c2 = term_add(c1, d2.clone());
                    let Some(n3) = unique_rot(&term_xor(b1.clone(), c2.clone())) else {
                        derived = false;
                        break;
                    };
                    let b2 = term_rot(term_xor(b1, c2.clone()), n3);
                    if j == 0 {
                        r = [r1, n1, n2, n3];
                    } else if r != [r1, n1, n2, n3] {
                        derived = false;
                        break;
                    }
                    first_res.push(a2);
                    first_res.push(b2);
                    first_res.push(c2);
                    first_res.push(d2);
                }
                if !derived {
                    continue;
                }
                // first_res is grouped per column j as (a, b, c, d) at
                // 4*j..4*j+4; re-index to x order.
                let mut res_x: Vec<Term> = vec![first_res[0].clone(); 16];
                for j in 0..4 {
                    res_x[j] = first_res[4 * j].clone();
                    res_x[4 + j] = first_res[4 * j + 1].clone();
                    res_x[8 + j] = first_res[4 * j + 2].clone();
                    res_x[12 + j] = first_res[4 * j + 3].clone();
                }

                // ---- 2b. The second group's lane offsets and schedule:
                //      try every (kb, kc, kd); the correct one is the
                //      whose QR wiring exists in the term forest. ----
                for kb in 0u8..4 {
                    for kc in 0u8..4 {
                        for kd in 0u8..4 {
                            let mut s = [0u8; 4];
                            let mut ok2 = true;
                            for j in 0..4usize {
                                let a = res_x[j].clone();
                                let b = res_x[4 + (j + kb as usize) % 4].clone();
                                let c = res_x[8 + (j + kc as usize) % 4].clone();
                                let d = res_x[12 + (j + kd as usize) % 4].clone();
                                let a1p = term_add(a.clone(), b.clone());
                                let d1p_child = term_xor(d.clone(), a1p.clone());
                                let Some(m0) = unique_rot(&d1p_child) else {
                                    ok2 = false;
                                    break;
                                };
                                let d1p = term_rot(d1p_child, m0);
                                let c1p = term_add(c.clone(), d1p.clone());
                                let Some(m1) = unique_rot(&term_xor(b.clone(), c1p.clone())) else {
                                    ok2 = false;
                                    break;
                                };
                                let b1p = term_rot(term_xor(b, c1p.clone()), m1);
                                let a2p = term_add(a1p, b1p.clone());
                                let Some(m2) = unique_rot(&term_xor(d1p.clone(), a2p.clone()))
                                else {
                                    ok2 = false;
                                    break;
                                };
                                let d2p = term_rot(term_xor(d1p, a2p), m2);
                                let c2p = term_add(c1p, d2p.clone());
                                let Some(m3) = unique_rot(&term_xor(b1p, c2p)) else {
                                    ok2 = false;
                                    break;
                                };
                                if j == 0 {
                                    s = [m0, m1, m2, m3];
                                } else if s != [m0, m1, m2, m3] {
                                    ok2 = false;
                                    break;
                                }
                            }
                            if !ok2 {
                                continue;
                            }
                            // ---- The proof: reference vs actual backedge
                            //      terms, renamed to x indices. ----
                            let reference = ref_memo
                                .entry((r, s, kb, kc, kd))
                                .or_insert_with(|| reference_double_round(&r, &s, kb, kc, kd));
                            let mut to_x: Vec<Option<usize>> = vec![None; 16];
                            for xk in 0..16 {
                                to_x[phi_of_x[xk]] = Some(xk);
                            }
                            let mut ok3 = true;
                            for xk in 0..16 {
                                let renamed = remap_term(&backedge[phi_of_x[xk]], &to_x);
                                if *renamed != *reference[xk] {
                                    ok3 = false;
                                    break;
                                }
                            }
                            if ok3 {
                                return Some(ArxShape {
                                    phi_of_x,
                                    r,
                                    s,
                                    kb,
                                    kc,
                                    kd,
                                });
                            }
                        }
                    }
                }
            }
            if !next_perm(&mut cperm) {
                break 'cs;
            }
        }
        if !next_perm(&mut perm) {
            break 'cols;
        }
    }
    None
}

/// The recognized double-round shape: the word-to-role bijection, the two
/// groups' rotate schedules, and the second group's lane offsets.
struct ArxShape {
    phi_of_x: [usize; 16],
    /// First (lane-aligned) group: [rot(d) after (d^=a) #1, rot(b) after
    /// (b^=c) #1, rot(d) after (d^=a) #2, rot(b) after (b^=c) #2].
    r: [u8; 4],
    /// Second (lane-rotated) group, same layout.
    s: [u8; 4],
    /// Lane offsets of the second group's (b, c, d) roles.
    kb: u8,
    kc: u8,
    kd: u8,
}

fn scan_column_shapes(t: &Term, out: &mut Vec<([usize; 3], u8)>) {
    if let TermNode::Rot(x1, n) = t.as_ref() {
        if let TermNode::Xor(l, r) = x1.as_ref() {
            for (d, s1) in [(l, r), (r, l)] {
                if let TermNode::Add(a, b) = s1.as_ref() {
                    if let (Some(ai), Some(bi), Some(di)) =
                        (phi_idx_of(a), phi_idx_of(b), phi_idx_of(d))
                    {
                        // Shared subterms are visited once per path; a
                        // distinct column QR appears exactly once.  The
                        // rotate amount is arbitrary (cipher constant).
                        let triple = [ai, bi, di];
                        if !out.iter().any(|(c, _)| *c == triple) {
                            out.push((triple, *n));
                        }
                        break;
                    }
                }
            }
        }
    }
    match t.as_ref() {
        TermNode::Phi(_) | TermNode::Const(_) => {}
        TermNode::Add(a, b) | TermNode::Xor(a, b) => {
            scan_column_shapes(a, out);
            scan_column_shapes(b, out);
        }
        TermNode::Rot(x, _) => scan_column_shapes(x, out),
    }
}

fn phi_idx_of(t: &Term) -> Option<usize> {
    match t.as_ref() {
        TermNode::Phi(k) => Some(*k),
        _ => None,
    }
}

/// In-place next lexicographic permutation.
fn next_perm(p: &mut [usize]) -> bool {
    let n = p.len();
    let mut i = n as isize - 2;
    while i >= 0 && p[i as usize] >= p[i as usize + 1] {
        i -= 1;
    }
    if i < 0 {
        return false;
    }
    let mut j = n - 1;
    while p[j] <= p[i as usize] {
        j -= 1;
    }
    p.swap(i as usize, j);
    p[i as usize + 1..].reverse();
    true
}

// ===========================================================================
// Loop analysis
// ===========================================================================

/// A matched ARX loop, with every piece the transform needs.  All word
/// data is indexed by x position (`x0..x15`), per `phi_of_x`.
struct ArxLoop {
    header_idx: usize,
    preheader_idx: usize,
    latch_idx: usize,
    /// Blocks in the loop (header included).
    body: FxHashSet<usize>,
    /// The loop-carried state word phis, indexed by x position.
    word_phis: [Value; 16],
    /// Word-phi initial values (incoming from the preheader edge).
    word_inits: [Operand; 16],
    /// Value -> term for every ARX-classified instruction in the loop
    /// (including the copy/cast chains; these all become dead).
    term_values: FxHashMap<Value, Term>,
    /// The body blocks (header excluded), fewest-instructions first.
    order: Vec<usize>,
    /// The value feeding the loop-counter phi from the latch (the `i+1`
    /// add) — retained verbatim by the transform.
    counter_next: Value,
    /// The proved double-round shape: rotate schedules of both groups and
    /// the second group's lane offsets (the word bijection is already
    /// applied to `word_phis`/`word_inits`).
    shape_r: [u8; 4],
    shape_s: [u8; 4],
    shape_kb: u8,
    shape_kc: u8,
    shape_kd: u8,
}

/// Evaluate one loop against the ARX pattern.  Returns `None` (loop stays
/// scalar) for any shape deviation, however small: purity of the loop body
/// is the soundness foundation — every instruction must be a lane-exact
/// u32 operation whose value either feeds the backedge phis or is dead.
fn analyze_arx_loop(
    func: &IrFunction,
    cfg: &CfgAnalysis,
    lp: &loop_analysis::NaturalLoop,
) -> Option<ArxLoop> {
    let header = &func.blocks[lp.header];

    // ---- Single latch + preheader. ----
    let latch_candidates = lp.latches(&cfg.preds);
    if latch_candidates.len() != 1 {
        arx_dbg("multi-latch");
        return None;
    }
    let latch_idx = latch_candidates[0];
    let latch_label = func.blocks[latch_idx].label;
    let preheader_idx = match lp.find_preheader(&cfg.preds) {
        Some(p) => p,
        None => {
            arx_dbg("no preheader");
            return None;
        }
    };
    let preheader_label = func.blocks[preheader_idx].label;

    // ---- Header: phis (classify by edge labels), exit compare, branch. ----
    struct CandPhi {
        dest: Value,
        init: Operand,
        backedge: Operand,
    }
    let mut candidates: Vec<CandPhi> = Vec::new();
    let mut other_phis = 0usize;
    for inst in header.instructions.iter() {
        match inst {
            Instruction::Phi { dest, ty, incoming } => {
                if incoming.len() != 2 {
                    arx_dbg("phi with != 2 incoming");
                    return None;
                }
                let mut init = None;
                let mut back = None;
                for (op, lbl) in incoming.iter() {
                    if *lbl == preheader_label {
                        init = Some(op.clone());
                    } else if *lbl == latch_label {
                        back = Some(op.clone());
                    } else {
                        arx_dbg("phi edge from neither preheader nor latch");
                        return None;
                    }
                }
                let (Some(init), Some(back)) = (init, back) else {
                    arx_dbg("phi missing an edge");
                    return None;
                };
                match ty {
                    IrType::U32 | IrType::I32 => candidates.push(CandPhi {
                        dest: *dest,
                        init,
                        backedge: back,
                    }),
                    _ => other_phis += 1,
                }
            }
            Instruction::Cmp { .. } | Instruction::Copy { .. } | Instruction::Cast { .. } => {}
            _ => return None,
        }
    }
    // The loop counter is exactly one non-word phi (I32 counters, possibly
    // I64); more means a shape we do not model.
    // Census: the 16 state words plus the loop counter.  The counter is
    // I32 (same type class as U32/I32 words, so it counts as a candidate)
    // or I64 (then it is the lone "other" phi).  Which candidate IS the
    // counter is decided after the symbolic evaluation, by its canonical
    // `self + const` backedge shape.
    if !(candidates.len() == 17 && other_phis == 0) && !(candidates.len() == 16 && other_phis == 1)
    {
        arx_dbg(&format!(
            "phi census: {} candidates, {} other phis",
            candidates.len(),
            other_phis
        ));
        return None;
    }

    // ---- Body: ARX purity + symbolic evaluation to fixpoint. ----
    let mut order: Vec<usize> = lp
        .body
        .iter()
        .copied()
        .filter(|&b| b != lp.header)
        .collect();
    order.sort_by_key(|&b| func.blocks[b].instructions.len());

    let mut terms: FxHashMap<Value, Term> = FxHashMap::default();
    // Symbol numbering: every candidate phi gets a symbol Phi(0..n).
    // The counter candidate's `self + 1` backedge evaluates to a trivial
    // self-referential Add term and is recognized (and discarded) in the
    // classification below; if the counter VALUE ever feeds the word
    // dataflow, the reference proof fails closed on its own.
    for (k, cand) in candidates.iter().enumerate() {
        terms.insert(cand.dest, Rc::new(TermNode::Phi(k)));
    }
    let mut counter_phi: Option<Value> = None;
    let mut word_backedge_term: Vec<Option<Term>> = vec![None; candidates.len()];
    let mut evaluated_all: FxHashSet<(usize, usize)> = FxHashSet::default();
    let mut iv_insts: FxHashSet<(usize, usize)> = FxHashSet::default();

    fn operand_term(op: &Operand, terms: &FxHashMap<Value, Term>) -> Option<Term> {
        match op {
            Operand::Const(c) => c.to_i64().map(|n| Rc::new(TermNode::Const(n as u32))),
            Operand::Value(v) => terms.get(v).cloned(),
        }
    }

    // Definition index over the body (for the rotate-idiom recognizer:
    // bit_idioms may not have run yet at this pipeline point, so the
    // matcher itself understands `(x << n) | (x >> (32-n))`).
    let mut def_map: FxHashMap<Value, &Instruction> = FxHashMap::default();
    for &bi in order.iter() {
        for inst in func.blocks[bi].instructions.iter() {
            if let Some(d) = inst.dest() {
                def_map.insert(d, inst);
            }
        }
    }
    /// Follow Cast/Copy chains from a value to the instruction that
    /// defines its dataflow (the idiom's shifts hide behind width casts).
    fn resolve<'a>(
        def_map: &FxHashMap<Value, &'a Instruction>,
        v: Value,
    ) -> Option<&'a Instruction> {
        let mut cur = def_map.get(&v).copied()?;
        for _ in 0..8 {
            match cur {
                Instruction::Cast { src, .. } | Instruction::Copy { src, .. } => {
                    cur = def_map.get(&src.as_value()?)?;
                }
                _ => return Some(cur),
            }
        }
        None
    }

    // Destinations consumed by a recognized rotate idiom (Or + shifts +
    // intermediate casts): all dead once the Rot term exists.
    let mut idiom_members: FxHashSet<Value> = FxHashSet::default();
    let mut progress = true;
    while progress {
        progress = false;
        // Rotate idiom: Or(Shl(x, n), LShr(x, 32-n)) == Rot(x, n), with
        // casts between the shifts and the Or tolerated on both sides.
        for &bi in order.iter() {
            for inst in func.blocks[bi].instructions.iter() {
                let Instruction::BinOp {
                    dest,
                    op: IrBinOp::Or,
                    lhs,
                    rhs,
                    ..
                } = inst
                else {
                    continue;
                };
                if terms.contains_key(dest) {
                    continue;
                }
                let side = |op: &Operand| -> Option<(Value, i64, bool)> {
                    // (x, amount, is_shl), through casts.
                    let v = op.as_value()?;
                    let def = resolve(&def_map, v)?;
                    let Instruction::BinOp {
                        op: sop,
                        lhs: x,
                        rhs: amt,
                        ..
                    } = def
                    else {
                        return None;
                    };
                    let is_shl = match sop {
                        IrBinOp::Shl => true,
                        IrBinOp::LShr => false,
                        _ => return None,
                    };
                    let n = match amt {
                        Operand::Const(c) => c.to_i64()?,
                        _ => return None,
                    };
                    Some((x.as_value()?, n, is_shl))
                };
                let (Some(l), Some(r)) = (side(lhs), side(rhs)) else {
                    continue;
                };
                let (x, n) = match (l, r) {
                    ((x, n, true), (x2, m, false)) if x == x2 && n + m == 32 => (x, n),
                    ((x, m, false), (x2, n, true)) if x == x2 && n + m == 32 => (x, n),
                    _ => continue,
                };
                if !(1..31).contains(&n) {
                    continue;
                }
                if let Some(xt) = terms.get(&x).cloned() {
                    terms.insert(*dest, term_rot(xt, n as u8));
                    idiom_members.insert(*dest);
                    // The shifts and every Cast/Copy on the path from
                    // them to the Or are members too.
                    for op in [lhs, rhs] {
                        let mut cur = op.as_value();
                        while let Some(v) = cur {
                            idiom_members.insert(v);
                            match def_map.get(&v) {
                                Some(Instruction::Cast { src, .. })
                                | Some(Instruction::Copy { src, .. }) => {
                                    cur = src.as_value();
                                }
                                _ => break,
                            }
                        }
                    }
                    progress = true;
                }
            }
        }
        for &bi in order.iter() {
            for (ii, inst) in func.blocks[bi].instructions.iter().enumerate() {
                if evaluated_all.contains(&(bi, ii)) || iv_insts.contains(&(bi, ii)) {
                    continue;
                }
                let evaluated = match inst {
                    Instruction::Copy { dest, src } => {
                        operand_term(src, &terms).map(|t| (*dest, t))
                    }
                    Instruction::Cast { dest, src, .. } => {
                        operand_term(src, &terms).map(|t| (*dest, t))
                    }
                    Instruction::BinOp {
                        dest,
                        op,
                        lhs,
                        rhs,
                        ty,
                    } if matches!(ty, IrType::U32 | IrType::I32) => match op {
                        IrBinOp::Add | IrBinOp::Xor => {
                            let l = operand_term(lhs, &terms);
                            let r = operand_term(rhs, &terms);
                            match (l, r, op) {
                                (Some(l), Some(r), IrBinOp::Add) => Some((*dest, term_add(l, r))),
                                (Some(l), Some(r), IrBinOp::Xor) => Some((*dest, term_xor(l, r))),
                                _ => None,
                            }
                        }
                        IrBinOp::RotateLeft => {
                            let l = operand_term(lhs, &terms);
                            let n = match rhs {
                                Operand::Const(c) => c.to_i64(),
                                _ => None,
                            };
                            match (l, n) {
                                (Some(l), Some(n)) if (1..=31).contains(&n) => {
                                    Some((*dest, term_rot(l, n as u8)))
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    },
                    // The loop-counter increment: I32/I64 add that is not
                    // part of the word dataflow.  Tolerated, retained.
                    Instruction::BinOp {
                        op: IrBinOp::Add,
                        ty: IrType::I32 | IrType::I64,
                        ..
                    } => {
                        // Neither operand is a word value (words are
                        // U32-typed dataflow; if the counter chain ever
                        // mixes in a word term the scan below rejects).
                        iv_insts.insert((bi, ii));
                        progress = true;
                        None
                    }
                    _ => None,
                };
                match evaluated {
                    Some((dest, t)) => {
                        terms.insert(dest, t);
                        evaluated_all.insert((bi, ii));
                        progress = true;
                    }
                    None => {
                        // Not evaluable THIS round: could be an IV feed,
                        // a rotate-idiom member (fires in a later round,
                        // once the shifted value's term exists), or an
                        // instruction this matcher does not understand at
                        // all.  Defer the verdict to the classification
                        // scan after the fixpoint converges.
                    }
                }
            }
        }
    }

    // ---- Post-fixpoint classification: every body instruction must be
    //      an evaluated ARX term, an IV feed, or a rotate-idiom member.
    //      Anything else fails the whole match (fail closed). ----
    for &bi in order.iter() {
        for (ii, inst) in func.blocks[bi].instructions.iter().enumerate() {
            if evaluated_all.contains(&(bi, ii)) || iv_insts.contains(&(bi, ii)) {
                continue;
            }
            let idiom = inst
                .dest()
                .map(|d| idiom_members.contains(&d))
                .unwrap_or(false);
            if !idiom {
                arx_dbg(&format!(
                    "unclassified instruction after fixpoint: {}",
                    match inst {
                        Instruction::BinOp { op, .. } => format!("BinOp {op:?}"),
                        other => format!("{:?}", std::mem::discriminant(other)),
                    }
                ));
                return None;
            }
        }
    }

    // ---- Classify the candidate phis: 16 words + 1 counter. ----
    // The counter is the candidate whose backedge term is exactly
    // `Add(Phi(self), Const(c))` — the canonical induction shape.  Its
    // incoming-from-preheader value and its uses are left untouched: the
    // loop keeps its counter, compare and branch verbatim.
    if other_phis == 1 {
        // I64 counter: it was never a candidate; pick it out of the
        // header phis by elimination.
        let mut counter = None;
        for inst in header.instructions.iter() {
            if let Instruction::Phi { dest, ty, incoming } = inst {
                if !matches!(ty, IrType::U32 | IrType::I32) && incoming.len() == 2 {
                    counter = Some(*dest);
                }
            }
        }
        counter_phi = counter;
    }
    let mut counters_found = 0usize;
    for (k, cand) in candidates.iter().enumerate() {
        let v = match cand.backedge.as_value() {
            Some(v) => v,
            None => {
                // Non-value backedge (constant): cannot be a word update;
                // acceptable only as the counter shape if it also is not
                // self+const... a constant backedge is no counter.  Reject.
                arx_dbg("constant backedge on a candidate phi");
                return None;
            }
        };
        let t = match terms.get(&v) {
            Some(t) => t.clone(),
            None => {
                arx_dbg("candidate backedge has no term");
                return None;
            }
        };
        let is_counter = match t.as_ref() {
            TermNode::Add(l, r) => {
                let own = Rc::new(TermNode::Phi(k));
                let const_side = matches!(r.as_ref(), TermNode::Const(_));
                (**l == *own && const_side)
                    || matches!(l.as_ref(), TermNode::Const(_)) && **r == *own
            }
            _ => false,
        };
        if is_counter {
            counters_found += 1;
            counter_phi = Some(cand.dest);
        } else {
            word_backedge_term[k] = Some(t);
        }
    }
    if counters_found != 1 || counter_phi.is_none() {
        arx_dbg(&format!(
            "counter classification: {counters_found} counters found"
        ));
        return None;
    }
    if word_backedge_term.iter().filter(|t| t.is_none()).count() != 1 {
        arx_dbg(&format!(
            "backedge term census: {} without terms",
            word_backedge_term.iter().filter(|t| t.is_none()).count()
        ));
        return None;
    }
    let counter_phi = counter_phi?;

    // Word phis + their backedge terms, renumbered into word order
    // (candidate indices include the counter's slot; the reference and
    // the proof speak in word indices 0..16).
    let mut remap: Vec<Option<usize>> = vec![None; candidates.len()];
    let mut word_phis = [Value(0); 16];
    let mut word_inits = [Operand::Const(IrConst::I32(0)); 16];
    let mut backedge: [Term; 16] = core::array::from_fn(|_| Rc::new(TermNode::Const(0)));
    let mut w = 0usize;
    for (k, cand) in candidates.iter().enumerate() {
        if cand.dest == counter_phi {
            continue;
        }
        let Some(t) = word_backedge_term[k].clone() else {
            arx_dbg("word phi without backedge term");
            return None;
        };
        remap[k] = Some(w);
        word_phis[w] = cand.dest;
        word_inits[w] = cand.init.clone();
        backedge[w] = t;
        w += 1;
    }
    debug_assert_eq!(w, 16);
    for t in backedge.iter_mut() {
        *t = remap_term(t, &remap);
    }

    // ---- The proof. ----
    let shape = match prove_arx_double_round(&backedge) {
        Some(s) => s,
        None => {
            arx_dbg("term proof failed");
            return None;
        }
    };
    let phi_of_x = shape.phi_of_x;

    // Reindex into x order.
    let mut xs_phis = [Value(0); 16];
    let mut xs_inits = [Operand::Const(IrConst::I32(0)); 16];
    for xk in 0..16 {
        xs_phis[xk] = word_phis[phi_of_x[xk]];
        xs_inits[xk] = word_inits[phi_of_x[xk]].clone();
    }

    // ---- Pre-scan: every use of a word phi OUTSIDE the loop must live in
    //      an instruction the rewriter understands (fail closed BEFORE any
    //      mutation). ----
    for (bi, blk) in func.blocks.iter().enumerate() {
        if lp.body.contains(&bi) {
            continue;
        }
        for inst in blk.instructions.iter() {
            if instruction_uses_any(inst, &xs_phis) && !rewritable_kind(inst) {
                arx_dbg("outside use in non-rewritable instruction");
                return None;
            }
        }
    }

    // The counter's backedge value (its defining add is retained by the
    // transform; it carries a trivial self+const TERM but nothing in the
    // kernel consumes it).
    let counter_next = candidates
        .iter()
        .find(|c| c.dest == counter_phi)
        .and_then(|c| c.backedge.as_value())?;

    Some(ArxLoop {
        header_idx: lp.header,
        preheader_idx,
        latch_idx,
        body: lp.body.clone(),
        word_phis: xs_phis,
        word_inits: xs_inits,
        term_values: terms,
        order,
        counter_next,
        shape_r: shape.r,
        shape_s: shape.s,
        shape_kb: shape.kb,
        shape_kc: shape.kc,
        shape_kd: shape.kd,
    })
}

/// Does this instruction reference any of the given values as an operand?
fn instruction_uses_any(inst: &Instruction, vals: &[Value; 16]) -> bool {
    let hit = |op: &Operand| matches!(op, Operand::Value(v) if vals.contains(v));
    match inst {
        Instruction::BinOp { lhs, rhs, .. } | Instruction::Cmp { lhs, rhs, .. } => {
            hit(lhs) || hit(rhs)
        }
        Instruction::Copy { src, .. } | Instruction::Cast { src, .. } => hit(src),
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ..
        } => hit(cond) || hit(true_val) || hit(false_val),
        Instruction::Store { val, .. } => hit(val),
        Instruction::Phi { incoming, .. } => incoming.iter().any(|(op, _)| hit(op)),
        Instruction::Intrinsic { args, .. } => args.iter().any(hit),
        Instruction::GetElementPtr { offset, .. } => hit(offset),
        _ => false,
    }
}

/// Instruction kinds whose value operands the post-loop rewriter handles.
fn rewritable_kind(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::BinOp { .. }
            | Instruction::Cmp { .. }
            | Instruction::Copy { .. }
            | Instruction::Cast { .. }
            | Instruction::Select { .. }
            | Instruction::Store { .. }
            | Instruction::Phi { .. }
            | Instruction::Intrinsic { .. }
            | Instruction::GetElementPtr { .. }
    )
}

/// Rewrite every value operand of the instruction through `f`.
fn rewrite_operands(inst: &mut Instruction, f: &mut dyn FnMut(&mut Operand)) {
    match inst {
        Instruction::BinOp { lhs, rhs, .. } | Instruction::Cmp { lhs, rhs, .. } => {
            f(lhs);
            f(rhs);
        }
        Instruction::Copy { src, .. } | Instruction::Cast { src, .. } => f(src),
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ..
        } => {
            f(cond);
            f(true_val);
            f(false_val);
        }
        Instruction::Store { val, .. } => f(val),
        Instruction::Phi { incoming, .. } => {
            for (op, _) in incoming.iter_mut() {
                f(op);
            }
        }
        Instruction::Intrinsic { args, .. } => {
            for a in args.iter_mut() {
                f(a);
            }
        }
        Instruction::GetElementPtr { offset, .. } => f(offset),
        _ => {}
    }
}

impl Operand {
    fn as_value(&self) -> Option<Value> {
        match self {
            Operand::Value(v) => Some(*v),
            _ => None,
        }
    }
}

// ===========================================================================
// Transform
// ===========================================================================

/// `pshufd` immediates for lane rotations: dest lane j = src lane
/// (j + k) % 4.  0x39 / 0x4E / 0x93 — the OpenSSL `_MM_SHUFFLE` forms.
const LANEROT_1: i64 = 0x39;
const LANEROT_2: i64 = 0x4E;
const LANEROT_3: i64 = 0x93;

/// 16-byte pshufb mask packed as four little-endian I32 lanes.
fn mask_words(bytes: [u8; 16]) -> [i64; 4] {
    let mut w = [0i64; 4];
    for (i, b) in bytes.iter().enumerate() {
        w[i / 4] |= (*b as i64) << (8 * (i % 4));
    }
    w
}

/// pshufb mask for a left rotation by `k` whole bytes per dword
/// (k = 1..=3, i.e. 8/16/24-bit rotates).  pshufb byte j of the result
/// comes from source byte mask[j], so a left rotation by one byte needs
/// [3, 0, 1, 2] per dword (NOT [1, 2, 3, 0], which is a right rotation —
/// verified against hardware pshufb semantics).
fn byterot_bytes(k: u8) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    for lane in 0..4usize {
        for j in 0..4usize {
            bytes[4 * lane + j] = (4 * lane + (j + 4 - k as usize) % 4) as u8;
        }
    }
    bytes
}

fn transform_arx_loop(
    func: &mut IrFunction,
    cfg: &CfgAnalysis,
    arx: &ArxLoop,
    use_pshufb: bool,
    debug: bool,
) -> usize {
    let mut next_val_id = func.next_value_id;

    let preheader_label = func.blocks[arx.preheader_idx].label;
    let latch_label = func.blocks[arx.latch_idx].label;

    // ---- CFG shape validation, BEFORE the first mutation (the three
    //      fail-closed checks from the integration audit of PR #455: the
    //      original resolved the exit only AFTER inserting the preheader
    //      packs, header phis and the kernel body, so a do-while /
    //      latch-exit loop whose header ends in `Branch` sailed through
    //      analysis and then hit a mid-mutation bail-out that left the
    //      function in a half-transformed, dangling-SSA state). ----
    // (1) The header must end in a CondBranch (guard-at-top form): the
    //     extracts and the outside-use rewrite below assume the exit edge
    //     leaves from the header.
    // (2) Exit polarity: exactly one CondBranch target must be OUTSIDE the
    //     loop body — that block is the exit, whichever edge it is.  Both
    //     targets inside (the exit lives elsewhere in the loop) and both
    //     outside (degenerate) decline.
    // (3) Every predecessor of the exit block must be the loop header: an
    //     edge entering the exit from outside this loop never flowed
    //     through the header, so the vector phis — and the extracts
    //     reading them — would not dominate that path.  Decline rather
    //     than edge-split.
    let exit_idx = {
        let (t_lbl, f_lbl) = match &func.blocks[arx.header_idx].terminator {
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => (*true_label, *false_label),
            _ => {
                arx_dbg("header terminator is not a CondBranch (latch-exit loop)");
                return 0;
            }
        };
        let idx_of = |lbl: BlockId| func.blocks.iter().position(|b| b.label == lbl);
        let (Some(t_idx), Some(f_idx)) = (idx_of(t_lbl), idx_of(f_lbl)) else {
            return 0;
        };
        let exit_idx = match (arx.body.contains(&t_idx), arx.body.contains(&f_idx)) {
            (false, true) => t_idx,
            (true, false) => f_idx,
            _ => {
                arx_dbg("header CondBranch does not leave the loop on exactly one edge");
                return 0;
            }
        };
        let bad_pred = cfg
            .preds
            .row(exit_idx)
            .iter()
            .any(|&p| p as usize != arx.header_idx);
        if bad_pred {
            arx_dbg("exit block has a predecessor other than the loop header");
            return 0;
        }
        exit_idx
    };

    // The main ARX body block: the one holding the most instructions.
    let main_block = *arx
        .order
        .iter()
        .max_by_key(|&&b| func.blocks[b].instructions.len())
        .expect("non-empty body");

    // ---- Preheader: four role packs (+ two pshufb masks). ----
    let mut pre_insts: Vec<Instruction> = Vec::new();
    fn pack_into(vals: [Operand; 4], pre: &mut Vec<Instruction>, next_val_id: &mut u32) -> Value {
        let dest = Value(*next_val_id);
        *next_val_id += 1;
        pre.push(Instruction::Intrinsic {
            dest: Some(dest),
            op: IntrinsicOp::VecPackI32x4,
            dest_ptr: None,
            args: vals.to_vec(),
        });
        dest
    }
    let pack_a = pack_into(
        [
            arx.word_inits[0].clone(),
            arx.word_inits[1].clone(),
            arx.word_inits[2].clone(),
            arx.word_inits[3].clone(),
        ],
        &mut pre_insts,
        &mut next_val_id,
    );
    let pack_b = pack_into(
        [
            arx.word_inits[4].clone(),
            arx.word_inits[5].clone(),
            arx.word_inits[6].clone(),
            arx.word_inits[7].clone(),
        ],
        &mut pre_insts,
        &mut next_val_id,
    );
    let pack_c = pack_into(
        [
            arx.word_inits[8].clone(),
            arx.word_inits[9].clone(),
            arx.word_inits[10].clone(),
            arx.word_inits[11].clone(),
        ],
        &mut pre_insts,
        &mut next_val_id,
    );
    let pack_d = pack_into(
        [
            arx.word_inits[12].clone(),
            arx.word_inits[13].clone(),
            arx.word_inits[14].clone(),
            arx.word_inits[15].clone(),
        ],
        &mut pre_insts,
        &mut next_val_id,
    );
    // Loop-invariant pshufb masks, one per DISTINCT whole-byte rotate
    // amount used by either group (SSSE3+; the profile exposes SSE4.1,
    // which implies SSSE3).  Non-byte rotates (and the SSE2 fallback)
    // use the 3-instruction pslld/psrld/por form instead.
    let mut rot_masks: FxHashMap<u8, Value> = FxHashMap::default();
    if use_pshufb {
        fn mask_into(bytes: [u8; 16], pre: &mut Vec<Instruction>, next_val_id: &mut u32) -> Value {
            let w = mask_words(bytes);
            let dest = Value(*next_val_id);
            *next_val_id += 1;
            pre.push(Instruction::Intrinsic {
                dest: Some(dest),
                op: IntrinsicOp::VecPackI32x4,
                dest_ptr: None,
                args: w
                    .iter()
                    .map(|&x| Operand::Const(IrConst::I32(x as i32)))
                    .collect(),
            });
            dest
        }
        let mut amounts: Vec<u8> = arx
            .shape_r
            .iter()
            .chain(arx.shape_s.iter())
            .copied()
            .filter(|&n| n % 8 == 0 && (1..=3).contains(&(n / 8)))
            .collect();
        amounts.sort_unstable();
        amounts.dedup();
        for n in amounts {
            let m = mask_into(byterot_bytes(n / 8), &mut pre_insts, &mut next_val_id);
            rot_masks.insert(n, m);
        }
    }
    func.blocks[arx.preheader_idx]
        .instructions
        .extend(pre_insts);

    // ---- Header: four vector phis (backedge values named upfront; the
    //      kernel renames its final role values onto them). ----
    let phi_a = Value(next_val_id);
    let phi_b = Value(next_val_id + 1);
    let phi_c = Value(next_val_id + 2);
    let phi_d = Value(next_val_id + 3);
    let next_a = Value(next_val_id + 4);
    let next_b = Value(next_val_id + 5);
    let next_c = Value(next_val_id + 6);
    let next_d = Value(next_val_id + 7);
    next_val_id += 8;
    {
        let header = &mut func.blocks[arx.header_idx];
        let phi_pos = header
            .instructions
            .iter()
            .position(|i| !matches!(i, Instruction::Phi { .. }))
            .unwrap_or(header.instructions.len());
        for (dest, init, nxt) in [
            (phi_a, pack_a, next_a),
            (phi_b, pack_b, next_b),
            (phi_c, pack_c, next_c),
            (phi_d, pack_d, next_d),
        ] {
            header.instructions.insert(
                phi_pos,
                Instruction::Phi {
                    dest,
                    ty: IrType::I32,
                    incoming: vec![
                        (Operand::Value(init), preheader_label),
                        (Operand::Value(nxt), latch_label),
                    ],
                },
            );
        }
    }

    // ---- Kernel: the proven double round, four lanes at a time. ----
    // Every intrinsic below evaluates EXACTLY one node of the reference
    // term construction the matcher proved equal to the scalar loop.
    let mut k: Vec<Instruction> = Vec::with_capacity(64);
    fn kbin(
        op: IntrinsicOp,
        a: Value,
        b: Value,
        k: &mut Vec<Instruction>,
        next_val_id: &mut u32,
    ) -> Value {
        let dest = Value(*next_val_id);
        *next_val_id += 1;
        k.push(Instruction::Intrinsic {
            dest: Some(dest),
            op,
            dest_ptr: None,
            args: vec![Operand::Value(a), Operand::Value(b)],
        });
        dest
    }
    fn krot(v: Value, n: i64, k: &mut Vec<Instruction>, next_val_id: &mut u32) -> Value {
        let dest = Value(*next_val_id);
        *next_val_id += 1;
        k.push(Instruction::Intrinsic {
            dest: Some(dest),
            op: IntrinsicOp::VecRolI32x4,
            dest_ptr: None,
            args: vec![Operand::Value(v), Operand::Const(IrConst::I32(n as i32))],
        });
        dest
    }
    fn kshufb(v: Value, m: Value, k: &mut Vec<Instruction>, next_val_id: &mut u32) -> Value {
        let dest = Value(*next_val_id);
        *next_val_id += 1;
        k.push(Instruction::Intrinsic {
            dest: Some(dest),
            op: IntrinsicOp::VecShufbI32x4,
            dest_ptr: None,
            args: vec![Operand::Value(v), Operand::Value(m)],
        });
        dest
    }
    fn kshufd(v: Value, imm: i64, k: &mut Vec<Instruction>, next_val_id: &mut u32) -> Value {
        let dest = Value(*next_val_id);
        *next_val_id += 1;
        k.push(Instruction::Intrinsic {
            dest: Some(dest),
            op: IntrinsicOp::VecShufdI32x4,
            dest_ptr: None,
            args: vec![Operand::Value(v), Operand::Const(IrConst::I32(imm as i32))],
        });
        dest
    }

    // One SIMD quarter round over the current role registers (all four
    // scalar QRs at once); every intrinsic evaluates EXACTLY one node of
    // the reference term construction the matcher proved equal.  The
    // rotate amounts and the second group's lane offsets come from the
    // proved shape — arbitrary cipher constants, not just RFC 7539's.
    let lanerot = |k: u8| -> i64 {
        match k {
            1 => LANEROT_1,
            2 => LANEROT_2,
            3 => LANEROT_3,
            _ => unreachable!("lane offset 0 is skipped by the callers"),
        }
    };
    let mut a = phi_a;
    let mut b = phi_b;
    let mut c = phi_c;
    let mut d = phi_d;
    for round in 0..2 {
        let kcst: &[u8; 4] = if round == 0 {
            &arx.shape_r
        } else {
            &arx.shape_s
        };
        if round == 1 {
            // Lane-rotated group setup: rotate the b/c/d roles into the
            // proved offsets.  After this, lane j of (A, B, C, D) is
            // group-1 QR j's (a, b, c, d).  A zero offset is a no-op.
            if arx.shape_kb != 0 {
                b = kshufd(b, lanerot(arx.shape_kb), &mut k, &mut next_val_id);
            }
            if arx.shape_kc != 0 {
                c = kshufd(c, lanerot(arx.shape_kc), &mut k, &mut next_val_id);
            }
            if arx.shape_kd != 0 {
                d = kshufd(d, lanerot(arx.shape_kd), &mut k, &mut next_val_id);
            }
        }
        // a += b; d ^= a; d = rot[0](d)
        a = kbin(IntrinsicOp::VecAddI32x4, a, b, &mut k, &mut next_val_id);
        d = kbin(IntrinsicOp::VecXorI32x4, d, a, &mut k, &mut next_val_id);
        d = match rot_masks.get(&kcst[0]) {
            Some(&m) => kshufb(d, m, &mut k, &mut next_val_id),
            None => krot(d, kcst[0] as i64, &mut k, &mut next_val_id),
        };
        // c += d; b ^= c; b = rot[1](b)
        c = kbin(IntrinsicOp::VecAddI32x4, c, d, &mut k, &mut next_val_id);
        b = kbin(IntrinsicOp::VecXorI32x4, b, c, &mut k, &mut next_val_id);
        b = match rot_masks.get(&kcst[1]) {
            Some(&m) => kshufb(b, m, &mut k, &mut next_val_id),
            None => krot(b, kcst[1] as i64, &mut k, &mut next_val_id),
        };
        // a += b; d ^= a; d = rot[2](d)
        a = kbin(IntrinsicOp::VecAddI32x4, a, b, &mut k, &mut next_val_id);
        d = kbin(IntrinsicOp::VecXorI32x4, d, a, &mut k, &mut next_val_id);
        d = match rot_masks.get(&kcst[2]) {
            Some(&m) => kshufb(d, m, &mut k, &mut next_val_id),
            None => krot(d, kcst[2] as i64, &mut k, &mut next_val_id),
        };
        // c += d; b ^= c; b = rot[3](b)
        c = kbin(IntrinsicOp::VecAddI32x4, c, d, &mut k, &mut next_val_id);
        b = kbin(IntrinsicOp::VecXorI32x4, b, c, &mut k, &mut next_val_id);
        b = match rot_masks.get(&kcst[3]) {
            Some(&m) => kshufb(b, m, &mut k, &mut next_val_id),
            None => krot(b, kcst[3] as i64, &mut k, &mut next_val_id),
        };
        if round == 1 {
            // Lane-rotated group teardown: rotate the roles back, so the
            // backedge registers are in the same alignment the preheader
            // packs and the epilogue extracts assume.  rotl_{4-k} is the
            // inverse of rotl_k (rotl2 is an involution); zero offsets
            // need nothing.
            let ib = (4 - arx.shape_kb) % 4;
            let ic = (4 - arx.shape_kc) % 4;
            let id = (4 - arx.shape_kd) % 4;
            if ib != 0 {
                b = kshufd(b, lanerot(ib), &mut k, &mut next_val_id);
            }
            if ic != 0 {
                c = kshufd(c, lanerot(ic), &mut k, &mut next_val_id);
            }
            if id != 0 {
                d = kshufd(d, lanerot(id), &mut k, &mut next_val_id);
            }
        }
    }
    // Rename the final role values onto the backedge phi inputs: BOTH
    // the definition and every use.  (Patching only the destination left
    // the later `d ^= a`-style uses referencing the old value id, and the
    // backend rightly refused `value 1227 has no register, stack slot,
    // Copy, or GlobalAddr definition`.)
    let mut rename = |from: Value, to: Value| {
        for inst in k.iter_mut() {
            if let Instruction::Intrinsic { dest, args, .. } = inst {
                if *dest == Some(from) {
                    *dest = Some(to);
                }
                for arg in args.iter_mut() {
                    if matches!(arg, Operand::Value(v) if *v == from) {
                        *arg = Operand::Value(to);
                    }
                }
            }
        }
    };
    rename(a, next_a);
    rename(b, next_b);
    rename(c, next_c);
    rename(d, next_d);

    // ---- Body rewrite: the scalar ARX dies, the kernel takes its place. ----
    // Retained outside the word dataflow: the loop-counter increment (an
    // I32/I64 add, already identified during analysis as IV feed).
    {
        let mut retained: Vec<Instruction> = Vec::new();
        for &bi in arx.order.iter() {
            for (ii, inst) in func.blocks[bi].instructions.iter().enumerate() {
                let is_iv_feed = matches!(
                    inst,
                    Instruction::BinOp {
                        op: IrBinOp::Add,
                        ty: IrType::I32 | IrType::I64,
                        ..
                    }
                ) && !arx.term_values.contains_key(&match inst {
                    Instruction::BinOp { dest, .. } => *dest,
                    _ => Value(0),
                });
                let is_counter_add = inst.dest().map(|d| d == arx.counter_next).unwrap_or(false);
                if is_iv_feed || is_counter_add {
                    // Retain IV feeds from EVERY body block: the increment
                    // can sit in a routing block (which is cleared below),
                    // so everything retained is re-homed into the main
                    // block.  Dominance holds: the header phis dominate
                    // the whole body, and the only users are the header
                    // phis' backedges.  (The counter's own `i+1` add DOES
                    // carry a trivial self+const term from the symbolic
                    // evaluation — nothing in the kernel consumes it, but
                    // the phi still needs its backedge value.)
                    retained.push(inst.clone());
                }
            }
        }
        let mut body_insts = k;
        body_insts.extend(retained);
        func.blocks[main_block].instructions = body_insts;
        for &bi in arx.order.iter() {
            if bi != main_block {
                func.blocks[bi].instructions.clear();
            }
        }
    }

    // ---- Epilogue: extract the 16 words at the loop exit.  The exit
    //      block index, its outside-the-body polarity and its
    //      header-only predecessor set were validated BEFORE the first
    //      mutation at the top of this function. ----
    let mut x_extract = [Value(0); 16];
    let mut ext_insts: Vec<Instruction> = Vec::with_capacity(16);
    for xk in 0..16 {
        let (role, lane) = (xk / 4, xk % 4);
        let src = [phi_a, phi_b, phi_c, phi_d][role];
        let dest = Value(next_val_id);
        next_val_id += 1;
        ext_insts.push(Instruction::Intrinsic {
            dest: Some(dest),
            op: IntrinsicOp::VecExtractLaneI32x4,
            dest_ptr: None,
            args: vec![
                Operand::Value(src),
                Operand::Const(IrConst::I32(lane as i32)),
            ],
        });
        x_extract[xk] = dest;
    }
    {
        let exit = &mut func.blocks[exit_idx];
        let phi_end = exit
            .instructions
            .iter()
            .position(|i| !matches!(i, Instruction::Phi { .. }))
            .unwrap_or(exit.instructions.len());
        let mut at = phi_end;
        for inst in ext_insts {
            exit.instructions.insert(at, inst);
            at += 1;
        }
    }

    // ---- Rewrite every use of the word phis outside the loop. ----
    let body = &arx.body;
    for (bi, blk) in func.blocks.iter_mut().enumerate() {
        if body.contains(&bi) {
            continue;
        }
        for inst in blk.instructions.iter_mut() {
            rewrite_operands(inst, &mut |op: &mut Operand| {
                if let Operand::Value(v) = op {
                    if let Some(xk) = arx.word_phis.iter().position(|&p| p == *v) {
                        *op = Operand::Value(x_extract[xk]);
                    }
                }
            });
        }
    }

    // ---- Delete the dead word phis from the header. ----
    let word_phis = arx.word_phis;
    func.blocks[arx.header_idx]
        .instructions
        .retain(|i| !matches!(i, Instruction::Phi { dest, .. } if word_phis.contains(dest)));

    func.next_value_id = next_val_id;
    if debug {
        eprintln!(
            "[ARX] vectorized loop: header=blk{} main=blk{} pshufb={}",
            arx.header_idx, main_block, use_pshufb
        );
    }
    16
}

// ===========================================================================
// Entry point
// ===========================================================================

/// Run ARX lane vectorization on a function.  Returns the number of state
/// words lifted to vector lanes (16 per transformed loop).
pub(crate) fn run(func: &mut IrFunction) -> usize {
    let debug = std::env::var("LCCC_DEBUG_ARX").is_ok();
    if std::env::var("CCC_NO_ARX_VEC").is_ok() {
        return 0;
    }
    // ISA gate: the whole scheme lives in the SSE2 XMM file; SSSE3's
    // pshufb (checked through SSE4.1, which implies it) upgrades the
    // 16/8-bit rotates to single instructions.
    if !crate::passes::vectorize::x86_simd_available_pub() {
        return 0;
    }
    let use_pshufb = crate::passes::vectorize::x86_sse41_available_pub();

    let num_blocks = func.blocks.len();
    let cfg = CfgAnalysis::build(func);
    let loops = loop_analysis::find_natural_loops(num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
    if loops.is_empty() {
        return 0;
    }

    let mut changed = 0usize;
    for (idx, lp) in loops.iter().enumerate() {
        // Innermost loops only (an ARX loop never nests another loop by
        // construction, but stay defensive about outer wrappers).
        if loops.iter().enumerate().any(|(j, other)| {
            j != idx
                && other.body.len() < lp.body.len()
                && other.body.iter().all(|b| lp.body.contains(b))
        }) {
            continue;
        }
        match analyze_arx_loop(func, &cfg, lp) {
            Some(arx) => changed += transform_arx_loop(func, &cfg, &arx, use_pshufb, debug),
            None => {
                if debug {
                    eprintln!("[ARX] loop at blk{} not ARX-shaped", lp.header);
                }
            }
        }
    }
    changed
}
