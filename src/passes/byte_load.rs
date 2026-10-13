//! Little-endian byte-load reassembly: fold `p[0] | p[1] << 8 | ...` back
//! into one wide load.
//!
//! Portable C spells an unaligned little-endian integer read as a chain of
//! byte loads (`SIP_U8TO64_LE`, Linux's `get_unaligned_le*`, `htole*`/`le*toh`
//! helpers, and the `memcpy(&w, p, 4)` idiom in code that cannot use `memcpy`
//! under `-ffreestanding`). The front end lowers the chain literally, and on a
//! little-endian machine that chain is *semantically identical* to a single
//! unaligned load of the same width.
//!
//! Measured before this pass existed (`-O2 -march=x86-64-v3`, static
//! instruction count, Compiler Explorer oracles gcc 16.2 / clang 23.1):
//!
//! | function | shape | LCCC | gcc | clang |
//! |---|---|---:|---:|---:|
//! | `load_le64` (canonical 8-byte chain) | 8 byte loads + 7 shl + 7 or | 61 | 2 | 2 |
//! | `sip24_update` (Expat SipHash) | 2 × 8-byte chain + 1 × 4-byte chain | 229 | 76 | 79 |
//!
//! Both oracles collapse the chain — gcc keeps the pair in `xmm` and extracts
//! with `vpextrq`, i.e. it recognises the wider load first and vectorises
//! after. This pass restores the scalar half of that, so the byte assembly a
//! front end spelled out is no longer invisible to the existing wide-load
//! machinery (vectorisers, store/load forwarding, address folding).
//!
//! # What is matched
//!
//! One basic block, one `Or`-tree with exactly `bits/8` byte terms, all of
//! them in that block:
//!
//! ```text
//! %pK = %base + (O + K)         ; K = 0..n-1, via one or more constant GEPs
//! %bK = load u8, %pK            ; not volatile, one shared root `%base`
//! %cK = cast u8 -> uN, %bK      ; unsigned, zero-extending
//! %sK = shl uN %cK, 8*K         ; K > 0 (K == 0 is the bare cast)
//! %r  = or ... (uN)             ; any association/order of the same terms
//! ```
//!
//! rewritten to
//!
//! ```text
//! %r = load uN, %base           ; same destination, same position
//! ```
//!
//! with the byte loads, casts, shifts and inner `Or`s removed.
//!
//! # Why it is safe
//!
//! * **Ordering / aliasing.** A wide load at the position of the final `Or`
//!   would read the *new* contents of any byte a store overwrote after the
//!   original byte load. The pass refuses when any instruction between the
//!   earliest consumed instruction and the rewrite position may write memory
//!   ([`Instruction::may_write_memory`], the predicate GVN uses for cached
//!   loads), and requires the whole chain in one block, so no other block can
//!   intervene either.
//! * **Validity of the widened access.** The wide load touches exactly the
//!   bytes the chain already touched: the pointers are an enforced
//!   *consecutive run* `O, O+1, ..., O+n-1` off one shared root, so it cannot
//!   fault where the original did not. The ordering question above is deliberately *asymmetric*: a torn
//!   wide load that overlaps a source byte lands on the conservative side and
//!   can never invent a value the byte chain would not have produced.
//! * **Value equivalence.** The chain ORs each byte at its own little-endian
//!   position, a pure reinterpretation of the same `n` bytes — the only
//!   target-dependent step, so the pass is gated to the little-endian x86
//!   targets in [`run_function`].
//! * **Provenance.** Consumed values must be single-def *and* single-use, the
//!   byte loads non-volatile with the default address space, and all terms
//!   must share one base *expression* (peeled through `Copy`s and constant
//!   `GetElementPtr`s) whose reaching definition does not change anywhere in
//!   the window — the induction variable of a buffer walk is written by a
//!   `Copy` at the bottom of its block, which is outside the window and
//!   therefore fine. The base may be *any* pointer expression: a parameter, a
//!   global, a loaded pointer or a computed address. The soundness argument
//!   above never depended on where the address came from, only on the bytes
//!   touched and on what may have written them in between; requiring a
//!   parameter cost the dominant real-world shape (`le32(buf + i)`) for no
//!   gain in confidence.
//!
//! # Refused, deliberately
//!
//! * Big-endian order (`p[0] << 56 | ...`) — a `bswap` load, which
//!   [`super::bit_idioms`] owns; the two passes must not fight over it.
//! * Signed byte casts (`(int8_t)p[0]`): the sign extension is not a
//!   reinterpretation, so a widened load would be wrong.
//! * Chains split across blocks, a base redefined inside the window, any
//!   multi-use/multi-def chain value, and duplicate or non-multiple-of-8
//!   offsets.
//!
//! Pass name for `CCC_DISABLE_PASSES`: `byte_load`.
//! * `-Os`/`-Oz`: a wide load is not always the smaller *sequence* on every
//!   target (on x86-64 it is, by a wide margin — but the size profile is
//!   handled by the pipeline, not by a special case here).

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{BasicBlock, Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

/// One matched byte term: the byte loaded at `ptr`, shifted into place.
struct Term {
    /// Shift amount in bits. Distinct multiples of 8 covering `0..bits` are
    /// what makes the OR-tree an n-byte load (enforced in [`validate`]).
    shift: u32,
    /// Value the byte is loaded from: the shared base expression or a chain
    /// of constant-offset `GetElementPtr`s on it, resolved by [`resolve_ptr`].
    ptr: Value,
}

/// A fully validated rewrite.
struct Plan {
    block: usize,
    /// Destination of the final `Or`, preserved so its users are untouched.
    /// When `convert` is present the conversion defines it, otherwise the load
    /// does.
    dest: Value,
    /// Carrier type: the type of the `Or` the rewrite replaces.
    ty: IrType,
    /// Type of the replacement load: the carrier width for a complete run, or
    /// the run's own width when the carrier is wider (a 16-bit field assembled
    /// in an `int`, which C's integer promotion produces).
    load_ty: IrType,
    /// The pointer the load reads from: the lowest-address byte of the run.
    load_ptr: Value,
    /// Destination of the load. Equal to `dest` unless a conversion follows,
    /// in which case it is one of the folded values reused as a dead temporary.
    load_dest: Value,
    /// Conversion from the loaded bytes to the assembled value.
    convert: Option<Convert>,
    /// Values whose defining instructions are folded away.
    consumed: Vec<Value>,
}

/// How the loaded bytes become the `Or`'s value.
#[derive(Clone, Copy, PartialEq)]
enum Convert {
    /// Big-endian byte order: reverse the loaded bytes.
    Bswap,
    /// Little-endian run narrower than its carrier: zero-extend (`Cast`).
    Zext,
}

/// Per-function context: the single definition of each value, plus exact use
/// counts (operands, address uses and terminators all counted).
struct Ctx {
    defs: Vec<Option<Instruction>>,
    loc: Vec<Option<(usize, usize)>>,
    /// Every definition site of every value. Multi-def values (the post-phi
    /// coalescing web) have no single meaning, but a *pointer* with several
    /// definitions is still usable as a base when none of those definitions
    /// sits inside the chain window — see `uniform_in_window`.
    def_locs: Vec<Vec<(usize, usize)>>,
    uses: Vec<u32>,
}

impl Ctx {
    fn build(func: &IrFunction) -> Self {
        let nvals = func.max_value_id() as usize + 1;
        let mut defs: Vec<Option<Instruction>> = vec![None; nvals];
        let mut loc: Vec<Option<(usize, usize)>> = vec![None; nvals];
        let mut def_locs: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nvals];
        let mut uses = vec![0u32; nvals];
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                if let Some(dest) = inst.dest() {
                    if let Some(slot) = defs.get_mut(dest.0 as usize) {
                        def_locs[dest.0 as usize].push((bi, ii));
                        *slot = Some(inst.clone());
                        loc[dest.0 as usize] = Some((bi, ii));
                    }
                }
                crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        if let Some(c) = uses.get_mut(v.0 as usize) {
                            *c += 1;
                        }
                    }
                });
                crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                    if let Some(c) = uses.get_mut(v.0 as usize) {
                        *c += 1;
                    }
                });
            }
            crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
                if let Operand::Value(v) = op {
                    if let Some(c) = uses.get_mut(v.0 as usize) {
                        *c += 1;
                    }
                }
            });
        }
        // A value with several definitions has position-dependent content
        // (the post-phi coalescing web `gvn::find_multi_def_values` guards
        // elsewhere): trust none of them.
        for (v, sites) in def_locs.iter().enumerate() {
            if sites.len() != 1 {
                defs[v] = None;
                loc[v] = None;
            }
        }
        Ctx {
            defs,
            loc,
            def_locs,
            uses,
        }
    }

    /// Definition of a value that has exactly one definition.
    fn def(&self, v: Value) -> Option<&Instruction> {
        self.defs.get(v.0 as usize).and_then(Option::as_ref)
    }

    fn loc_of(&self, v: Value) -> Option<(usize, usize)> {
        self.loc.get(v.0 as usize).copied().flatten()
    }

    fn use_count(&self, v: Value) -> u32 {
        self.uses.get(v.0 as usize).copied().unwrap_or(0)
    }

    /// Follow `Copy` chains (single-def only). A peeled `Copy` is left in
    /// place for DCE/copy-prop to remove, so this pass never has to reason
    /// about a copied value that has further users.
    fn peel(&self, op: Operand) -> Option<Value> {
        self.peel_or_self(operand_value(op)?, 0)
    }

    /// [`Self::peel`] for values that may have several definitions or none
    /// this pass knows about: those are returned unchanged (they are their own
    /// meaning), `Copy` chains are still followed.
    fn peel_or_self(&self, mut v: Value, mut budget: u32) -> Option<Value> {
        while budget < 8 {
            match self.def(v) {
                Some(Instruction::Copy { src, .. }) => match src {
                    Operand::Value(src) => {
                        v = *src;
                        budget += 1;
                    }
                    Operand::Const(_) => return None,
                },
                _ => return Some(v),
            }
        }
        Some(v)
    }

    /// Consume `from` and each `Copy` link below it down to `stop`
    /// (inclusive), proving every link is used exactly once — by the next link
    /// up the chain. The top of the chain is the pointer of a byte load the
    /// rewrite removes, so after the rewrite none of these values has a
    /// surviving user: deleting them cannot leave a dangling operand.
    fn consume_dead_chain(
        &self,
        consumed: &mut Vec<Value>,
        from: Value,
        stop: Value,
        block: usize,
    ) -> Option<()> {
        let mut v = from;
        for _ in 0..8 {
            if self.use_count(v) != 1 || self.loc_of(v)?.0 != block {
                return None;
            }
            consumed.push(v);
            if v == stop {
                return Some(());
            }
            match self.def(v)? {
                Instruction::Copy {
                    src: Operand::Value(src),
                    ..
                } => v = *src,
                _ => return None,
            }
        }
        None
    }

    /// Whether `v` has the same reaching definition at every position of the
    /// chain window `[earliest, top]` in `block`.
    ///
    /// A pointer used as the chain's base may legitimately have several
    /// definitions (the induction variable of the loop walking a buffer is
    /// written by a `Copy` at the block's bottom): what matters is that none
    /// of those definitions sits *inside* the window, so the value the byte
    /// loads read and the value the wide load reads are one and the same.
    fn uniform_in_window(&self, v: Value, block: usize, earliest: usize, top: usize) -> bool {
        match self.def_locs.get(v.0 as usize) {
            Some(sites) => sites
                .iter()
                .all(|(b, i)| *b != block || *i < earliest || *i > top),
            None => true,
        }
    }
}

/// The value inside a pointer operand, or `None` for a constant operand.
fn operand_value(op: Operand) -> Option<Value> {
    match op {
        Operand::Value(v) => Some(v),
        Operand::Const(_) => None,
    }
}

/// Wide integer types the pass may build a load of.
fn wide_bits(ty: IrType) -> Option<u32> {
    match ty {
        IrType::U16 | IrType::I16 => Some(16),
        IrType::U32 | IrType::I32 => Some(32),
        IrType::U64 | IrType::I64 => Some(64),
        _ => None,
    }
}

/// Non-negative integer constant, as an unsigned value.
fn const_u64(op: Operand) -> Option<u64> {
    match op {
        Operand::Const(IrConst::I64(k)) if k >= 0 => Some(k as u64),
        Operand::Const(IrConst::I32(k)) if k >= 0 => Some(k as u64),
        Operand::Const(IrConst::I16(k)) if k >= 0 => Some(k as u64),
        Operand::Const(IrConst::I8(k)) if k >= 0 => Some(k as u64),
        _ => None,
    }
}

/// One `Or` operand: either a partial sum (recurse) or a byte term.
///
/// `consumed` collects the values whose defining instruction the rewrite
/// removes. The final `Or`'s destination is never pushed — users keep it.
fn match_sum(
    ctx: &Ctx,
    op: Operand,
    bits: u32,
    depth: u32,
    terms: &mut Vec<Term>,
    consumed: &mut Vec<Value>,
) -> Option<()> {
    if depth > 64 {
        return None;
    }
    let v = ctx.peel(op)?;
    match ctx.def(v)? {
        Instruction::BinOp {
            dest,
            op: IrBinOp::Or,
            lhs,
            rhs,
            ty,
        } if *dest == v && wide_bits(*ty) == Some(bits) => {
            consumed.push(v);
            match_sum(ctx, *lhs, bits, depth + 1, terms, consumed)?;
            match_sum(ctx, *rhs, bits, depth + 1, terms, consumed)
        }
        _ => match_term(ctx, v, bits, terms, consumed),
    }
}

/// One byte term: `shl (cast u8 -> uN) %c, k`, or the `k == 0` term, which the
/// front end leaves as the bare zero-extending cast.
fn match_term(
    ctx: &Ctx,
    v: Value,
    bits: u32,
    terms: &mut Vec<Term>,
    consumed: &mut Vec<Value>,
) -> Option<()> {
    match ctx.def(v)? {
        Instruction::BinOp {
            dest,
            op: IrBinOp::Shl,
            lhs,
            rhs,
            ty,
        } if *dest == v && wide_bits(*ty) == Some(bits) => {
            let k = const_u64(*rhs)?;
            if k == 0 || k % 8 != 0 || k >= u64::from(bits) {
                return None;
            }
            consumed.push(v);
            match_cast_load(ctx, *lhs, bits, k as u32, terms, consumed)
        }
        _ => match_cast_load(ctx, Operand::Value(v), bits, 0, terms, consumed),
    }
}

/// `cast u8 -> uN` of a plain (non-volatile) byte load.
fn match_cast_load(
    ctx: &Ctx,
    op: Operand,
    bits: u32,
    shift: u32,
    terms: &mut Vec<Term>,
    consumed: &mut Vec<Value>,
) -> Option<()> {
    let v = ctx.peel(op)?;
    match ctx.def(v)? {
        Instruction::Cast {
            dest,
            src,
            from_ty: IrType::U8,
            to_ty,
        } if *dest == v && wide_bits(*to_ty) == Some(bits) => {
            consumed.push(v);
            let lv = ctx.peel(*src)?;
            match ctx.def(lv)? {
                Instruction::Load {
                    dest,
                    ptr,
                    ty: IrType::U8,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                } if *dest == lv => {
                    if ctx.use_count(lv) != 1 {
                        return None;
                    }
                    consumed.push(lv);
                    terms.push(Term { shift, ptr: *ptr });
                    Some(())
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Fold byte-load chains into one wide load; returns the number of chains
/// folded. Enabled where the oracles themselves widen the access, which is a
/// measured question, not a matter of taste (`scripts/codegen_oracle.py`,
/// x86-64 baseline `-O3 -march=...`; GCC 16.1, GCC trunk and Clang 23.1 per
/// architecture, instruction counts for `le16/le32/le64/be64/body_le64`):
///
/// * x86-64, i686: 2/2/2/2/2 (GCC, Clang; ICX 2 on all but `be64`) -- on.
/// * AArch64: 2/2/2/3/2 for all three channels: every oracle merges the chain
///   into one unaligned `ldr`, and the big-endian form into `ldr` + `rev` --
///   on, which is the same rewrite.
/// * RISC-V 64: 5/12/23/23/23 (GCC 16.1 and Clang 23.1).  No oracle widens:
///   bare RISC-V does not guarantee unaligned access, so the targets keep the
///   byte-by-byte form and a wide load here could trap into the kernel's
///   misaligned-access handler.  Off until the target gains an
///   alignment-assumed subtarget flag, and off *by measurement*, not by
///   omission.
///
/// The rewrite is a byte-order reinterpretation of bytes the program already
/// read, so it is exact where the machine allows the access; big-endian chains
/// become a byte reversal, which is why the order is detected per run.
pub(crate) fn run_function(func: &mut IrFunction, target: crate::backend::Target) -> usize {
    if !matches!(
        target,
        crate::backend::Target::X86_64
            | crate::backend::Target::I686
            | crate::backend::Target::Aarch64
    ) {
        return 0;
    }
    let ctx = Ctx::build(func);
    if ctx.defs.is_empty() {
        return 0;
    }
    let mut plans: Vec<Plan> = Vec::new();

    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::BinOp {
                dest,
                op: IrBinOp::Or,
                lhs,
                rhs,
                ty,
            } = inst
            else {
                continue;
            };
            let Some(bits) = wide_bits(*ty) else {
                continue;
            };
            // The rewrite keeps this destination, so it must be the value's
            // only definition.
            if ctx.def(*dest).is_none() {
                continue;
            }
            let mut terms = Vec::new();
            let mut consumed = Vec::new();
            let mut ok = match_sum(&ctx, *lhs, bits, 0, &mut terms, &mut consumed).is_some()
                && match_sum(&ctx, *rhs, bits, 0, &mut terms, &mut consumed).is_some();
            if ok {
                ok = !consumed.contains(dest);
            }
            if !ok {
                continue;
            }
            if let Some(plan) = validate(func, &ctx, bi, ii, *dest, *ty, terms, consumed) {
                plans.push(plan);
            }
        }
    }
    if plans.is_empty() {
        return 0;
    }
    // A plan's destination must not be another plan's folded value.
    let mut all_consumed = FxHashSet::default();
    for plan in &plans {
        for v in &plan.consumed {
            all_consumed.insert(v.0);
        }
    }
    plans.retain(|plan| !all_consumed.contains(&plan.dest.0));

    let mut folded = 0;
    for block_idx in 0..func.blocks.len() {
        let block_plans: Vec<&Plan> = plans.iter().filter(|p| p.block == block_idx).collect();
        if block_plans.is_empty() {
            continue;
        }
        folded += apply_block(func.blocks.get_mut(block_idx), &block_plans);
    }
    folded
}

/// Resolve a pointer operand to `(root, constant byte offset)`, following any
/// chain of constant-offset `GetElementPtr`s (and `Copy`s — [`Ctx::peel`]).
/// Every `GetElementPtr` step traversed on the way to `root` is recorded in
/// `gep_steps` so the caller can include its position in the stability window.
fn resolve_ptr(
    ctx: &Ctx,
    op: Operand,
    depth: u32,
    gep_steps: &mut Vec<Value>,
) -> Option<(Value, u64)> {
    if depth > 8 {
        return None;
    }
    let v = ctx.peel_or_self(operand_value(op)?, 0)?;
    match ctx.def(v) {
        Some(Instruction::GetElementPtr {
            dest,
            base,
            offset,
            ty: IrType::Ptr,
        }) if *dest == v => match const_u64(*offset) {
            // Constant step: keep walking towards the root.
            Some(off) => {
                gep_steps.push(v);
                let (root, root_off) =
                    resolve_ptr(ctx, Operand::Value(*base), depth + 1, gep_steps)?;
                Some((root, root_off.checked_add(off)?))
            }
            // Variable step (`p = buf + i`): this `GetElementPtr` *is* the
            // base expression, and the terms are constant offsets off it.
            None => Some((v, 0)),
        },
        _ => Some((v, 0)),
    }
}

/// Check the collected terms really describe one unaligned load, and that
/// nothing in between may have rewritten those bytes.
fn validate(
    func: &IrFunction,
    ctx: &Ctx,
    block: usize,
    top: usize,
    dest: Value,
    ty: IrType,
    terms: Vec<Term>,
    mut consumed: Vec<Value>,
) -> Option<Plan> {
    let carrier_bits = wide_bits(ty)?;
    let carrier = (carrier_bits / 8) as usize;
    // `m` = the run's width in bytes: the number of terms, which must be a
    // native width (`2`, `4` or `8`) no wider than the carrier. The shifts are
    // then a permutation of {0, 8, ..., 8(m-1)} — the value proof.
    let m = terms.len();
    if !(2..=8).contains(&m) || m.count_ones() != 1 || m > carrier {
        return None;
    }
    let mut by_shift: Vec<Option<Value>> = vec![None; m];
    for term in &terms {
        if term.shift % 8 != 0 {
            return None;
        }
        let idx = (term.shift / 8) as usize;
        if idx >= m || by_shift[idx].is_some() {
            return None;
        }
        by_shift[idx] = Some(term.ptr);
    }
    let by_shift: Vec<Value> = by_shift.into_iter().collect::<Option<Vec<Value>>>()?;

    // Consumed values: single use, single definition, in this block, and not
    // the value the rewrite keeps.
    for v in &consumed {
        if *v == dest || ctx.use_count(*v) != 1 || ctx.loc_of(*v)?.0 != block {
            return None;
        }
    }

    // Resolve every term's pointer to (base expression, byte offset).
    let mut resolved: Vec<(Value, u64)> = Vec::with_capacity(m);
    let mut gep_steps: Vec<Value> = Vec::with_capacity(m);
    for ptr in &by_shift {
        resolved.push(resolve_ptr(ctx, Operand::Value(*ptr), 0, &mut gep_steps)?);
    }
    let root = resolved[0].0;
    if resolved.iter().any(|(r, _)| *r != root) {
        return None;
    }
    let anchor = resolved.iter().map(|(_, o)| *o).min()?;

    // Which byte order? Little-endian puts the shift-0 byte at the run's
    // lowest address, big-endian at its highest. Anything else (a gap, an
    // overlap) is not a load of the run and is refused.
    let little = |k: usize| anchor + k as u64;
    let big = |k: usize| anchor + (m - 1 - k) as u64;
    let (convert, load_slot) = if resolved
        .iter()
        .enumerate()
        .all(|(k, (_, o))| *o == little(k))
    {
        let c = if m < carrier {
            Some(Convert::Zext)
        } else {
            None
        };
        (c, 0)
    } else if resolved.iter().enumerate().all(|(k, (_, o))| *o == big(k)) {
        // Big-endian: one wide load plus a byte reversal. The carrier is
        // exactly the run's width here, so no width-changing conversion has to
        // follow the reversal.
        if m != carrier {
            return None;
        }
        (Some(Convert::Bswap), m - 1)
    } else {
        return None;
    };

    // The load reads the run's lowest-address byte. It reads it through the
    // root only where the root *is* that address (a run starting at offset 0);
    // for a run at a nonzero offset the term's own pointer is kept, which costs
    // nothing — a `GetElementPtr` of constant displacement folds into the
    // load's addressing mode, so the emitted instruction is byte-identical.
    let lowest = by_shift[load_slot];
    let peeled_lowest = ctx.peel_or_self(lowest, 0)?;
    let fold_anchor = resolved[load_slot].1 == 0
        && peeled_lowest != root
        && matches!(
            ctx.def(peeled_lowest),
            Some(Instruction::GetElementPtr { .. })
        );
    let load_ptr = if fold_anchor {
        // The root denotes the same address, and the zero-offset `gep` chain
        // that reached it is dead once the byte loads go.
        ctx.consume_dead_chain(&mut consumed, lowest, peeled_lowest, block)?;
        root
    } else {
        lowest
    };
    // Every other term's pointer is single-use, defined in this block, and
    // folded away with the byte it fed.
    for (k, ptr) in by_shift.iter().enumerate() {
        if k == load_slot {
            continue;
        }
        let peeled = ctx.peel_or_self(*ptr, 0)?;
        if matches!(ctx.def(peeled), Some(Instruction::GetElementPtr { .. })) {
            ctx.consume_dead_chain(&mut consumed, *ptr, peeled, block)?;
        } else if ctx.use_count(peeled) != 1 || ctx.loc_of(peeled)?.0 != block {
            return None;
        }
    }

    // Ordering: no memory write between the earliest consumed instruction and
    // the rewrite position, and all constant-offset GEP steps must sit in the
    // same block inside the base stability window.
    let mut consumed_locs: Vec<(usize, usize)> = Vec::with_capacity(consumed.len());
    for v in &consumed {
        if *v == dest || ctx.use_count(*v) != 1 {
            return None;
        }
        let (cb, ci) = ctx.loc_of(*v)?;
        if cb != block {
            return None;
        }
        consumed_locs.push((cb, ci));
    }
    let mut earliest = consumed_locs
        .iter()
        .map(|(_, ci)| *ci)
        .min()
        .unwrap_or(top)
        .min(top);
    for g in &gep_steps {
        let (gb, gi) = ctx.loc_of(*g)?;
        if gb != block {
            return None;
        }
        earliest = earliest.min(gi);
    }
    // The base must have one reaching definition across the whole window: a
    // pointer redefined between the byte loads and the merge would make the
    // wide load read a different address than the bytes it replaces.
    if !ctx.uniform_in_window(root, block, earliest, top) {
        return None;
    }
    let block_ref = func.blocks.get(block)?;
    for inst in block_ref.instructions.get(earliest..top)? {
        if inst.may_write_memory() {
            return None;
        }
    }

    // A conversion needs a temporary to hold the loaded bytes. Reuse a folded
    // value: each is single-use and about to become dead, so its id is free
    // and no new value is created. Its former type does not matter — types
    // live on instructions, not on value ids.
    let load_dest = match convert {
        None => dest,
        Some(_) => *consumed.iter().find(|v| **v != dest)?,
    };

    Some(Plan {
        block,
        dest,
        ty,
        load_ty: unsigned_of(if convert == Some(Convert::Zext) {
            m as u32 * 8
        } else {
            carrier_bits
        })?,
        load_ptr,
        load_dest,
        convert,
        consumed,
    })
}

/// The unsigned integer type of `bits` width.
fn unsigned_of(bits: u32) -> Option<IrType> {
    match bits {
        16 => Some(IrType::U16),
        32 => Some(IrType::U32),
        64 => Some(IrType::U64),
        _ => None,
    }
}

/// Replace each plan's chain with its wide load, rebuilding the block once so
/// several chains in one block (SipHash has three) cannot invalidate each
/// other's coordinates. Returns the number of plans applied.
fn apply_block(block: Option<&mut BasicBlock>, plans: &[&Plan]) -> usize {
    let Some(block) = block else {
        return 0;
    };
    let mut replaced: FxHashMap<u32, &Plan> = FxHashMap::default();
    for plan in plans {
        replaced.insert(plan.dest.0, plan);
    }
    let mut consumed: FxHashSet<u32> = FxHashSet::default();
    for plan in plans {
        for v in &plan.consumed {
            consumed.insert(v.0);
        }
    }
    let keep_spans =
        block.source_spans.len() == block.instructions.len() && !block.source_spans.is_empty();
    let old = std::mem::take(&mut block.instructions);
    let old_len = old.len();
    let mut new_insns = Vec::with_capacity(old_len);
    let mut new_spans = Vec::with_capacity(if keep_spans { old_len } else { 0 });
    let mut applied = 0;
    for (idx, inst) in old.into_iter().enumerate() {
        let dest = inst.dest().map(|d| d.0);
        if let Some(plan) = dest.and_then(|d| replaced.get(&d)) {
            let span = if keep_spans {
                Some(block.source_spans[idx])
            } else {
                None
            };
            new_insns.push(Instruction::Load {
                dest: plan.load_dest,
                ptr: plan.load_ptr,
                ty: plan.load_ty,
                seg_override: AddressSpace::Default,
                volatile: false,
            });
            if let Some(span) = span {
                new_spans.push(span);
            }
            match plan.convert {
                Some(Convert::Bswap) => {
                    new_insns.push(Instruction::UnaryOp {
                        dest: plan.dest,
                        op: crate::ir::reexports::IrUnaryOp::Bswap,
                        src: Operand::Value(plan.load_dest),
                        ty: plan.load_ty,
                    });
                    if let Some(span) = span {
                        new_spans.push(span);
                    }
                }
                Some(Convert::Zext) => {
                    new_insns.push(Instruction::Cast {
                        dest: plan.dest,
                        src: Operand::Value(plan.load_dest),
                        from_ty: plan.load_ty,
                        to_ty: plan.ty,
                    });
                    if let Some(span) = span {
                        new_spans.push(span);
                    }
                }
                None => {}
            }
            applied += 1;
        } else if dest.is_some_and(|d| consumed.contains(&d)) {
            // Folded away.
        } else {
            new_insns.push(inst);
            if keep_spans {
                new_spans.push(block.source_spans[idx]);
            }
        }
    }
    block.instructions = new_insns;
    // `source_spans` is a parallel array: keep it in lockstep when it started
    // that way, otherwise drop it (the convention used by copy_prop/dce).
    if keep_spans {
        block.source_spans = new_spans;
    } else if block.source_spans.len() != block.instructions.len() {
        block.source_spans.clear();
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Target;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    const X64: Target = Target::X86_64;

    #[derive(Default, Clone, Copy, Debug)]
    struct Flags {
        big_endian: bool,
        volatile: bool,
        extra_use: bool,
        duplicate_offset: bool,
        store_inside: bool,
        dynamic_base: bool,
        /// Byte offset of the first term inside the object (the struct-member
        /// shape: SipHash reads `buf[0..7]` at offset 32).
        field_offset: u64,
        /// Reach `field_offset` through two nested `GetElementPtr`s instead of
        /// one, as SSA publishers frequently produce.
        nested_gep: bool,
        /// Anchor the run on `GetElementPtr(base, %i)` with a *variable* index,
        /// i.e. the shape `le32(buf + i)` produces.
        variable_index: bool,
        /// The `Or`'s carrier is wider than the run (integer promotion of a
        /// 16-bit field into `int`): `bits` is then the *run* width.
        narrow_carrier: bool,
        /// Read the run at this byte offset from the root (`f->body`, a struct
        /// member), so the load must not degenerate to offset 0.
        run_offset: i64,
    }

    /// Append one canonical byte-load chain for `bits` using `base` and return
    /// the value of the final `Or`. `next` is the id allocator.
    fn append_chain(
        insns: &mut Vec<Instruction>,
        next: &mut u32,
        base: Value,
        bits: u32,
        wide: IrType,
        flags: Flags,
    ) -> Value {
        let n = u64::from(bits / 8);
        let mut ids = || {
            let v = *next;
            *next += 1;
            v
        };
        // The pointer every term loads from: either the shared base (with
        // per-term GEPs) or a value produced by a load (refused base).
        let anchor = if flags.dynamic_base {
            let anchor = Value(ids());
            insns.push(Instruction::Load {
                dest: anchor,
                ptr: base,
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
                volatile: false,
            });
            anchor
        } else {
            base
        };
        // `le32(buf + i)`: the run's anchor is a variable-offset GEP.
        let anchor = if flags.variable_index {
            let index = Value(ids());
            insns.push(Instruction::ParamRef {
                dest: index,
                param_idx: 1,
                ty: IrType::U64,
            });
            let pv = Value(ids());
            insns.push(Instruction::GetElementPtr {
                dest: pv,
                base: anchor,
                offset: Operand::Value(index),
                ty: IrType::Ptr,
            });
            pv
        } else {
            anchor
        };
        let carrier = if flags.narrow_carrier {
            IrType::U32
        } else {
            wide
        };
        let mut terms = Vec::new();
        let mut first_load = None;
        for k in 0..n {
            let offset = flags.field_offset
                + if flags.duplicate_offset && k == n - 1 {
                    k - 1
                } else {
                    k
                };
            let inner = if flags.nested_gep {
                let inner = Value(ids());
                insns.push(Instruction::GetElementPtr {
                    dest: inner,
                    base: anchor,
                    offset: Operand::Const(IrConst::I64(flags.field_offset as i64)),
                    ty: IrType::Ptr,
                });
                inner
            } else {
                anchor
            };
            let gep = Value(ids());
            insns.push(Instruction::GetElementPtr {
                dest: gep,
                base: inner,
                offset: Operand::Const(IrConst::I64(
                    (offset
                        - if flags.nested_gep {
                            flags.field_offset
                        } else {
                            0
                        }) as i64,
                )),
                ty: IrType::Ptr,
            });
            let load = Value(ids());
            if first_load.is_none() {
                first_load = Some(load);
            }
            insns.push(Instruction::Load {
                dest: load,
                ptr: gep,
                ty: IrType::U8,
                seg_override: AddressSpace::Default,
                volatile: flags.volatile,
            });
            let cast = Value(ids());
            insns.push(Instruction::Cast {
                dest: cast,
                src: Operand::Value(load),
                from_ty: IrType::U8,
                to_ty: carrier,
            });
            let shift = if flags.big_endian {
                (n - 1 - k) * 8
            } else {
                k * 8
            };
            if shift == 0 {
                terms.push(Operand::Value(cast));
            } else {
                let shl = Value(ids());
                insns.push(Instruction::BinOp {
                    dest: shl,
                    op: IrBinOp::Shl,
                    lhs: Operand::Value(cast),
                    rhs: Operand::Const(IrConst::I32(shift as i32)),
                    ty: carrier,
                });
                terms.push(Operand::Value(shl));
            }
            if flags.store_inside && k == 0 {
                insns.push(Instruction::Store {
                    val: Operand::Const(IrConst::I64(7)),
                    ptr: gep,
                    ty: IrType::U8,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                });
            }
        }
        if flags.extra_use {
            let alias = Value(ids());
            insns.push(Instruction::Copy {
                dest: alias,
                src: Operand::Value(first_load.expect("a first load exists")),
            });
        }
        let mut acc = terms[0];
        for term in &terms[1..] {
            let or = Value(ids());
            insns.push(Instruction::BinOp {
                dest: or,
                op: IrBinOp::Or,
                lhs: acc,
                rhs: *term,
                ty: carrier,
            });
            acc = Operand::Value(or);
        }
        match acc {
            Operand::Value(v) => v,
            Operand::Const(_) => unreachable!("the chain ends in an Or"),
        }
    }

    fn chain_func(bits: u32, wide: IrType, flags: Flags) -> IrFunction {
        let mut next = 1u32;
        let mut insns = vec![Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty: IrType::Ptr,
        }];
        let result = append_chain(&mut insns, &mut next, Value(0), bits, wide, flags);
        let mut f = IrFunction::new("chain".to_string(), wide, vec![], false);
        f.next_value_id = next;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            source_spans: vec![],
            instructions: insns,
            terminator: Terminator::Return(Some(Operand::Value(result))),
        });
        f
    }

    /// Resolve a pointer value through constant `GetElementPtr`s inside the
    /// function under test: `(root, total offset)`.
    fn resolve(f: &IrFunction, mut v: Value) -> (Value, i64) {
        let mut off = 0i64;
        for _ in 0..8 {
            match f.blocks[0]
                .instructions
                .iter()
                .find(|i| matches!(i, Instruction::GetElementPtr { dest, .. } if *dest == v))
            {
                Some(Instruction::GetElementPtr {
                    base,
                    offset: Operand::Const(IrConst::I64(o)),
                    ..
                }) => {
                    v = *base;
                    off += *o;
                }
                _ => break,
            }
        }
        (v, off)
    }

    fn wide_loads(f: &IrFunction) -> Vec<(Value, Value, IrType, bool)> {
        f.blocks[0]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Load {
                    dest,
                    ptr,
                    ty,
                    volatile,
                    ..
                } if wide_bits(*ty).is_some() => Some((*dest, *ptr, *ty, *volatile)),
                _ => None,
            })
            .collect()
    }

    /// The bytes the original chain read: `(root, offset)` per byte load.
    fn byte_footprint(f: &IrFunction) -> Vec<(Value, i64)> {
        f.blocks[0]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Load {
                    ptr,
                    ty: IrType::U8,
                    ..
                } => {
                    let (root, off) = resolve(f, *ptr);
                    Some((root, off))
                }
                _ => None,
            })
            .collect()
    }

    /// The proof obligation, executable: every wide load the rewrite creates
    /// must read a range lying inside the bytes the original chain read — same
    /// root, and every byte of the range was a byte of the chain. A load
    /// invented outside that footprint could fault where the original did not.
    fn assert_loads_stay_in_footprint(orig: &IrFunction, new: &IrFunction) {
        let footprint = byte_footprint(orig);
        for (_, ptr, ty, _) in wide_loads(new) {
            let (root, off) = resolve(new, ptr);
            let size = ty.size() as i64;
            for k in 0..size {
                assert!(
                    footprint.contains(&(root, off + k)),
                    "wide load covers (root={root:?}, +{}) which the chain never read",
                    off + k
                );
            }
        }
    }

    /// Rows of the same code shape at a nonzero run offset: the wide load must
    /// read the run, not the object's first byte. Regression cover for an
    /// earlier revision that adopted the resolved root as the load's pointer
    /// and read offset 0 — the corpus kernel's known-answer test caught it at
    /// run time, this catches it in the unit suite.
    #[test]
    fn folds_a_run_at_a_nonzero_offset() {
        let shapes: [(u32, IrType, Flags); 3] = [
            (64, IrType::U64, Flags::default()),
            (
                64,
                IrType::U64,
                Flags {
                    big_endian: true,
                    ..Flags::default()
                },
            ),
            (
                16,
                IrType::U32,
                Flags {
                    narrow_carrier: true,
                    ..Flags::default()
                },
            ),
        ];
        for (bits, ty, flags) in shapes {
            let mut f = chain_func(
                bits,
                ty,
                Flags {
                    field_offset: 8,
                    nested_gep: true,
                    ..flags
                },
            );
            let orig = f.clone();
            let what = format!("{bits}-bit run at offset 8, flags {flags:?}");
            assert_eq!(run_function(&mut f, X64), 1, "{what}");
            let loads = wide_loads(&f);
            assert_eq!(loads.len(), 1, "{what}");
            assert_eq!(resolve(&f, loads[0].1), (Value(0), 8), "{what}");
            assert_loads_stay_in_footprint(&orig, &f);
        }
    }

    #[test]
    fn folds_the_canonical_little_endian_chain() {
        let mut f = chain_func(64, IrType::U64, Flags::default());
        let orig = f.clone();
        let before = f.blocks[0].instructions.len();
        assert_eq!(run_function(&mut f, X64), 1);
        assert_loads_stay_in_footprint(&orig, &f);
        assert!(before > 20, "sanity: the chain was long ({before})");
        assert_eq!(
            f.blocks[0].instructions.len(),
            2,
            "expected param + wide load: {:?}",
            f.blocks[0].instructions
        );
        let loads = wide_loads(&f);
        assert_eq!(loads.len(), 1);
        let (dest, ptr, ty, volatile) = loads[0];
        assert_eq!(resolve(&f, ptr), (Value(0), 0), "loads from the parameter");
        assert_eq!(ty, IrType::U64);
        assert!(!volatile);
        match &f.blocks[0].terminator {
            Terminator::Return(Some(Operand::Value(v))) => assert_eq!(*v, dest),
            other => panic!("unexpected terminator {other:?}"),
        }
    }

    #[test]
    fn folds_32_bit_and_16_bit_chains() {
        for (bits, ty) in [(32u32, IrType::U32), (16, IrType::U16)] {
            let mut f = chain_func(bits, ty, Flags::default());
            assert_eq!(run_function(&mut f, X64), 1, "{bits}-bit chain");
            let loads = wide_loads(&f);
            assert_eq!(loads.len(), 1);
            assert_eq!(resolve(&f, loads[0].1), (Value(0), 0));
            assert_eq!(loads[0].2, ty);
        }
    }

    #[test]
    fn folds_where_the_oracles_widen_and_is_gated_on_riscv() {
        // x86-64 and i686 are the original targets; AArch64 joined them after
        // the cross-architecture oracle run showed all three AArch64 channels
        // (GCC 16.1, GCC trunk, Clang 23.1) emitting one unaligned `ldr` where
        // LCCC emitted a scalar chain.  RISC-V stays out: no RISC-V oracle
        // widens the chain (23 instructions on both GCC 16.1 and Clang 23.1),
        // because bare RISC-V does not guarantee unaligned access.
        for target in [Target::I686, Target::Aarch64] {
            let mut f = chain_func(32, IrType::U32, Flags::default());
            assert_eq!(run_function(&mut f, target), 1, "{target:?}");
        }
        let mut f = chain_func(32, IrType::U32, Flags::default());
        assert_eq!(run_function(&mut f, Target::Riscv64), 0);
        let before = f.blocks[0].instructions.len();
        assert_eq!(f.blocks[0].instructions.len(), before, "untouched");
    }

    #[test]
    fn folds_big_endian_as_a_byte_swap() {
        // `p[0] << 8 | p[1]` in a 32-bit carrier, and the 8-byte form: a wide
        // load plus a byte reversal, which is what every oracle emits.
        for (bits, ty) in [(32u32, IrType::U32), (64, IrType::U64)] {
            let mut f = chain_func(
                bits,
                ty,
                Flags {
                    big_endian: true,
                    ..Flags::default()
                },
            );
            let orig = f.clone();
            assert_eq!(run_function(&mut f, X64), 1, "{bits}-bit big-endian run");
            assert_loads_stay_in_footprint(&orig, &f);
            let loads = wide_loads(&f);
            assert_eq!(loads.len(), 1);
            assert_eq!(loads[0].2, ty, "the load has the carrier's width");
            assert_eq!(resolve(&f, loads[0].1), (Value(0), 0));
            // Exactly one `Bswap`, defining the `Or`'s destination.
            let bswaps: Vec<_> = f.blocks[0]
                .instructions
                .iter()
                .filter_map(|i| match i {
                    Instruction::UnaryOp {
                        dest,
                        op: crate::ir::reexports::IrUnaryOp::Bswap,
                        src,
                        ..
                    } => Some((*dest, *src)),
                    _ => None,
                })
                .collect();
            assert_eq!(bswaps.len(), 1);
            assert_eq!(bswaps[0].1, Operand::Value(loads[0].0));
            match &f.blocks[0].terminator {
                Terminator::Return(Some(Operand::Value(v))) => assert_eq!(*v, bswaps[0].0),
                other => panic!("unexpected terminator {other:?}"),
            }
        }
    }

    #[test]
    fn folds_a_narrow_run_into_a_wider_carrier() {
        // `(uint16_t)p[0] | ((uint16_t)p[1] << 8)` promotes to `int`, so the
        // Or's carrier is 32 bits while the run is two bytes: the rewrite is a
        // 16-bit load plus a zero-extending cast (the high carrier bits are
        // provably zero — every term is a zero-extended byte).
        let mut f = chain_func(
            16,
            IrType::U32,
            Flags {
                narrow_carrier: true,
                ..Flags::default()
            },
        );
        assert_eq!(run_function(&mut f, X64), 1);
        let loads = wide_loads(&f);
        assert_eq!(loads.len(), 1);
        assert_eq!(loads[0].2, IrType::U16, "the load is 16-bit");
        let casts: Vec<_> = f.blocks[0]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Cast {
                    dest,
                    from_ty: IrType::U16,
                    to_ty: IrType::U32,
                    src,
                } => Some((*dest, *src)),
                _ => None,
            })
            .collect();
        assert_eq!(casts.len(), 1, "one zero-extending cast");
        assert_eq!(casts[0].1, Operand::Value(loads[0].0));
        match &f.blocks[0].terminator {
            Terminator::Return(Some(Operand::Value(v))) => assert_eq!(*v, casts[0].0),
            other => panic!("unexpected terminator {other:?}"),
        }
    }

    #[test]
    fn refuses_a_big_endian_run_narrower_than_its_carrier() {
        // `(int)((p[0] << 8) | p[1])`: the reversal would have to be followed
        // by a width-changing conversion (bswap16 then zext). That chain is
        // deliberately left alone rather than risk a width-sensitive sequence
        // in the backend; the little-endian twin is handled above.
        let mut f = chain_func(
            16,
            IrType::U32,
            Flags {
                narrow_carrier: true,
                big_endian: true,
                ..Flags::default()
            },
        );
        assert_eq!(run_function(&mut f, X64), 0);
    }

    #[test]
    fn refuses_volatile_byte_loads() {
        let mut f = chain_func(
            64,
            IrType::U64,
            Flags {
                volatile: true,
                ..Flags::default()
            },
        );
        assert_eq!(run_function(&mut f, X64), 0);
    }

    #[test]
    fn refuses_multi_use_byte_load() {
        let mut f = chain_func(
            64,
            IrType::U64,
            Flags {
                extra_use: true,
                ..Flags::default()
            },
        );
        assert_eq!(run_function(&mut f, X64), 0);
    }

    #[test]
    fn a_duplicated_offset_never_widens_past_the_chain() {
        // Offsets 0,1,2,3,4,5,6,6: the last two terms share a byte. No rewrite
        // may cover the missing byte. A *prefix* that is a complete run (bytes
        // 0..3) is a legitimate fold and is allowed — the invariant is the
        // footprint, not the fold count.
        let orig = chain_func(
            64,
            IrType::U64,
            Flags {
                duplicate_offset: true,
                ..Flags::default()
            },
        );
        let mut f = orig.clone();
        let folded = run_function(&mut f, X64);
        assert!(folded <= 1, "at most the 0..3 prefix folds: {folded}");
        assert_loads_stay_in_footprint(&orig, &f);
    }

    #[test]
    fn refuses_a_store_between_loads_and_merge() {
        let mut f = chain_func(
            64,
            IrType::U64,
            Flags {
                store_inside: true,
                ..Flags::default()
            },
        );
        assert_eq!(run_function(&mut f, X64), 0);
    }

    #[test]
    fn folds_a_base_that_is_a_loaded_pointer() {
        // The base need not be a parameter: the soundness argument is about
        // the bytes touched, not about where the address came from.
        let mut f = chain_func(
            64,
            IrType::U64,
            Flags {
                dynamic_base: true,
                ..Flags::default()
            },
        );
        assert_eq!(run_function(&mut f, X64), 1);
        let loads = wide_loads(&f);
        assert_eq!(loads.len(), 1);
        let (_, ptr, ty, _) = loads[0];
        assert_eq!(ty, IrType::U64);
        // The wide load reads through the pointer the chain's first byte came
        // from (a loaded pointer), not through the original parameter.
        let (root, off) = resolve(&f, ptr);
        assert_eq!(off, 0);
        let root_is_loaded = f.blocks[0]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Load { dest, ty: IrType::Ptr, .. } if *dest == root));
        assert!(root_is_loaded, "the base is the loaded pointer");
    }

    #[test]
    fn folds_a_variable_index_base() {
        // `le32(buf + i)` inlined and unrolled: the anchor is a
        // `GetElementPtr` with a *variable* offset, which therefore becomes
        // the base expression itself.
        let mut f = chain_func(
            32,
            IrType::U32,
            Flags {
                variable_index: true,
                ..Flags::default()
            },
        );
        assert_eq!(run_function(&mut f, X64), 1);
        let loads = wide_loads(&f);
        assert_eq!(loads.len(), 1);
        assert_eq!(loads[0].2, IrType::U32);
        let (root, off) = resolve(&f, loads[0].1);
        assert_eq!(off, 0);
        // The wide load's base is the variable-offset GEP.
        let is_var_gep = f.blocks[0].instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::GetElementPtr { dest, offset: Operand::Value(_), .. } if *dest == root
            )
        });
        assert!(is_var_gep, "the anchor is the variable-index getelementptr");
    }

    #[test]
    fn refuses_a_base_redefined_inside_the_window() {
        // A pointer written between the byte loads and the merge: the wide
        // load would read a different address than the bytes it replaces.
        // This is the restriction that *is* load-bearing — the one the
        // parameter-only rule used to approximate.
        let mut f = chain_func(64, IrType::U64, Flags::default());
        let second_use = {
            let at = f.blocks[0]
                .instructions
                .iter()
                .position(|i| matches!(i, Instruction::Load { ty: IrType::U8, .. }))
                .expect("chain has byte loads");
            at + 1
        };
        f.blocks[0].instructions.insert(
            second_use,
            Instruction::Copy {
                dest: Value(0),
                src: Operand::Value(Value(1000)),
            },
        );
        f.blocks[0].instructions.insert(
            0,
            Instruction::ParamRef {
                dest: Value(1000),
                param_idx: 1,
                ty: IrType::Ptr,
            },
        );
        f.next_value_id = 1001;
        assert_eq!(run_function(&mut f, X64), 0, "base redefined in the window");
    }

    #[test]
    fn folds_when_multi_def_base_is_redefined_outside_the_window() {
        // Post-phi loop-induction pointer: `Value(0)` is defined before the
        // chain and updated by a `Copy` at the bottom of the block (after the
        // merge). Because no definition sits inside `[earliest, top]`,
        // `uniform_in_window` proves the base has one value across the chain.
        let mut f = chain_func(32, IrType::U32, Flags::default());
        f.blocks[0].instructions.push(Instruction::GetElementPtr {
            dest: Value(1000),
            base: Value(0),
            offset: Operand::Const(IrConst::I64(4)),
            ty: IrType::Ptr,
        });
        f.blocks[0].instructions.push(Instruction::Copy {
            dest: Value(0),
            src: Operand::Value(Value(1000)),
        });
        f.next_value_id = 1001;
        assert_eq!(
            run_function(&mut f, X64),
            1,
            "multi-def base updated after the window still folds"
        );
        let loads = wide_loads(&f);
        assert_eq!(loads.len(), 1);
        assert_eq!(loads[0].1, Value(0));
        assert_eq!(loads[0].2, IrType::U32);
    }

    #[test]
    fn refuses_when_base_is_redefined_after_unconsumed_anchor_gep() {
        // When `anchor > 0`, `load_ptr` is an unconsumed `GetElementPtr` at the
        // start of the chain. A redefinition of the base right after that GEP
        // (before the first consumed instruction) must still be caught by the
        // stability window `[earliest, top]`.
        let mut f = chain_func(
            32,
            IrType::U32,
            Flags {
                run_offset: 8,
                ..Flags::default()
            },
        );
        let first_gep = f.blocks[0]
            .instructions
            .iter()
            .position(|i| matches!(i, Instruction::GetElementPtr { .. }))
            .expect("chain has anchor gep");
        f.blocks[0].instructions.insert(
            first_gep + 1,
            Instruction::Copy {
                dest: Value(0),
                src: Operand::Value(Value(1000)),
            },
        );
        f.blocks[0].instructions.insert(
            0,
            Instruction::ParamRef {
                dest: Value(1000),
                param_idx: 1,
                ty: IrType::Ptr,
            },
        );
        f.next_value_id = 1001;
        assert_eq!(
            run_function(&mut f, X64),
            0,
            "base redefined after unconsumed anchor gep must be refused"
        );
    }

    #[test]
    fn second_run_is_a_no_op() {
        let mut f = chain_func(64, IrType::U64, Flags::default());
        assert_eq!(run_function(&mut f, X64), 1);
        assert_eq!(run_function(&mut f, X64), 0, "idempotence");
    }

    #[test]
    fn folds_two_chains_in_one_block() {
        // SipHash shapes three chains into one block: the block rebuild must
        // not let one rewrite invalidate another.
        let mut next = 1u32;
        let mut insns = vec![Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty: IrType::Ptr,
        }];
        let _a = append_chain(
            &mut insns,
            &mut next,
            Value(0),
            32,
            IrType::U32,
            Flags::default(),
        );
        let b = append_chain(
            &mut insns,
            &mut next,
            Value(0),
            64,
            IrType::U64,
            Flags::default(),
        );
        let c = append_chain(
            &mut insns,
            &mut next,
            Value(0),
            64,
            IrType::U64,
            Flags::default(),
        );
        let mut f = IrFunction::new("chains".to_string(), IrType::U64, vec![], false);
        f.next_value_id = next;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            source_spans: vec![],
            instructions: insns,
            terminator: Terminator::Return(Some(Operand::Value(b))),
        });
        let _ = c;
        let before = f.blocks[0].instructions.len();
        assert_eq!(run_function(&mut f, X64), 3, "all three chains fold");
        let loads = wide_loads(&f);
        assert_eq!(loads.len(), 3, "one wide load per chain");
        let mut types: Vec<IrType> = loads.iter().map(|l| l.2).collect();
        types.sort_by_key(|t| t.size());
        assert_eq!(types, vec![IrType::U32, IrType::U64, IrType::U64]);
        assert!(
            f.blocks[0].instructions.len() < before / 3,
            "the block shrinks: {before} -> {}",
            f.blocks[0].instructions.len()
        );
        for (_, ptr, _, _) in loads {
            assert_eq!(resolve(&f, ptr), (Value(0), 0));
        }
    }
}
