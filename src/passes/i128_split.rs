//! Split closed 128-bit integer computations into 64-bit halves.
//!
//! # Why
//!
//! The x86-64 backend keeps every `__int128` value in a 16-byte stack slot
//! and computes through the `%rax:%rdx` accumulator pair: each use reloads
//! two words and each definition stores two.  A value that is carried round
//! a loop therefore makes a store-to-load round trip per iteration, which
//! is the whole of the gap the bignum kernel `tests/bench/k_bn_mul128.c`
//! measures -- GCC and Clang keep the carry in a register and run the step
//! in two cycles, the slot form needs two forwarding hops (about thirteen).
//! Any function containing such a value also loses `%rdx`, `%rsi`, `%rdi`,
//! `%r8` and `%r9` to the wide emitters for its whole body.
//!
//! A 128-bit value is only ever a pair of machine words.  Where the IR can
//! say so, the ordinary 64-bit pipeline -- value numbering, loop passes,
//! the register allocator, the peepholes -- handles each word like any
//! other, and no wide instruction reaches the backend at all.
//!
//! # What is split
//!
//! An *island* is a connected set of 128-bit SSA values (and of the
//! 128-bit locals they are stored in) whose every definition and every use
//! is an operation with an exact two-word form:
//!
//! * widening from, and truncation to, a register-sized integer;
//! * `Add`, `Sub`, `Mul`, `And`, `Or`, `Xor`, `Neg`, `Not`;
//! * shifts by a constant in `0..128`;
//! * comparisons;
//! * `Phi`, `Select`, `Copy`;
//! * plain (non-volatile, default address space) loads and stores.
//!
//! Anything else -- a call argument or result, a returned value, a
//! parameter reference, division, a variable shift, a floating conversion,
//! an atomic or volatile access, inline assembly -- *taints* the island and
//! it is left exactly as it was.  Splitting is all or nothing per island
//! because the halves of a value and the value itself never coexist: there
//! is no "join" instruction, so no partially converted state can be built.
//!
//! An island that only moves data (load, store, copy, merge) is left alone
//! as well: the backend moves 16 bytes with one SSE load and one store, and
//! two word moves would be the slower spelling.
//!
//! A 128-bit local is split with its island when it is only ever loaded and
//! stored whole; it becomes two 64-bit locals, which the `mem2reg` run that
//! follows promotes like any other scalar.  A local whose island is tainted
//! stays in memory, and the analysis is repeated without it so the values
//! that merely pass through it can still be split around it.
//!
//! # The two-word forms
//!
//! With `x = (x.hi << 64) | x.lo`, every operation below is arithmetic
//! modulo 2^64 on the halves; `c` is the carry out of the low addition
//! (`lo <u b.lo`) or the borrow of the low subtraction (`a.lo <u b.lo`).
//!
//! ```text
//! add   lo = a.lo + b.lo            hi = (a.hi + c) + b.hi
//! sub   lo = a.lo - b.lo            hi = (a.hi - c) - b.hi
//! mul   lo = a.lo * b.lo            hi = umulh(a.lo, b.lo) + a.lo*b.hi + a.hi*b.lo
//! shl k (0<k<64)   lo = a.lo << k   hi = a.hi << k | a.lo >> (64-k)
//! shr k (0<k<64)   lo = a.lo >> k | a.hi << (64-k)      hi = a.hi >> k
//! ```
//!
//! The carry joins one high word *before* the other is added: that is the
//! order the x86-64 peepholes (`peephole::passes::multiword`) fuse into
//! `add; adc`, and it keeps the high word one operation deep.
//!
//! When both factors of a product are sign extensions of 64-bit values the
//! high word is one `SMulHigh`; a zero high half folds its cross product
//! away while the form is built, so `(unsigned __int128)a * b` is exactly
//! `Mul` + `UMulHigh`.  The local folder here only removes operations whose
//! operand is a literal zero, one or all-ones; everything else is left to
//! the constant folder and the simplifier, which run next.
//!
//! # Scope
//!
//! x86-64 only.  The carry and borrow tests above are recovered as
//! `adc`/`sbb` by the flags peepholes, and the product pair as one `mul`;
//! AArch64 and RISC-V have no such recovery yet, and their wide emitters
//! already use `adcs`/`sltu` directly, so splitting there would trade a
//! two-instruction add for a four-instruction one.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{
    Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, IrModule, IrUnaryOp, Operand, Value,
};

/// The type of one half.  Halves are unsigned words; the few operations that
/// read a half as signed (`AShr`, `SMulHigh`, signed compares) say so with
/// [`SIGNED_HALF`] at that instruction only.
const HALF: IrType = IrType::U64;
const SIGNED_HALF: IrType = IrType::I64;

fn is_wide(ty: IrType) -> bool {
    matches!(ty, IrType::I128 | IrType::U128)
}

/// A type a wide value may be widened from or truncated to: an integer no
/// wider than a register, or a pointer.  This is the domain of the backend's
/// own integer<->128-bit cast (`emit_cast_default`): everything else there is
/// a floating conversion, which is a library call.
fn is_register_scalar(ty: IrType) -> bool {
    ty == IrType::Ptr || (ty.is_integer() && ty.size() <= 8)
}

fn c64(value: i64) -> Operand {
    Operand::Const(IrConst::I64(value))
}

fn const64(op: &Operand) -> Option<i64> {
    match op {
        Operand::Const(IrConst::I64(v)) => Some(*v),
        _ => None,
    }
}

fn is_zero(op: &Operand) -> bool {
    const64(op) == Some(0)
}

/// The two words of a 128-bit constant operand.  Only the constant kinds the
/// backend reads as a full 128-bit value are accepted: a narrower literal in
/// a 128-bit position has no single agreed extension, so it taints instead.
fn wide_const(op: &Operand) -> Option<(i64, i64)> {
    match op {
        Operand::Const(IrConst::I128(v)) => Some((*v as i64, (*v >> 64) as i64)),
        Operand::Const(IrConst::Zero) => Some((0, 0)),
        _ => None,
    }
}

/// A constant shift count the wide emitters accept (`0..128`).
fn shift_count(op: &Operand) -> Option<u32> {
    match op {
        Operand::Const(c) => c
            .to_i64()
            .filter(|k| (0..128).contains(k))
            .map(|k| k as u32),
        Operand::Value(_) => None,
    }
}

fn plain_access(volatile: bool, seg: AddressSpace) -> bool {
    !volatile && seg == AddressSpace::Default
}

/// Union-find over the wide values and wide locals of one function.
struct Islands {
    index: FxHashMap<u32, usize>,
    parent: Vec<usize>,
    tainted: Vec<bool>,
    computes: Vec<bool>,
    /// The node is a local (an `Alloca`), not an SSA value.
    home: Vec<bool>,
}

impl Islands {
    fn new() -> Self {
        Self {
            index: FxHashMap::default(),
            parent: Vec::new(),
            tainted: Vec::new(),
            computes: Vec::new(),
            home: Vec::new(),
        }
    }

    fn add(&mut self, id: u32, home: bool) -> bool {
        if self.index.contains_key(&id) {
            return false;
        }
        let node = self.parent.len();
        self.index.insert(id, node);
        self.parent.push(node);
        self.tainted.push(false);
        self.computes.push(false);
        self.home.push(home);
        true
    }

    fn find(&mut self, mut node: usize) -> usize {
        while self.parent[node] != node {
            self.parent[node] = self.parent[self.parent[node]];
            node = self.parent[node];
        }
        node
    }

    /// The node of an SSA value (never of a local).
    fn value(&self, op: &Operand) -> Option<usize> {
        match op {
            Operand::Value(v) => self.index.get(&v.0).copied().filter(|&n| !self.home[n]),
            Operand::Const(_) => None,
        }
    }

    fn home_of(&self, ptr: Value) -> Option<usize> {
        self.index.get(&ptr.0).copied().filter(|&n| self.home[n])
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[b] = a;
            self.tainted[a] |= self.tainted[b];
            self.computes[a] |= self.computes[b];
        }
    }

    fn taint(&mut self, node: usize) {
        let root = self.find(node);
        self.tainted[root] = true;
    }

    fn taint_id(&mut self, id: u32) {
        if let Some(&node) = self.index.get(&id) {
            self.taint(node);
        }
    }

    fn mark_computes(&mut self, node: usize) {
        let root = self.find(node);
        self.computes[root] = true;
    }

    fn is_tainted(&mut self, id: u32) -> bool {
        match self.index.get(&id).copied() {
            Some(node) => {
                let root = self.find(node);
                self.tainted[root]
            }
            None => true,
        }
    }

    fn splits(&mut self, id: u32) -> bool {
        match self.index.get(&id).copied() {
            Some(node) => {
                let root = self.find(node);
                !self.tainted[root] && self.computes[root]
            }
            None => false,
        }
    }

    /// A wide operand in an accepted position: join it to `anchor`'s island,
    /// accept a full-width constant, and taint on anything else (a value the
    /// backend does not treat as 128 bits wide, or a narrower literal).
    fn join_operand(&mut self, anchor: usize, op: &Operand) {
        match op {
            Operand::Value(_) => match self.value(op) {
                Some(node) => self.union(anchor, node),
                None => self.taint(anchor),
            },
            Operand::Const(_) => {
                if wide_const(op).is_none() {
                    self.taint(anchor);
                }
            }
        }
    }

    /// Fallback for every instruction without a two-word form: whatever wide
    /// value it defines or reads stays whole.
    fn taint_all(&mut self, inst: &Instruction) {
        if let Some(dest) = inst.dest() {
            self.taint_id(dest.0);
        }
        inst.for_each_used_value(|id| self.taint_id(id));
    }
}

/// Wide locals that are only ever loaded and stored **whole, through their own
/// address**.
///
/// The address half of that rule is load-bearing, and it is the same rule
/// `mem2reg` applies (`find_promotable_allocas`): a local whose address is
/// *derived* from — a `GetElementPtr` off it, a copy of the pointer, a call
/// argument — is no longer a pair of words this pass may rename independently,
/// because the derived address aliases both halves at their real offsets.  The
/// backend already relies on that shape: a 16-byte home reached through a
/// derived `ptr + 8` address is modelled by `isl` as a *general* 8-byte load and
/// may take the `%rax`/`%rdx` pair relay, whose source register is the slot, not
/// the derived address.  Splitting such a home produced exactly that miscompile
/// in `check_ms_abi_varargs.sh` (see `engineering/REDTEAM-2026-11-11-PR1178.md`).
///
/// A 16-byte local that is reached only as `alloca`, or `alloca` plus a zero
/// constant offset (`GetElementPtr { base, offset: 0 }`, which the frontend
/// emits for an aggregate member at offset 0), is a plain home and is eligible.
fn splittable_homes(func: &IrFunction) -> FxHashSet<u32> {
    let params: FxHashSet<u32> = func.param_alloca_values.iter().map(|v| v.0).collect();
    let mut homes = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                ty,
                size: 16,
                volatile: false,
                semantic_volatile: false,
                ..
            } = inst
            {
                if is_wide(*ty) && !params.contains(&dest.0) {
                    homes.insert(dest.0);
                }
            }
        }
    }
    if homes.is_empty() {
        return homes;
    }
    // A home whose address is derived — `GetElementPtr` off it, at any offset —
    // stays in memory.  This is checked before the escape scan because a derived
    // address is *used* by the GEP itself, so the escape scan alone would see the
    // alloca as an ordinary operand of a "supported" instruction and keep it.
    let mut derived = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::GetElementPtr { base, .. } = inst {
                derived.insert(base.0);
            }
        }
    }
    homes.retain(|id| !derived.contains(id));
    let mut escaped = Vec::new();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Load {
                    ptr,
                    ty,
                    volatile,
                    seg_override,
                    ..
                } => {
                    if !(is_wide(*ty) && plain_access(*volatile, *seg_override)) {
                        escaped.push(ptr.0);
                    }
                }
                Instruction::Store {
                    val,
                    ptr,
                    ty,
                    volatile,
                    seg_override,
                } => {
                    if !(is_wide(*ty) && plain_access(*volatile, *seg_override)) {
                        escaped.push(ptr.0);
                    }
                    // The address stored as data.
                    if let Operand::Value(v) = val {
                        escaped.push(v.0);
                    }
                }
                Instruction::Alloca { .. } => {}
                other => other.for_each_used_value(|id| escaped.push(id)),
            }
        }
        block.terminator.for_each_used_value(|id| escaped.push(id));
    }
    for id in escaped {
        homes.remove(&id);
    }
    homes
}

/// Build the islands of `func` with `homes` as the splittable locals.
///
/// Parameter homes are never homes here: before inlining one is about to become
/// an ordinary local of the caller (so touching its words would pin it in
/// memory there), and after it the local it became is handled by
/// [`splittable_homes`] on its own merits.
fn analyse(func: &IrFunction, homes: &FxHashSet<u32>) -> Islands {
    let mut islands = Islands::new();

    // Nodes.  The typing rule is the backend's (`i128_values`): a result
    // type of I128/U128, or a `Copy` of such a value or of an I128 literal.
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca { dest, .. } if homes.contains(&dest.0) => {
                    islands.add(dest.0, true);
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Const(IrConst::I128(_)),
                } => {
                    islands.add(dest.0, false);
                }
                _ => {
                    if let (Some(dest), Some(ty)) = (inst.dest(), inst.result_type()) {
                        if is_wide(ty) {
                            islands.add(dest.0, false);
                        }
                    }
                }
            }
        }
    }
    loop {
        let mut grew = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Copy { dest, src } = inst {
                    if islands.value(src).is_some() {
                        grew |= islands.add(dest.0, false);
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }

    for block in &func.blocks {
        for inst in &block.instructions {
            classify(&mut islands, inst);
        }
        block
            .terminator
            .for_each_used_value(|id| islands.taint_id(id));
    }
    islands
}

/// Classify one instruction into the union-find.
///
/// A wide value is only an island member when *every* definition and use of it
/// has a two-word form.  In particular a wide load/store participates only when
/// its address is a home local of the island; every other 16-byte access is a
/// whole-object transfer the backend already lowers exactly, and stays whole.
fn classify(islands: &mut Islands, inst: &Instruction) {
    match inst {
        Instruction::Alloca { .. } => {}
        Instruction::Load {
            dest,
            ptr,
            ty,
            volatile,
            seg_override,
        } if is_wide(*ty) => {
            // A wide value used as an address is not a shape this pass owns.
            if let Some(address) = islands.value(&Operand::Value(*ptr)) {
                islands.taint(address);
            }
            let Some(node) = islands.value(&Operand::Value(*dest)) else {
                return;
            };
            // A 16-byte read through a pointer the IR cannot name — the
            // Microsoft `va_arg` cursor, a parameter base, any computed
            // address — is a whole-object load the backend already lowers
            // exactly (one `Mov128` through `%xmm0`).  Splitting it would
            // replace that with two 8-byte loads through the *same* pointer,
            // which the wide-instruction model does not describe, and that is
            // the miscompile this rule exists to prevent
            // (`engineering/REDTEAM-2026-11-11-PR1178.md` §1.2).  Only a load
            // through a home local joins that home's island.
            match islands.home_of(*ptr) {
                Some(home) if plain_access(*volatile, *seg_override) => {
                    islands.union(node, home);
                }
                _ => islands.taint(node),
            }
        }
        Instruction::Store {
            val,
            ptr,
            ty,
            volatile,
            seg_override,
        } if is_wide(*ty) => {
            if let Some(address) = islands.value(&Operand::Value(*ptr)) {
                islands.taint(address);
            }
            let plain = plain_access(*volatile, *seg_override);
            let home = islands.home_of(*ptr);
            // The storage twin of the load rule above: only a home local can
            // hold a split value.  A store through a computed address is left
            // whole, exactly like the read.
            match islands.value(val) {
                Some(node) => {
                    if !plain || home.is_none() {
                        islands.taint(node);
                    } else if let Some(home) = home {
                        islands.union(node, home);
                    }
                }
                None => {
                    // A constant, or a value the backend does not hold as
                    // 128 bits: a split local cannot take it unless it is a
                    // full-width literal.
                    if let Some(home) = home {
                        if wide_const(val).is_none() {
                            islands.taint(home);
                        }
                    }
                }
            }
        }
        Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        } if is_wide(*ty) => {
            let Some(node) = islands.value(&Operand::Value(*dest)) else {
                return;
            };
            islands.mark_computes(node);
            match op {
                IrBinOp::Add
                | IrBinOp::Sub
                | IrBinOp::Mul
                | IrBinOp::And
                | IrBinOp::Or
                | IrBinOp::Xor => {
                    islands.join_operand(node, lhs);
                    islands.join_operand(node, rhs);
                }
                IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr => {
                    islands.join_operand(node, lhs);
                    if shift_count(rhs).is_none() {
                        islands.taint(node);
                        if let Some(count) = islands.value(rhs) {
                            islands.taint(count);
                        }
                    }
                }
                _ => islands.taint_all(inst),
            }
        }
        Instruction::UnaryOp { dest, op, src, ty } if is_wide(*ty) => {
            let Some(node) = islands.value(&Operand::Value(*dest)) else {
                return;
            };
            islands.mark_computes(node);
            if matches!(op, IrUnaryOp::Neg | IrUnaryOp::Not) {
                islands.join_operand(node, src);
            } else {
                islands.taint_all(inst);
            }
        }
        Instruction::Cmp { lhs, rhs, ty, .. } if is_wide(*ty) => {
            match (islands.value(lhs), islands.value(rhs)) {
                (Some(a), Some(b)) => {
                    islands.union(a, b);
                    islands.mark_computes(a);
                }
                (Some(node), None) => {
                    islands.mark_computes(node);
                    islands.join_operand(node, rhs);
                }
                (None, Some(node)) => {
                    islands.mark_computes(node);
                    islands.join_operand(node, lhs);
                }
                (None, None) => {}
            }
        }
        Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        } if is_wide(*from_ty) || is_wide(*to_ty) => {
            match (is_wide(*from_ty), is_wide(*to_ty)) {
                // Signedness reinterpretation: the same two words.
                (true, true) => {
                    if let Some(node) = islands.value(&Operand::Value(*dest)) {
                        islands.join_operand(node, src);
                    }
                }
                // Widening.
                (false, true) => {
                    let Some(node) = islands.value(&Operand::Value(*dest)) else {
                        return;
                    };
                    islands.mark_computes(node);
                    if !is_register_scalar(*from_ty) {
                        islands.taint(node);
                    }
                    if let Some(source) = islands.value(src) {
                        islands.taint(source);
                        islands.taint(node);
                    }
                }
                // Truncation.
                (true, false) => {
                    if let Some(node) = islands.value(src) {
                        islands.mark_computes(node);
                        if !is_register_scalar(*to_ty) {
                            islands.taint(node);
                        }
                    }
                }
                (false, false) => {}
            }
        }
        Instruction::Phi { dest, ty, incoming } if is_wide(*ty) => {
            if let Some(node) = islands.value(&Operand::Value(*dest)) {
                for (op, _) in incoming {
                    islands.join_operand(node, op);
                }
            }
        }
        Instruction::Select {
            dest,
            cond,
            true_val,
            false_val,
            ty,
        } if is_wide(*ty) => {
            if let Some(node) = islands.value(&Operand::Value(*dest)) {
                islands.join_operand(node, true_val);
                islands.join_operand(node, false_val);
                if let Some(condition) = islands.value(cond) {
                    islands.taint(condition);
                    islands.taint(node);
                }
            }
        }
        Instruction::Copy { dest, src } => {
            if let Some(node) = islands.value(&Operand::Value(*dest)) {
                // A wide literal source needs no join; a wide value does.
                if !matches!(src, Operand::Const(IrConst::I128(_))) {
                    islands.join_operand(node, src);
                }
            } else {
                islands.taint_all(inst);
            }
        }
        _ => islands.taint_all(inst),
    }
}

/// The two words of one split value.  A half is either a fresh SSA value or,
/// where the defining instruction alone fixes it, a literal.
#[derive(Clone, Copy)]
struct Halves {
    lo: Operand,
    hi: Operand,
}

/// Instruction builder with the local folds described in the module docs.
struct Emit<'a> {
    next_value: &'a mut u32,
    out: Vec<Instruction>,
}

impl Emit<'_> {
    fn fresh(&mut self) -> Value {
        let value = Value(*self.next_value);
        *self.next_value += 1;
        value
    }

    /// Give `target` the value `source`.  A literal target is a half its
    /// definition already fixed, so there is nothing to emit.
    fn assign(&mut self, target: Operand, source: Operand) {
        if let Operand::Value(dest) = target {
            if source != target {
                self.out.push(Instruction::Copy { dest, src: source });
            }
        }
    }

    fn bin(&mut self, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Operand {
        if let Some(folded) = fold_bin(op, &lhs, &rhs) {
            return folded;
        }
        let dest = self.fresh();
        self.out.push(Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        });
        Operand::Value(dest)
    }

    /// [`Self::bin`] delivering into `target`.
    fn bin_into(&mut self, target: Operand, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) {
        let Operand::Value(dest) = target else {
            return;
        };
        match fold_bin(op, &lhs, &rhs) {
            Some(folded) => self.assign(target, folded),
            None => self.out.push(Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            }),
        }
    }

    fn cmp(&mut self, op: IrCmpOp, lhs: Operand, rhs: Operand, ty: IrType) -> Operand {
        let dest = self.fresh();
        self.out.push(Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        });
        Operand::Value(dest)
    }

    /// A comparison result (0 or 1, typed I8) as a word.
    fn flag_word(&mut self, flag: Operand) -> Operand {
        let dest = self.fresh();
        self.out.push(Instruction::Cast {
            dest,
            src: flag,
            from_ty: IrType::I8,
            to_ty: HALF,
        });
        Operand::Value(dest)
    }

    /// `a + b` into `h`.
    fn add(&mut self, a: Halves, b: Halves, h: Halves) {
        // A zero low word carries nothing.
        if is_zero(&a.lo) || is_zero(&b.lo) {
            let low = if is_zero(&a.lo) { b.lo } else { a.lo };
            self.assign(h.lo, low);
            self.bin_into(h.hi, IrBinOp::Add, a.hi, b.hi, HALF);
            return;
        }
        self.bin_into(h.lo, IrBinOp::Add, a.lo, b.lo, HALF);
        // `low <u x` for either addend x is the carry; an immediate addend
        // keeps the other one's live range from stretching past the add.
        let addend = if const64(&a.lo).is_some() { a.lo } else { b.lo };
        let carry = self.cmp(IrCmpOp::Ult, h.lo, addend, HALF);
        let carry = self.flag_word(carry);
        // The carry joins one high word first (as the right operand, so the
        // word is staged and the carry added to it); the other word follows.
        let (first, second) = if is_zero(&a.hi) {
            (b.hi, a.hi)
        } else {
            (a.hi, b.hi)
        };
        let with_carry = self.bin(IrBinOp::Add, first, carry, HALF);
        self.bin_into(h.hi, IrBinOp::Add, with_carry, second, HALF);
    }

    /// `a - b` into `h`.
    fn sub(&mut self, a: Halves, b: Halves, h: Halves) {
        // A zero low word borrows nothing.
        if is_zero(&b.lo) {
            self.assign(h.lo, a.lo);
            self.bin_into(h.hi, IrBinOp::Sub, a.hi, b.hi, HALF);
            return;
        }
        // The subtraction first, then the comparison of the same operands:
        // adjacent, the comparison is the borrow the subtraction just left
        // in the carry flag.
        self.bin_into(h.lo, IrBinOp::Sub, a.lo, b.lo, HALF);
        let borrow = self.cmp(IrCmpOp::Ult, a.lo, b.lo, HALF);
        let borrow = self.flag_word(borrow);
        let less_borrow = self.bin(IrBinOp::Sub, a.hi, borrow, HALF);
        self.bin_into(h.hi, IrBinOp::Sub, less_borrow, b.hi, HALF);
    }

    /// `a * b` into `h`.  `signed_pair`: both factors are sign extensions of
    /// their low words, so the high word of the product is `SMulHigh`.
    fn mul(&mut self, a: Halves, b: Halves, h: Halves, signed_pair: bool) {
        // The cross products first; then the high word and the low product
        // of the same two operands, adjacent: one `mul` on x86-64.
        if signed_pair {
            self.bin_into(h.hi, IrBinOp::SMulHigh, a.lo, b.lo, SIGNED_HALF);
        } else {
            let cross_right = self.bin(IrBinOp::Mul, a.lo, b.hi, HALF);
            let cross_left = self.bin(IrBinOp::Mul, a.hi, b.lo, HALF);
            let cross = self.bin(IrBinOp::Add, cross_right, cross_left, HALF);
            let high = self.bin(IrBinOp::UMulHigh, a.lo, b.lo, HALF);
            self.bin_into(h.hi, IrBinOp::Add, high, cross, HALF);
        }
        self.bin_into(h.lo, IrBinOp::Mul, a.lo, b.lo, HALF);
    }

    /// `a SHIFT count` into `h`, `count` in `0..128`.
    fn shift(&mut self, op: IrBinOp, a: Halves, count: u32, h: Halves) {
        let amount = |k: u32| c64(i64::from(k));
        let (lo, hi) = match (op, count) {
            (_, 0) => (a.lo, a.hi),
            (IrBinOp::Shl, 1..=63) => {
                let spill = self.bin(IrBinOp::LShr, a.lo, amount(64 - count), HALF);
                let kept = self.bin(IrBinOp::Shl, a.hi, amount(count), HALF);
                (
                    self.bin(IrBinOp::Shl, a.lo, amount(count), HALF),
                    self.bin(IrBinOp::Or, kept, spill, HALF),
                )
            }
            (IrBinOp::Shl, 64) => (c64(0), a.lo),
            (IrBinOp::Shl, _) => (
                c64(0),
                self.bin(IrBinOp::Shl, a.lo, amount(count - 64), HALF),
            ),
            (IrBinOp::LShr | IrBinOp::AShr, 1..=63) => {
                let spill = self.bin(IrBinOp::Shl, a.hi, amount(64 - count), HALF);
                let kept = self.bin(IrBinOp::LShr, a.lo, amount(count), HALF);
                let low = self.bin(IrBinOp::Or, kept, spill, HALF);
                let high = if op == IrBinOp::AShr {
                    self.bin(IrBinOp::AShr, a.hi, amount(count), SIGNED_HALF)
                } else {
                    self.bin(IrBinOp::LShr, a.hi, amount(count), HALF)
                };
                (low, high)
            }
            (IrBinOp::LShr, 64) => (a.hi, c64(0)),
            (IrBinOp::LShr, _) => (
                self.bin(IrBinOp::LShr, a.hi, amount(count - 64), HALF),
                c64(0),
            ),
            (IrBinOp::AShr, 64) => (a.hi, self.bin(IrBinOp::AShr, a.hi, amount(63), SIGNED_HALF)),
            (_, _) => (
                self.bin(IrBinOp::AShr, a.hi, amount(count - 64), SIGNED_HALF),
                self.bin(IrBinOp::AShr, a.hi, amount(63), SIGNED_HALF),
            ),
        };
        self.assign(h.lo, lo);
        self.assign(h.hi, hi);
    }

    /// `dest = a OP b`, a 0/1 byte like every comparison result.
    fn compare(&mut self, dest: Value, op: IrCmpOp, a: Halves, b: Halves) {
        if matches!(op, IrCmpOp::Eq | IrCmpOp::Ne) {
            let low = self.bin(IrBinOp::Xor, a.lo, b.lo, HALF);
            let high = self.bin(IrBinOp::Xor, a.hi, b.hi, HALF);
            let any = self.bin(IrBinOp::Or, low, high, HALF);
            self.out.push(Instruction::Cmp {
                dest,
                op,
                lhs: any,
                rhs: c64(0),
                ty: HALF,
            });
            return;
        }
        // The high words decide unless they are equal; then the low words
        // do, always as unsigned.
        use IrCmpOp::{Sge, Sgt, Sle, Slt, Uge, Ugt, Ule, Ult};
        let (strict, strict_ty, low_op) = match op {
            Ult => (Ult, HALF, Ult),
            Ule => (Ult, HALF, Ule),
            Ugt => (Ugt, HALF, Ugt),
            Uge => (Ugt, HALF, Uge),
            Slt => (Slt, SIGNED_HALF, Ult),
            Sle => (Slt, SIGNED_HALF, Ule),
            Sgt => (Sgt, SIGNED_HALF, Ugt),
            _ => (Sgt, SIGNED_HALF, Uge),
        };
        let decided = self.cmp(strict, a.hi, b.hi, strict_ty);
        let tied = self.cmp(IrCmpOp::Eq, a.hi, b.hi, HALF);
        let low = self.cmp(low_op, a.lo, b.lo, HALF);
        let tie_break = self.fresh();
        self.out.push(Instruction::BinOp {
            dest: tie_break,
            op: IrBinOp::And,
            lhs: tied,
            rhs: low,
            ty: IrType::I8,
        });
        self.out.push(Instruction::BinOp {
            dest,
            op: IrBinOp::Or,
            lhs: decided,
            rhs: Operand::Value(tie_break),
            ty: IrType::I8,
        });
    }
}

/// Fold a 64-bit operation whose result a literal operand already decides.
/// Returns `None` whenever an instruction is still needed.
fn fold_bin(op: IrBinOp, lhs: &Operand, rhs: &Operand) -> Option<Operand> {
    use IrBinOp::{AShr, Add, And, LShr, Mul, Or, SMulHigh, Shl, Sub, UMulHigh, Xor};
    let (l, r) = (const64(lhs), const64(rhs));
    if let (Some(a), Some(b)) = (l, r) {
        let (ua, ub) = (a as u64, b as u64);
        let value = match op {
            Add => ua.wrapping_add(ub),
            Sub => ua.wrapping_sub(ub),
            Mul => ua.wrapping_mul(ub),
            And => ua & ub,
            Or => ua | ub,
            Xor => ua ^ ub,
            Shl if ub < 64 => ua << ub,
            LShr if ub < 64 => ua >> ub,
            AShr if ub < 64 => (a >> ub) as u64,
            UMulHigh => ((u128::from(ua) * u128::from(ub)) >> 64) as u64,
            SMulHigh => ((i128::from(a) * i128::from(b)) >> 64) as u64,
            _ => return None,
        };
        return Some(c64(value as i64));
    }
    match (op, l, r) {
        (Add | Or | Xor, Some(0), _) => Some(*rhs),
        (Add | Sub | Or | Xor | Shl | LShr | AShr, _, Some(0)) => Some(*lhs),
        (Mul | And | UMulHigh | SMulHigh, Some(0), _)
        | (Mul | And | UMulHigh | SMulHigh, _, Some(0))
        | (Shl | LShr | AShr, Some(0), _)
        | (UMulHigh, Some(1), _)
        | (UMulHigh, _, Some(1)) => Some(c64(0)),
        (Mul, Some(1), _) | (And, Some(-1), _) => Some(*rhs),
        (Mul, _, Some(1)) | (And, _, Some(-1)) => Some(*lhs),
        (Or, Some(-1), _) | (Or, _, Some(-1)) => Some(c64(-1)),
        _ => None,
    }
}

struct Splitter<'a> {
    next_value: &'a mut u32,
    halves: FxHashMap<u32, Halves>,
    /// Split local -> its two 64-bit locals.
    homes: FxHashMap<u32, (Value, Value)>,
    /// Split values that are the sign extension of their low word.
    sign_extended: FxHashSet<u32>,
}

impl Splitter<'_> {
    fn fresh(&mut self) -> Value {
        let value = Value(*self.next_value);
        *self.next_value += 1;
        value
    }

    fn involves(&self, id: u32) -> bool {
        self.halves.contains_key(&id) || self.homes.contains_key(&id)
    }

    /// Allocate the halves of the value `inst` defines.  A half the defining
    /// instruction alone fixes is a literal: uses then fold while they are
    /// built, instead of waiting for the constant folder.
    fn declare(&mut self, inst: &Instruction, dest: Value) {
        let mut halves = Halves {
            lo: Operand::Value(self.fresh()),
            hi: Operand::Value(self.fresh()),
        };
        match inst {
            Instruction::Cast { from_ty, to_ty, .. } if is_wide(*to_ty) && !is_wide(*from_ty) => {
                if from_ty.is_signed() {
                    self.sign_extended.insert(dest.0);
                } else {
                    halves.hi = c64(0);
                }
            }
            Instruction::BinOp {
                op: IrBinOp::LShr,
                rhs,
                ..
            } if shift_count(rhs).is_some_and(|k| k >= 64) => halves.hi = c64(0),
            Instruction::BinOp {
                op: IrBinOp::Shl,
                rhs,
                ..
            } if shift_count(rhs).is_some_and(|k| k >= 64) => halves.lo = c64(0),
            Instruction::Copy {
                src: src @ Operand::Const(_),
                ..
            } => {
                if let Some((lo, hi)) = wide_const(src) {
                    halves = Halves {
                        lo: c64(lo),
                        hi: c64(hi),
                    };
                }
            }
            _ => {}
        }
        self.halves.insert(dest.0, halves);
    }

    /// The two addresses of a wide object at `ptr`: the halves of a split
    /// local, or `ptr` and `ptr + 8` in memory (little-endian: x86-64 only).
    fn addresses(&mut self, ptr: Value, out: &mut Vec<Instruction>) -> (Value, Value) {
        if let Some(&pair) = self.homes.get(&ptr.0) {
            return pair;
        }
        let high = self.fresh();
        out.push(Instruction::GetElementPtr {
            dest: high,
            base: ptr,
            offset: c64(8),
            ty: IrType::Ptr,
        });
        (ptr, high)
    }

    /// The two-word form of `inst`, or `None` when `inst` has none.
    fn rewrite(&mut self, inst: &Instruction) -> Option<Vec<Instruction>> {
        let mut out = Vec::new();
        match inst {
            Instruction::Alloca { dest, .. } => {
                let &(lo, hi) = self.homes.get(&dest.0)?;
                for half in [lo, hi] {
                    out.push(Instruction::Alloca {
                        dest: half,
                        ty: HALF,
                        size: 8,
                        align: 0,
                        volatile: false,
                        semantic_volatile: false,
                    });
                }
                return Some(out);
            }
            Instruction::Load { dest, ptr, .. } => {
                let h = self.halves.get(&dest.0).copied()?;
                let (low, high) = self.addresses(*ptr, &mut out);
                for (half, address) in [(h.lo, low), (h.hi, high)] {
                    let Operand::Value(dest) = half else {
                        return None;
                    };
                    out.push(Instruction::Load {
                        dest,
                        ptr: address,
                        ty: HALF,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    });
                }
                return Some(out);
            }
            Instruction::Store { val, ptr, .. } => {
                let v = operand_halves(&self.halves, val)?;
                let (low, high) = self.addresses(*ptr, &mut out);
                for (half, address) in [(v.lo, low), (v.hi, high)] {
                    out.push(Instruction::Store {
                        val: half,
                        ptr: address,
                        ty: HALF,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    });
                }
                return Some(out);
            }
            Instruction::Phi { dest, incoming, .. } => {
                let h = self.halves.get(&dest.0).copied()?;
                let mut low = Vec::with_capacity(incoming.len());
                let mut high = Vec::with_capacity(incoming.len());
                for (op, block) in incoming {
                    let v = operand_halves(&self.halves, op)?;
                    low.push((v.lo, *block));
                    high.push((v.hi, *block));
                }
                for (half, incoming) in [(h.lo, low), (h.hi, high)] {
                    let Operand::Value(dest) = half else {
                        return None;
                    };
                    out.push(Instruction::Phi {
                        dest,
                        ty: HALF,
                        incoming,
                    });
                }
                return Some(out);
            }
            _ => {}
        }

        let mut emit = Emit {
            next_value: &mut *self.next_value,
            out,
        };
        match inst {
            Instruction::Cast {
                dest,
                src,
                from_ty,
                to_ty,
            } => match (is_wide(*from_ty), is_wide(*to_ty)) {
                (true, true) => {
                    let h = self.halves.get(&dest.0).copied()?;
                    let v = operand_halves(&self.halves, src)?;
                    emit.assign(h.lo, v.lo);
                    emit.assign(h.hi, v.hi);
                }
                (false, true) => {
                    let h = self.halves.get(&dest.0).copied()?;
                    let Operand::Value(low) = h.lo else {
                        return None;
                    };
                    // Mirrors the backend's widening: extend to 64 bits by
                    // the source signedness, then sign- or zero-fill.
                    if from_ty.size() < 8 {
                        emit.out.push(Instruction::Cast {
                            dest: low,
                            src: *src,
                            from_ty: *from_ty,
                            to_ty: if from_ty.is_signed() {
                                SIGNED_HALF
                            } else {
                                HALF
                            },
                        });
                    } else {
                        emit.out.push(Instruction::Copy {
                            dest: low,
                            src: *src,
                        });
                    }
                    if from_ty.is_signed() {
                        emit.bin_into(h.hi, IrBinOp::AShr, h.lo, c64(63), SIGNED_HALF);
                    }
                }
                (true, false) => {
                    let v = operand_halves(&self.halves, src)?;
                    emit.out.push(if to_ty.size() < 8 {
                        Instruction::Cast {
                            dest: *dest,
                            src: v.lo,
                            from_ty: SIGNED_HALF,
                            to_ty: *to_ty,
                        }
                    } else {
                        Instruction::Copy {
                            dest: *dest,
                            src: v.lo,
                        }
                    });
                }
                (false, false) => return None,
            },
            Instruction::BinOp {
                dest, op, lhs, rhs, ..
            } => {
                let h = self.halves.get(&dest.0).copied()?;
                let a = operand_halves(&self.halves, lhs)?;
                match op {
                    IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr => {
                        emit.shift(*op, a, shift_count(rhs)?, h);
                    }
                    _ => {
                        let b = operand_halves(&self.halves, rhs)?;
                        match op {
                            IrBinOp::Add => emit.add(a, b, h),
                            IrBinOp::Sub => emit.sub(a, b, h),
                            IrBinOp::Mul => {
                                let signed_pair = is_sign_extended(&self.sign_extended, lhs)
                                    && is_sign_extended(&self.sign_extended, rhs);
                                emit.mul(a, b, h, signed_pair);
                            }
                            IrBinOp::And | IrBinOp::Or | IrBinOp::Xor => {
                                emit.bin_into(h.lo, *op, a.lo, b.lo, HALF);
                                emit.bin_into(h.hi, *op, a.hi, b.hi, HALF);
                            }
                            _ => return None,
                        }
                    }
                }
            }
            Instruction::UnaryOp { dest, op, src, .. } => {
                let h = self.halves.get(&dest.0).copied()?;
                let v = operand_halves(&self.halves, src)?;
                match op {
                    IrUnaryOp::Neg => {
                        let zero = Halves {
                            lo: c64(0),
                            hi: c64(0),
                        };
                        emit.sub(zero, v, h);
                    }
                    IrUnaryOp::Not => {
                        emit.bin_into(h.lo, IrBinOp::Xor, v.lo, c64(-1), HALF);
                        emit.bin_into(h.hi, IrBinOp::Xor, v.hi, c64(-1), HALF);
                    }
                    _ => return None,
                }
            }
            Instruction::Cmp {
                dest, op, lhs, rhs, ..
            } => {
                let a = operand_halves(&self.halves, lhs)?;
                let b = operand_halves(&self.halves, rhs)?;
                emit.compare(*dest, *op, a, b);
            }
            Instruction::Select {
                dest,
                cond,
                true_val,
                false_val,
                ..
            } => {
                let h = self.halves.get(&dest.0).copied()?;
                let t = operand_halves(&self.halves, true_val)?;
                let f = operand_halves(&self.halves, false_val)?;
                for (half, true_val, false_val) in [(h.lo, t.lo, f.lo), (h.hi, t.hi, f.hi)] {
                    let Operand::Value(dest) = half else {
                        return None;
                    };
                    emit.out.push(Instruction::Select {
                        dest,
                        cond: *cond,
                        true_val,
                        false_val,
                        ty: HALF,
                    });
                }
            }
            Instruction::Copy { dest, src } => {
                let h = self.halves.get(&dest.0).copied()?;
                let v = operand_halves(&self.halves, src)?;
                emit.assign(h.lo, v.lo);
                emit.assign(h.hi, v.hi);
            }
            _ => return None,
        }
        Some(emit.out)
    }
}

/// Whether `op` is the sign extension of its own low word.
fn is_sign_extended(known: &FxHashSet<u32>, op: &Operand) -> bool {
    match op {
        Operand::Value(v) => known.contains(&v.0),
        Operand::Const(_) => wide_const(op).is_some_and(|(lo, hi)| hi == lo >> 63),
    }
}

fn operand_halves(halves: &FxHashMap<u32, Halves>, op: &Operand) -> Option<Halves> {
    match op {
        Operand::Value(v) => halves.get(&v.0).copied(),
        Operand::Const(_) => wide_const(op).map(|(lo, hi)| Halves {
            lo: c64(lo),
            hi: c64(hi),
        }),
    }
}

/// Split the closed islands of `func`.  Returns the number of 128-bit values
/// and locals that no longer exist.
///
/// The pass is idempotent and order-independent: an island either has a
/// two-word form for every definition and use (and is split) or is left
/// byte-for-byte as it was.  Running it before and after inlining therefore
/// needs no mode flag — the second run simply sees the islands the inliner
/// closed, and re-examines the ones the first run left whole.
pub(crate) fn run_function(func: &mut IrFunction) -> usize {
    if func.is_declaration || func.blocks.is_empty() {
        return 0;
    }
    let has_wide = func.blocks.iter().any(|block| {
        block
            .instructions
            .iter()
            .any(|inst| inst.result_type().is_some_and(is_wide))
    });
    if !has_wide {
        return 0;
    }
    // One definition per value is what makes "the defining instruction" and
    // "every use" well defined; a function that is not in that form is left
    // alone rather than reasoned about.
    let mut defined = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                if !defined.insert(dest.0) {
                    return 0;
                }
            }
        }
    }

    let mut homes = splittable_homes(func);
    let mut islands = loop {
        let mut islands = analyse(func, &homes);
        let before = homes.len();
        homes.retain(|&id| !islands.is_tainted(id));
        if homes.len() == before {
            break islands;
        }
    };

    let mut next_value = func.next_value_id;
    let mut splitter = Splitter {
        next_value: &mut next_value,
        halves: FxHashMap::default(),
        homes: FxHashMap::default(),
        sign_extended: FxHashSet::default(),
    };
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca { dest, .. } => {
                    if islands.splits(dest.0) {
                        let pair = (splitter.fresh(), splitter.fresh());
                        splitter.homes.insert(dest.0, pair);
                    }
                }
                _ => {
                    if let Some(dest) = inst.dest() {
                        if islands.splits(dest.0) {
                            splitter.declare(inst, dest);
                        }
                    }
                }
            }
        }
    }
    let split = splitter.halves.len() + splitter.homes.len();
    if split == 0 {
        return 0;
    }

    // Plan every replacement before touching the function: an instruction
    // that reads or writes a split value and has no two-word form would
    // leave a half-converted body, so the whole function is abandoned.
    let mut plan: Vec<(usize, usize, Vec<Instruction>)> = Vec::new();
    for (block_index, block) in func.blocks.iter().enumerate() {
        for (index, inst) in block.instructions.iter().enumerate() {
            let mut involved = inst.dest().is_some_and(|dest| splitter.involves(dest.0));
            inst.for_each_used_value(|id| involved |= splitter.involves(id));
            if !involved {
                continue;
            }
            match splitter.rewrite(inst) {
                Some(replacement) => plan.push((block_index, index, replacement)),
                None => return 0,
            }
        }
        let mut escapes = false;
        block
            .terminator
            .for_each_used_value(|id| escapes |= splitter.involves(id));
        if escapes {
            return 0;
        }
    }

    for (block_index, index, replacement) in plan.into_iter().rev() {
        func.blocks[block_index].splice_inst(index, replacement);
    }
    func.next_value_id = next_value;
    split
}

/// Run on every function.  Returns the number of functions changed.
pub(crate) fn run(module: &mut IrModule) -> usize {
    module.for_each_function(|func| usize::from(run_function(func) > 0))
}

#[cfg(test)]
#[path = "i128_split_tests.rs"]
mod tests;
