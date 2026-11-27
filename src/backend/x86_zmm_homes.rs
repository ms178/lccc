//! Homes in `zmm16`..`zmm31` for 512-bit vector objects (x86-64).
//!
//! A vector has no IR type. A 512-bit value is a 64-byte `Alloca`, every
//! operation on it is an intrinsic in pointer form, and every copy the
//! inliner leaves behind (an argument passed by value, a result returned)
//! is a `Memcpy` of 64 bytes:
//!
//! ```text
//!   %a = alloca 64, align 64
//!   %t = alloca 64, align 64
//!   memcpy %t, %a, 64                        ; _mm512_add_epi32(a, b)
//!   memcpy %u, %b, 64
//!   intrinsic Paddd512 dest_ptr=%r, args=[%t, %u]
//! ```
//!
//! The 128- and 256-bit widths have register families, an SSA promotion
//! (`vec_ssa`) and register homes. The 512-bit width had none of it, so
//! each of these lines was memory traffic: a 64-byte-aligned slot is
//! addressed through `leaq`/`andq $-64`, the copy went through two `ymm`
//! halves that the `zmm` load behind it cannot be forwarded from, and a
//! `Memcpy` is a call point to the general registers. A loop of six
//! 512-bit operations ran 42 times slower than GCC 16.2's (2.57 s against
//! 0.061 s; with the homes 0.060 s, and the gate
//! `tests/regression/check_x86_zmm_homes.py` keeps the shape).
//!
//! Nothing in this back end hands out `zmm16`..`zmm31`: its scratch is
//! `zmm0`..`zmm2`, its allocator's pool ends at `xmm15`, `vzeroupper` leaves
//! them alone, and both calling conventions leave them to the caller. A
//! function that has a 512-bit value has AVX-512F, which is what the
//! sixteen registers need. So an object that is only ever read and written
//! whole can simply live in one of them:
//!
//! ```text
//!   vpaddd %zmm17, %zmm16, %zmm18
//! ```
//!
//! [`assign`] decides which objects those are, behind the last pass that
//! can add a use of an object or a call:
//!
//! 1. A candidate is a 64-byte `Alloca`, not volatile and not a parameter's
//!    home, whose every mention is the source or destination of a 64-byte
//!    `Memcpy` or an operand position that [`signature`] says is read or
//!    written whole. One mention of any other kind (a lane access, a call
//!    argument, an address taken) leaves the object in memory.
//! 2. Liveness of the candidates is computed over the CFG. Two candidates
//!    interfere when one is written while the other holds a value that is
//!    still read, except that a copy does not make its two ends interfere.
//!    A candidate that is live across anything that can run other code (a
//!    call, a library helper, an `asm` statement) stays in memory: the
//!    registers are not preserved by anyone.
//! 3. The two ends of a copy that do not interfere become one class, and
//!    the graph is built again, until no copy is left to merge: the names
//!    an inlined call gives its arguments are the register of the value.
//!    The classes are coloured greedily in the order they are first
//!    written. Sixteen colours; what does not get one stays in memory.
//! 4. A 64-byte `Memcpy` that touches a home becomes `Storeu512`, which is
//!    "copy 64 bytes from `args[0]` to `dest_ptr`" for a home or an address
//!    at either end, which is not a call point, and which is no instruction
//!    at all between two objects that share a register.
//!
//! The homes are recorded in [`IrFunction::zmm_homes`]; the emitters name
//! the register wherever the object is an operand
//! (`backend/x86/codegen/intrinsics_simd.rs`), and the object gets no stack
//! slot, so nothing can read a stale one. The IR itself is left saying what
//! it said: the same objects, the same copies. Whatever runs between here
//! and the emitters reasons about a program, not about registers, and
//! [`homes_hold`] is asked once more before the registers are believed: a
//! function it says no to is compiled as if this module did not exist.

use crate::backend::liveness::instruction_is_call_point;
use crate::common::fx_hash::FxHashMap;
use crate::ir::analysis::{FlatAdj, build_cfg, build_label_map};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{BlockId, Instruction, IrFunction, IrModule, Operand, Value};

/// The size of the objects this module homes: one `zmm` register.
const BYTES: usize = 64;

/// The first register a home can be.
pub(crate) const FIRST_HOME: u8 = 16;

/// How many there are: `zmm16`..`zmm31`.
pub(crate) const HOME_COUNT: usize = 16;

/// The name of a home register.
pub(crate) fn home_name(register: u8) -> &'static str {
    const NAMES: [&str; HOME_COUNT] = [
        "zmm16", "zmm17", "zmm18", "zmm19", "zmm20", "zmm21", "zmm22", "zmm23", "zmm24", "zmm25",
        "zmm26", "zmm27", "zmm28", "zmm29", "zmm30", "zmm31",
    ];
    NAMES[usize::from(register - FIRST_HOME)]
}

/// How an intrinsic touches its 512-bit operands.
///
/// A row is a promise about the emitter of the operation: it reads each
/// vector argument whole through the home-aware operand helpers, and it
/// writes `dest_ptr` whole, after the last read. The second half is what
/// lets a result share the register of a source that dies at the
/// operation. `every_signature_holds_for_its_emitter` in
/// `intrinsics_simd.rs` runs each row through its emitter and checks both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Signature {
    /// `dest_ptr` receives a whole 512-bit value.
    pub writes_dest: bool,
    /// Bit `i`: argument `i` is a 512-bit vector, read whole.
    pub vector_args: u8,
    /// The argument the instruction's destination register is also a
    /// source for (`vpternlogd`, `vpdpbusd`, the fused multiply-adds): the
    /// operation is one instruction where that argument's register is the
    /// result's, and needs a move first where it is not.
    pub tied: Option<u8>,
}

/// The operations a homed object may be an operand of.
pub(crate) fn signature(op: IntrinsicOp) -> Option<Signature> {
    use IntrinsicOp::*;
    let row = |writes_dest, vector_args| {
        Some(Signature {
            writes_dest,
            vector_args,
            tied: None,
        })
    };
    let tied = |argument| {
        Some(Signature {
            writes_dest: true,
            vector_args: 0b111,
            tied: Some(argument),
        })
    };
    match op {
        // `emit_evex_binary_512`: (a, b).
        Paddb512 | Paddw512 | Paddd512 | Paddq512 | Psubb512 | Psubw512 | Psubd512 | Psubq512
        | Paddsb512 | Paddsw512 | Paddusb512 | Paddusw512 | Psubsb512 | Psubsw512 | Psubusb512
        | Psubusw512 | Pavgb512 | Pavgw512 | Pmaxub512 | Pminub512 | Pmaxuw512 | Pminuw512
        | Pmaxsb512 | Pminsb512 | Pmaxsw512 | Pminsw512 | Pmaxsd512 | Pminsd512 | Pmaxud512
        | Pminud512 | Pmaxsq512 | Pminsq512 | Pmaxuq512 | Pminuq512 | Psadbw512 | Pmaddubsw512
        | Pmaddwd512 | Pmullw512 | Pmulhw512 | Pmulhuw512 | Pmulld512 | Pmuludq512 | Pxor512
        | Por512 | Pand512 | Pandn512 | Pshufb512 | Punpcklbw512 | Punpcklwd512 | Punpckldq512
        | Punpcklqdq512 | Punpckhbw512 | Punpckhwd512 | Punpckhdq512 | Punpckhqdq512
        | Packsswb512 | Packuswb512 | Packssdw512 | Packusdw512 | PermutexvarEp32
        | PermutexvarEp64
        // `emit_evex_fp_binary_512`: (a, b).
        | AddPs512 | SubPs512 | MulPs512 | DivPs512 | MinPs512 | MaxPs512 | AddPd512 | SubPd512
        | MulPd512 | DivPd512 | MinPd512 | MaxPd512
        // `emit_evex_3src_imm_512`: (a, b, immediate).
        | Palignr512 | Vpclmulqdq512 => row(true, 0b011),
        // `emit_evex_unary_512`: (a).
        Pabsb512 | Pabsw512 | Pabsd512 | Pabsq512 | Popcntb512 | Popcntw512 | Popcntd512
        | Popcntq512 | SqrtPs512 | SqrtPd512 | CvtEp32_2Ps512 | CvtPs2Ep32_512 | CvttPs2Ep32_512
        // `emit_evex_shift_512`: (a, count).
        | Psllwi512 | Psrlwi512 | Psrawi512 | Psllidi512 | Psrlidi512 | Psradi512 | Psllqi512
        | Psrlqi512 | Psraqi512
        // `emit_evex_imm_512`: (a, immediate).
        | Pshufd512 | Pshuflw512 | Pshufhw512
        // The two ends of a copy: 64 bytes from `args[0]` to `dest_ptr`,
        // each a home or an address.
        | Loadu512 | Storeu512 => row(true, 0b001),
        // `emit_evex_ternary_512`: (a, b, c, immediate), computed in `a`.
        TernaryLogic512
        // `emit_evex_vpdpbusd_512`: (accumulator, a, b).
        | Vpdpbusd512 | Vpdpbusds512
        // `emit_evex_fma_512`: (a, b, c). The digits of the form say which
        // of the three the instruction computes in (`fma_role_order`).
        | FmaPs132v512 | FmaPd132v512 | FmsubPs132v512 | FnmaddPs132v512 | FnmsubPs132v512
        | FmaddsubPs132v512 | FmsubaddPs132v512 | FmsubPd132v512 | FnmaddPd132v512
        | FnmsubPd132v512 | FmaddsubPd132v512 | FmsubaddPd132v512 => tied(0),
        FmaPs213v512 | FmaPd213v512 => tied(1),
        FmaPs231v512 | FmaPd231v512 => tied(2),
        // `emit_evex_gpr_broadcast_512` and the zero: no vector source.
        SetEpi8_512 | SetEpi16_512 | SetEpi32_512 | SetEpi64x512 | Setzero512 => row(true, 0),
        // The mask compares of `emit_simd_scalar`: (a, b, predicate), and
        // the result is a mask in a general register.
        CmpeqEpu8Mask512 | CmpEpi8Mask512 | CmpeqEpu16Mask512 | CmpEpi16Mask512
        | CmpeqEpu32Mask512 | CmpEpi32Mask512 | CmpeqEpu64Mask512 | CmpEpi64Mask512 => {
            row(false, 0b011)
        }
        _ => None,
    }
}

/// Every operation [`signature`] has a row for, for the test that holds
/// each row to its emitter.
#[cfg(test)]
pub(crate) fn signature_ops() -> Vec<IntrinsicOp> {
    use IntrinsicOp::*;
    let ops = vec![
        Paddb512,
        Paddw512,
        Paddd512,
        Paddq512,
        Psubb512,
        Psubw512,
        Psubd512,
        Psubq512,
        Paddsb512,
        Paddsw512,
        Paddusb512,
        Paddusw512,
        Psubsb512,
        Psubsw512,
        Psubusb512,
        Psubusw512,
        Pavgb512,
        Pavgw512,
        Pmaxub512,
        Pminub512,
        Pmaxuw512,
        Pminuw512,
        Pmaxsb512,
        Pminsb512,
        Pmaxsw512,
        Pminsw512,
        Pmaxsd512,
        Pminsd512,
        Pmaxud512,
        Pminud512,
        Pmaxsq512,
        Pminsq512,
        Pmaxuq512,
        Pminuq512,
        Psadbw512,
        Pmaddubsw512,
        Pmaddwd512,
        Pmullw512,
        Pmulhw512,
        Pmulhuw512,
        Pmulld512,
        Pmuludq512,
        Pxor512,
        Por512,
        Pand512,
        Pandn512,
        Pshufb512,
        Punpcklbw512,
        Punpcklwd512,
        Punpckldq512,
        Punpcklqdq512,
        Punpckhbw512,
        Punpckhwd512,
        Punpckhdq512,
        Punpckhqdq512,
        Packsswb512,
        Packuswb512,
        Packssdw512,
        Packusdw512,
        PermutexvarEp32,
        PermutexvarEp64,
        AddPs512,
        SubPs512,
        MulPs512,
        DivPs512,
        MinPs512,
        MaxPs512,
        AddPd512,
        SubPd512,
        MulPd512,
        DivPd512,
        MinPd512,
        MaxPd512,
        Palignr512,
        Vpclmulqdq512,
        Pabsb512,
        Pabsw512,
        Pabsd512,
        Pabsq512,
        Popcntb512,
        Popcntw512,
        Popcntd512,
        Popcntq512,
        SqrtPs512,
        SqrtPd512,
        CvtEp32_2Ps512,
        CvtPs2Ep32_512,
        CvttPs2Ep32_512,
        Psllwi512,
        Psrlwi512,
        Psrawi512,
        Psllidi512,
        Psrlidi512,
        Psradi512,
        Psllqi512,
        Psrlqi512,
        Psraqi512,
        Pshufd512,
        Pshuflw512,
        Pshufhw512,
        Loadu512,
        Storeu512,
        TernaryLogic512,
        FmaPs132v512,
        FmaPs213v512,
        FmaPs231v512,
        FmaPd132v512,
        FmaPd213v512,
        FmaPd231v512,
        FmsubPs132v512,
        FnmaddPs132v512,
        FnmsubPs132v512,
        FmaddsubPs132v512,
        FmsubaddPs132v512,
        FmsubPd132v512,
        FnmaddPd132v512,
        FnmsubPd132v512,
        FmaddsubPd132v512,
        FmsubaddPd132v512,
        Vpdpbusd512,
        Vpdpbusds512,
        SetEpi8_512,
        SetEpi16_512,
        SetEpi32_512,
        SetEpi64x512,
        Setzero512,
        CmpeqEpu8Mask512,
        CmpEpi8Mask512,
        CmpeqEpu16Mask512,
        CmpEpi16Mask512,
        CmpeqEpu32Mask512,
        CmpEpi32Mask512,
        CmpeqEpu64Mask512,
        CmpEpi64Mask512,
    ];
    debug_assert!(ops.iter().all(|op| signature(*op).is_some()));
    ops
}

/// A set of candidate indices.
#[derive(Clone, PartialEq, Eq)]
struct Bits(Vec<u64>);

impl Bits {
    fn new(len: usize) -> Self {
        Bits(vec![0; len.div_ceil(64)])
    }

    fn insert(&mut self, at: usize) {
        self.0[at / 64] |= 1 << (at % 64);
    }

    fn remove(&mut self, at: usize) {
        self.0[at / 64] &= !(1 << (at % 64));
    }

    fn contains(&self, at: usize) -> bool {
        self.0[at / 64] & (1 << (at % 64)) != 0
    }

    /// `self &= !other`.
    fn subtract(&mut self, other: &Bits) {
        for (mine, theirs) in self.0.iter_mut().zip(&other.0) {
            *mine &= !theirs;
        }
    }

    /// `self |= other`; whether anything was added.
    fn union(&mut self, other: &Bits) -> bool {
        let mut grew = false;
        for (mine, theirs) in self.0.iter_mut().zip(&other.0) {
            let merged = *mine | *theirs;
            grew |= merged != *mine;
            *mine = merged;
        }
        grew
    }

    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(word, &bits)| {
            let mut rest = bits;
            std::iter::from_fn(move || {
                if rest == 0 {
                    return None;
                }
                let bit = rest.trailing_zeros() as usize;
                rest &= rest - 1;
                Some(word * 64 + bit)
            })
        })
    }
}

/// The positions of an instruction that hold a 512-bit object read or
/// written whole.
#[derive(Clone, Copy, Default)]
struct Whole {
    write: Option<Value>,
    reads: [Option<Value>; 3],
    /// The write is a copy of `reads[0]`.
    is_copy: bool,
    /// The read the instruction's own register has to hold first
    /// ([`Signature::tied`]).
    tied: Option<u8>,
}

impl Whole {
    /// The positions of `inst` this module understands: the two ends of a
    /// 64-byte `Memcpy`, and the operands [`signature`] has a row for.
    /// Everything that asks what an instruction does to an object asks
    /// here, so the candidates, their liveness and the check before code
    /// generation cannot disagree.
    fn of(inst: &Instruction) -> Option<Whole> {
        match inst {
            Instruction::Memcpy { dest, src, size } if *size == BYTES && dest != src => {
                Some(Whole {
                    write: Some(*dest),
                    reads: [Some(*src), None, None],
                    is_copy: true,
                    tied: None,
                })
            }
            Instruction::Intrinsic {
                op, dest_ptr, args, ..
            } => {
                let row = signature(*op)?;
                // The form the emitters of the table take: a result goes
                // through `dest_ptr` exactly when the row writes one.
                if row.writes_dest != dest_ptr.is_some() {
                    return None;
                }
                let mut reads = [None; 3];
                // The table has at most three vector arguments, the first
                // three (`the_table_and_its_list_are_the_same_set`).
                for (at, arg) in args.iter().enumerate().take(3) {
                    if let (1, Operand::Value(value)) = (row.vector_args >> at & 1, arg) {
                        reads[at] = Some(*value);
                    }
                }
                Some(Whole {
                    write: *dest_ptr,
                    reads,
                    is_copy: matches!(op, IntrinsicOp::Loadu512 | IntrinsicOp::Storeu512),
                    tied: row.tied,
                })
            }
            _ => None,
        }
    }

    /// How many of the positions `value` is in.
    fn mentions(&self, value: u32) -> u32 {
        self.reads
            .iter()
            .chain(std::iter::once(&self.write))
            .filter(|position| position.is_some_and(|at| at.0 == value))
            .count() as u32
    }
}

/// What one instruction does to the candidates, by their dense indices.
struct Step {
    reads: [Option<usize>; 3],
    write: Option<usize>,
    /// The write is a copy of `reads[0]`.
    is_copy: bool,
    /// The read whose register the instruction computes in (`Signature::tied`).
    tied: Option<usize>,
    /// The instruction can write `zmm16`..`zmm31` ([`runs_other_code`]).
    runs_other_code: bool,
}

impl Step {
    fn reads(&self) -> impl Iterator<Item = usize> + '_ {
        self.reads.iter().flatten().copied()
    }
}

/// The dense indices of the candidates, by the value of the `Alloca`.
struct Candidates {
    index: FxHashMap<u32, usize>,
    values: Vec<u32>,
}

impl Candidates {
    fn of(&self, value: Option<Value>) -> Option<usize> {
        self.index.get(&value?.0).copied()
    }

    /// [`Whole::of`] in candidates, or `None` for an instruction that is
    /// nothing to them. An end that is no candidate (the address of other
    /// memory, an object that stays in memory) is nothing either.
    fn step(&self, inst: &Instruction) -> Option<Step> {
        let runs_other_code = runs_other_code(inst);
        let whole = Whole::of(inst).unwrap_or_default();
        let reads = whole.reads.map(|value| self.of(value));
        let write = self.of(whole.write);
        if !runs_other_code && write.is_none() && reads.iter().all(Option::is_none) {
            return None;
        }
        Some(Step {
            reads,
            write,
            is_copy: whole.is_copy && write.is_some() && reads[0].is_some(),
            tied: whole.tied.and_then(|at| reads[usize::from(at)]),
            runs_other_code,
        })
    }
}

/// Whether an `asm` statement leaves `zmm16`..`zmm31` alone for certain: its
/// template assembles to nothing, and every operand is of a class that is
/// staged in a general register, in memory or as text.
///
/// Anything else is taken to write them. A template is not read here, and
/// a register variable (`{xmm20}`) or a vector class (`x`, `v`, `Y`) may be
/// staged in one of them. (A 256- or 512-bit operand is loaded and stored by
/// instructions the lowering adds to the template,
/// `ir::lowering::x86_asm_registers`, so its template is never empty.)
fn asm_leaves_the_homes_alone(
    template: &str,
    outputs: &[(String, Value, Option<String>)],
    inputs: &[(String, Operand, Option<String>)],
    goto_labels: &[(String, BlockId)],
) -> bool {
    // The modifiers, the general-register classes, memory, the immediates,
    // a tie to another operand, and the comma between alternatives.
    const HARMLESS: &str = "=+&%*?!#rqQabcdSDRlgmoVinsNIJKLMOZe,0123456789";
    goto_labels.is_empty()
        && crate::backend::inline_asm::inline_asm_template_has_no_instructions(template)
        && outputs
            .iter()
            .map(|(constraint, ..)| constraint)
            .chain(inputs.iter().map(|(constraint, ..)| constraint))
            .all(|constraint| constraint.chars().all(|class| HARMLESS.contains(class)))
}

/// Whether `inst` can run code this function does not see, or code of its
/// own that writes `zmm16`..`zmm31`. A value that is live across such an
/// instruction stays in memory: nobody preserves the registers.
///
/// What is left writes none of them. The emitters of this back end name no
/// register above 15 apart from the homes (`tests/regression/
/// check_x86_zmm_homes.py` compiles the corpus without this module and
/// looks), and the two kinds this function clears by name emit no call
/// (`the_inline_lowerings_emit_no_call`):
///
/// * a `Memcpy` is a call point to the general registers it stages its
///   addresses in, and is expanded in line (`emit_memcpy_impl_impl`:
///   `rep movs`, or moves through `%xmm0`..`%zmm1`);
/// * an `Intrinsic` is one or a few instructions. The three that are calls
///   or worse leave the whole function alone ([`defeats_the_analysis`]).
fn runs_other_code(inst: &Instruction) -> bool {
    match inst {
        Instruction::Memcpy { .. } | Instruction::Intrinsic { .. } => false,
        Instruction::InlineAsm {
            template,
            outputs,
            inputs,
            goto_labels,
            ..
        } => !asm_leaves_the_homes_alone(template, outputs, inputs, goto_labels),
        // The rest is what the allocator of the general registers has to
        // know as well: a call, a TLS address that is one, a library helper
        // (128-bit division, `long double` arithmetic, a 16-byte atomic),
        // the `va_*` family, a trampoline.
        _ => instruction_is_call_point(inst),
    }
}

/// Whether a function with `inst` in it is left alone altogether: control
/// that arrives where no edge of the CFG leads (the block summaries below
/// are per block, so an `asm goto`, which leaves from the middle of one, is
/// in this list too), or a call the IR does not show as one.
fn defeats_the_analysis(inst: &Instruction) -> bool {
    match inst {
        Instruction::InlineAsm { goto_labels, .. } => !goto_labels.is_empty(),
        Instruction::NonlocalGotoSave { .. } | Instruction::NonlocalGoto { .. } => true,
        Instruction::Intrinsic { op, .. } => matches!(
            op,
            IntrinsicOp::BuiltinSetjmp | IntrinsicOp::BuiltinLongjmp | IntrinsicOp::DoBuiltinApply
        ),
        _ => crate::backend::liveness::is_returns_twice_call(inst),
    }
}

/// The most objects of one function that are looked at. The interference
/// of `n` candidates is `n * n` bits (2 MiB here); the objects behind the
/// first `MAX_CANDIDATES` in the order of their values stay in memory.
const MAX_CANDIDATES: usize = 4096;

/// The 64-byte objects of `func` that are only ever read and written
/// whole, or `None` when there is none or the function is left alone.
fn candidates(func: &IrFunction) -> Option<Candidates> {
    // A naked function is the text of its `asm` statements.
    if func.is_naked {
        return None;
    }
    let mut mentioned: FxHashMap<u32, (u32, u32)> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                size: BYTES,
                volatile: false,
                semantic_volatile: false,
                ..
            } = inst
            {
                mentioned.insert(dest.0, (0, 0));
            }
        }
    }
    // A parameter's home is written by the prologue, which the IR does not
    // show; the stack guard is not an object of the program.
    for param in &func.param_alloca_values {
        mentioned.remove(&param.0);
    }
    if let Some(guard) = func.ssp.stack_guard_alloca {
        mentioned.remove(&guard.0);
    }
    if mentioned.is_empty() {
        return None;
    }

    // Every mention of a value against the mentions that are understood.
    // The two are counted the same way (`for_each_used_value` visits
    // `dest_ptr` and each argument once), so any other use leaves a
    // difference behind: a lane, a part, an address that goes somewhere.
    let mut has_512_bit_operation = false;
    for block in &func.blocks {
        for inst in &block.instructions {
            if defeats_the_analysis(inst) {
                return None;
            }
            inst.for_each_used_value(|value| {
                if let Some(counts) = mentioned.get_mut(&value) {
                    counts.0 += 1;
                }
            });
            if let Some(whole) = Whole::of(inst) {
                has_512_bit_operation |= matches!(inst, Instruction::Intrinsic { .. });
                for value in whole
                    .reads
                    .iter()
                    .chain(std::iter::once(&whole.write))
                    .flatten()
                {
                    if let Some(counts) = mentioned.get_mut(&value.0) {
                        counts.1 += 1;
                    }
                }
            }
        }
        block.terminator.for_each_used_value(|value| {
            if let Some(counts) = mentioned.get_mut(&value) {
                counts.0 += 1;
            }
        });
    }

    // The registers are AVX-512F. A function with an operation of the
    // table has it: the front end refuses a 512-bit intrinsic where the
    // function's ISA does not allow one, the inliner moves none to where
    // it does not, and the operation is an EVEX instruction on a `zmm`
    // register with or without this module. A function that only copies
    // 64-byte objects says nothing of the kind, and gets no home.
    if !has_512_bit_operation {
        return None;
    }
    let mut values: Vec<u32> = mentioned
        .iter()
        .filter(|(_, (every, understood))| every == understood && *every != 0)
        .map(|(value, _)| *value)
        .collect();
    if values.is_empty() {
        return None;
    }
    // The order of a hash map is not the order of anything: the indices,
    // and with them the registers, follow the value numbers.
    values.sort_unstable();
    values.truncate(MAX_CANDIDATES);
    let index = values
        .iter()
        .enumerate()
        .map(|(at, value)| (*value, at))
        .collect();
    Some(Candidates { index, values })
}

/// The steps of every block, and the edges between the blocks.
struct Flow {
    steps: Vec<Vec<Step>>,
    successors: FlatAdj,
}

impl Flow {
    fn of(func: &IrFunction, candidates: &Candidates) -> Flow {
        let labels = build_label_map(func);
        let (_, successors) = build_cfg(func, &labels);
        let steps = func
            .blocks
            .iter()
            .map(|block| {
                block
                    .instructions
                    .iter()
                    .filter_map(|inst| candidates.step(inst))
                    .collect()
            })
            .collect();
        Flow { steps, successors }
    }
}

/// Which candidates may not share a register, for one way of putting the
/// candidates into classes that do.
struct Graph {
    /// Row `i`: the classes that hold a value while `i` is written, and
    /// the classes `i` should not be computed in. Symmetric.
    interferes: Vec<Bits>,
    /// The classes that hold a value across code that may write the
    /// registers.
    in_memory: Bits,
    /// The copies between two classes, as `(written, read)`.
    copies: Vec<(usize, usize)>,
    /// The classes whose register each class would like to have.
    prefers: Vec<Vec<usize>>,
    /// Where each class is first written, in the order of the blocks. A
    /// class that is only read comes last.
    first_write: Vec<(usize, usize)>,
}

impl Graph {
    /// `class[i]` is the class of candidate `i`: a candidate that stands
    /// for itself and for every candidate that was merged into it.
    fn of(flow: &Flow, class: &[usize]) -> Graph {
        let count = class.len();
        // A copy inside one class moves nothing.
        let moves = |step: &Step| {
            !(step.is_copy && step.write.map(|at| class[at]) == step.reads[0].map(|at| class[at]))
        };

        // Per block: what it reads before it writes it, and what it writes.
        let mut live_in = Vec::with_capacity(flow.steps.len());
        let mut writes = Vec::with_capacity(flow.steps.len());
        for steps in &flow.steps {
            let mut first = Bits::new(count);
            let mut written = Bits::new(count);
            for step in steps.iter().filter(|step| moves(step)) {
                for source in step.reads() {
                    if !written.contains(class[source]) {
                        first.insert(class[source]);
                    }
                }
                if let Some(target) = step.write {
                    written.insert(class[target]);
                }
            }
            live_in.push(first);
            writes.push(written);
        }
        let mut live_out: Vec<Bits> = vec![Bits::new(count); flow.steps.len()];
        let mut changed = true;
        while changed {
            changed = false;
            for at in (0..flow.steps.len()).rev() {
                for &next in flow.successors.row(at) {
                    // `live_in[next]` is read while `live_out[at]` is
                    // written; a block that is its own successor needs the
                    // copy.
                    let incoming = live_in[next as usize].clone();
                    changed |= live_out[at].union(&incoming);
                }
                let mut through = live_out[at].clone();
                through.subtract(&writes[at]);
                changed |= live_in[at].union(&through);
            }
        }

        let mut graph = Graph {
            interferes: vec![Bits::new(count); count],
            in_memory: Bits::new(count),
            copies: Vec::new(),
            prefers: vec![Vec::new(); count],
            first_write: vec![(usize::MAX, usize::MAX); count],
        };
        for (block_at, steps) in flow.steps.iter().enumerate() {
            let mut live = live_out[block_at].clone();
            for (step_at, step) in steps.iter().enumerate().rev() {
                if !moves(step) {
                    continue;
                }
                if step.runs_other_code {
                    // What is live behind it is live across it. No
                    // instruction of this kind has an operand position that
                    // is understood, so it touches no candidate itself; if
                    // one ever does, the candidate is not in a register
                    // while it runs either.
                    graph.in_memory.union(&live);
                    for touched in step.reads().chain(step.write) {
                        graph.in_memory.insert(class[touched]);
                    }
                }
                if let Some(target) = step.write.map(|at| class[at]) {
                    graph.first_write[target] = graph.first_write[target].min((block_at, step_at));
                    // What this write overwrites: everything that holds a
                    // value behind it. A source that dies here is not among
                    // them (the emitters read every source before they
                    // write), and neither is the source of a copy, which
                    // holds the value that is written. The exception is
                    // this write's alone: another write of the same class,
                    // while the source of this copy lives, does interfere.
                    let mut overwritten = live.clone();
                    overwritten.remove(target);
                    if step.is_copy {
                        if let Some(source) = step.reads[0].map(|at| class[at]) {
                            overwritten.remove(source);
                            graph.copies.push((target, source));
                            graph.prefers[target].push(source);
                            graph.prefers[source].push(target);
                        }
                    }
                    if let Some(tied) = step.tied.map(|at| class[at]) {
                        // The instruction computes in the register of
                        // `tied`. In that register it is one instruction;
                        // in the register of another source it is three,
                        // through a scratch register, because bringing
                        // `tied` there would overwrite the source. So the
                        // result would like the first and stays out of the
                        // second, also where the source dies.
                        if tied != target {
                            graph.prefers[target].push(tied);
                            graph.prefers[tied].push(target);
                            for other in step.reads().map(|at| class[at]) {
                                if other != tied && other != target {
                                    overwritten.insert(other);
                                }
                            }
                        }
                    }
                    graph.interferes[target].union(&overwritten);
                    live.remove(target);
                }
                for source in step.reads() {
                    live.insert(class[source]);
                }
            }
        }
        for at in 0..count {
            let row = graph.interferes[at].clone();
            for other in row.iter() {
                graph.interferes[other].insert(at);
            }
        }
        graph
    }

    /// Put the two ends of every copy that may share a register into one
    /// class. Whether any were.
    ///
    /// Two names of one value do not interfere at the copy between them,
    /// but a third name does: in `b = a; c = b`, with `a` read later, `c`
    /// is written while `a` holds a value. Once `a` and `b` are one class
    /// the copy is from that class and `c` can join it, which is why the
    /// graph is built again after a round of this (Chaitin's coalescing).
    fn merge_copies(&mut self, class: &mut [usize]) -> bool {
        let mut merged = false;
        for at in 0..self.copies.len() {
            let (target, source) = self.copies[at];
            let (keep, join) = (class[target], class[source]);
            if keep == join
                || self.in_memory.contains(keep)
                || self.in_memory.contains(join)
                || self.interferes[keep].contains(join)
            {
                continue;
            }
            // The class has every interference either of the two had.
            let row = self.interferes[join].clone();
            for other in row.iter() {
                self.interferes[other].insert(keep);
            }
            self.interferes[keep].union(&row);
            for member in class.iter_mut().filter(|member| **member == join) {
                *member = keep;
            }
            merged = true;
        }
        merged
    }
}

/// How often the classes are built again after copies were merged. A chain
/// of `n` copies of one value needs about `n` rounds to become one class;
/// the chains are as long as inlining is deep.
const MERGE_ROUNDS: usize = 8;

/// The register of each candidate that gets one, as `(value, register)`
/// sorted by value.
fn colour(func: &IrFunction, candidates: &Candidates) -> Vec<(u32, u8)> {
    let count = candidates.values.len();
    let flow = Flow::of(func, candidates);
    let mut class: Vec<usize> = (0..count).collect();
    let mut rounds = 0;
    let graph = loop {
        let mut graph = Graph::of(&flow, &class);
        if rounds == MERGE_ROUNDS || !graph.merge_copies(&mut class) {
            break graph;
        }
        rounds += 1;
    };

    let mut order: Vec<usize> = (0..count)
        .filter(|&at| class[at] == at && !graph.in_memory.contains(at))
        .collect();
    order.sort_unstable_by_key(|&at| (graph.first_write[at], at));

    let mut register: Vec<Option<u8>> = vec![None; count];
    for at in order {
        let mut taken = 0u32;
        for other in graph.interferes[at].iter() {
            if let Some(index) = register[other] {
                taken |= 1 << index;
            }
        }
        let preferred = graph.prefers[at]
            .iter()
            .filter_map(|&other| register[class[other]])
            .find(|&index| taken & (1 << index) == 0);
        let free = (0..HOME_COUNT as u8).find(|&index| taken & (1 << index) == 0);
        register[at] = preferred.or(free);
    }

    (0..count)
        .filter_map(|at| Some((candidates.values[at], FIRST_HOME + register[class[at]]?)))
        .collect()
}

/// Give the 512-bit objects of `func` their registers and turn the copies
/// that touch one into the instruction that copies a register. Returns the
/// number of homes.
fn assign_in_function(func: &mut IrFunction) -> usize {
    func.zmm_homes.clear();
    let Some(candidates) = candidates(func) else {
        return 0;
    };
    let homes = colour(func, &candidates);
    if homes.is_empty() {
        return 0;
    }
    let homed = |value: Value| {
        homes
            .binary_search_by_key(&value.0, |(value, _)| *value)
            .is_ok()
    };

    // A 64-byte `Memcpy` with a home at either end becomes `Storeu512`,
    // which is that copy in the form `_mm512_storeu_si512` has: 64 bytes
    // from where `args[0]` points to where `dest_ptr` points. Its emitter
    // knows the homes, it is no call point, and between two objects that
    // share a register it emits nothing. The copy stays in the IR, which
    // goes on saying what the program does without knowing who shares a
    // register with whom.
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            if let Instruction::Memcpy { dest, src, size } = inst {
                if *size == BYTES && dest != src && (homed(*dest) || homed(*src)) {
                    *inst = Instruction::Intrinsic {
                        dest: None,
                        op: IntrinsicOp::Storeu512,
                        dest_ptr: Some(*dest),
                        args: vec![Operand::Value(*src)],
                    };
                }
            }
        }
    }

    let count = homes.len();
    func.zmm_homes = homes;
    count
}

/// Whether every mention of every homed object of `func` is still an
/// operand position its home covers.
///
/// The homes are decided on IR that the range splitters, the cleanup
/// behind them, phi elimination and the block layout edit afterwards. None
/// of them adds a use of a 64-byte vector object (the splitters leave
/// every `Alloca` alone). The code generator asks all the same, once per
/// function, and gives a function whose homes do not hold none: its
/// objects get their stack slots, and what was a `Memcpy` is the same copy
/// through memory.
pub(crate) fn homes_hold(func: &IrFunction) -> bool {
    if func.zmm_homes.is_empty() {
        return true;
    }
    let homed = |value: u32| func.zmm_home(value).is_some();
    for block in &func.blocks {
        for inst in &block.instructions {
            let mut mentions = 0u32;
            inst.for_each_used_value(|value| mentions += u32::from(homed(value)));
            if mentions == 0 {
                continue;
            }
            let Some(whole) = Whole::of(inst) else {
                return false;
            };
            let covered: u32 = func
                .zmm_homes
                .iter()
                .map(|(value, _)| whole.mentions(*value))
                .sum();
            // A `Memcpy` is covered as an analysis and not as an emitter:
            // every copy that touches a home was rewritten.
            if covered != mentions || matches!(inst, Instruction::Memcpy { .. }) {
                return false;
            }
        }
        let mut in_terminator = false;
        block
            .terminator
            .for_each_used_value(|value| in_terminator |= homed(value));
        if in_terminator {
            return false;
        }
    }
    true
}

/// Run over every function of `module`. Returns the number of homes.
pub(crate) fn assign(module: &mut IrModule) -> usize {
    module.for_each_function(|func| {
        if func.blocks.is_empty() {
            0
        } else {
            assign_in_function(func)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::source::Span;
    use crate::common::types::IrType;
    use crate::ir::reexports::{BasicBlock, CallInfo, IrConst, Terminator};

    const A: u32 = 1;
    const B: u32 = 2;
    const C: u32 = 3;
    const D: u32 = 4;
    /// A pointer that is no object of the function.
    const P: u32 = 90;

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            source_spans: vec![Span::dummy(); instructions.len()],
            source_inline_ids: Vec::new(),
            instructions,
            terminator,
        }
    }

    fn function(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut function = IrFunction::new("f".into(), IrType::Void, Vec::new(), false);
        function.blocks = blocks;
        function.next_value_id = 100;
        function
    }

    fn object(dest: u32) -> Instruction {
        sized(dest, 64)
    }

    fn sized(dest: u32, size: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::Ptr,
            size,
            align: 64,
            volatile: false,
            semantic_volatile: false,
        }
    }

    fn copy(dest: u32, src: u32) -> Instruction {
        Instruction::Memcpy {
            dest: Value(dest),
            src: Value(src),
            size: 64,
        }
    }

    fn op(op: IntrinsicOp, dest: u32, args: &[u32]) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(dest + 50)),
            op,
            dest_ptr: Some(Value(dest)),
            args: args.iter().map(|v| Operand::Value(Value(*v))).collect(),
        }
    }

    fn add(dest: u32, a: u32, b: u32) -> Instruction {
        op(IntrinsicOp::Paddd512, dest, &[a, b])
    }

    fn load(dest: u32, from: u32) -> Instruction {
        op(IntrinsicOp::Loadu512, dest, &[from])
    }

    fn store(to: u32, value: u32) -> Instruction {
        Instruction::Intrinsic {
            dest: None,
            op: IntrinsicOp::Storeu512,
            dest_ptr: Some(Value(to)),
            args: vec![Operand::Value(Value(value))],
        }
    }

    fn call() -> Instruction {
        Instruction::Call {
            func: "g".into(),
            info: CallInfo::default(),
        }
    }

    fn homes(function: &mut IrFunction) -> Vec<(u32, u8)> {
        assign_in_function(function);
        function.zmm_homes.clone()
    }

    fn home_of(function: &IrFunction, value: u32) -> Option<u8> {
        function
            .zmm_homes
            .iter()
            .find(|(v, _)| *v == value)
            .map(|(_, r)| *r)
    }

    #[test]
    fn the_table_and_its_list_are_the_same_set() {
        let ops = signature_ops();
        let unique: std::collections::HashSet<_> = ops.iter().copied().collect();
        assert_eq!(unique.len(), ops.len(), "an operation is listed twice");
        for op in &ops {
            let row = signature(*op).expect("a listed operation has a row");
            assert!(
                row.vector_args < 8,
                "{op:?}: three vector arguments at most"
            );
            if let Some(tied) = row.tied {
                assert!(
                    row.writes_dest && row.vector_args >> tied & 1 == 1,
                    "{op:?}"
                );
            }
        }
        // Operations of other widths and the ones whose emitters read a
        // part of the object have no row.
        for op in [
            IntrinsicOp::Paddd256,
            IntrinsicOp::Paddd128,
            IntrinsicOp::Cast512to256,
            IntrinsicOp::ExtractI64x4,
            IntrinsicOp::InsertI64x4,
            IntrinsicOp::Pmovzxbw512,
            IntrinsicOp::MaskStoreuEpi8_512,
        ] {
            assert_eq!(signature(op), None, "{op:?}");
        }
    }

    #[test]
    fn a_chain_of_whole_objects_lives_in_registers() {
        // a = *p; b = *p; t = a; u = b; c = t + u; *p = c
        let mut f = function(vec![block(
            0,
            vec![
                object(A),
                object(B),
                object(C),
                object(D),
                object(5),
                load(A, P),
                load(B, P),
                copy(C, A),
                copy(D, B),
                add(5, C, D),
                store(P, 5),
            ],
            Terminator::Return(None),
        )]);
        let homes = homes(&mut f);
        assert_eq!(homes.len(), 5);
        // Each copy shares the register of its source: it is still there,
        // as the `Storeu512` that emits nothing between the two.
        assert_eq!(home_of(&f, C), home_of(&f, A));
        assert_eq!(home_of(&f, D), home_of(&f, B));
        assert_ne!(home_of(&f, A), home_of(&f, B));
        assert!(
            !f.blocks[0]
                .instructions
                .iter()
                .any(|inst| matches!(inst, Instruction::Memcpy { .. })),
            "{:?}",
            f.blocks[0].instructions
        );
        assert_eq!(f.blocks[0].instructions.len(), 11);
        assert!(matches!(
            &f.blocks[0].instructions[7],
            Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::Storeu512,
                dest_ptr: Some(Value(C)),
                args,
            } if args == &[Operand::Value(Value(A))]
        ));
        // The sum is written when both of its sources are dead: it takes
        // the first register again.
        assert_eq!(home_of(&f, 5), Some(FIRST_HOME));
        assert_eq!(
            f.blocks[0].source_spans.len(),
            f.blocks[0].instructions.len()
        );
        assert!(homes_hold(&f));
    }

    #[test]
    fn a_copy_whose_source_changes_keeps_its_own_register() {
        // t = a; a = a + a; c = t + a: `t` holds the old `a`.
        let mut f = function(vec![block(
            0,
            vec![
                object(A),
                object(B),
                object(C),
                load(A, P),
                copy(B, A),
                add(A, A, A),
                add(C, B, A),
                store(P, C),
            ],
            Terminator::Return(None),
        )]);
        homes(&mut f);
        assert!(home_of(&f, A).is_some() && home_of(&f, B).is_some());
        assert_ne!(home_of(&f, A), home_of(&f, B));
        // The copy is still there, as a register move.
        assert!(matches!(
            &f.blocks[0].instructions[4],
            Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::Storeu512,
                dest_ptr: Some(Value(B)),
                args,
            } if args == &[Operand::Value(Value(A))]
        ));
    }

    #[test]
    fn the_names_of_one_value_are_one_register() {
        // b = a; c = b; d = c; r = d + a: `c` and `d` are written while `a`
        // holds a value, and it is their own.
        let mut f = function(vec![block(
            0,
            vec![
                object(A),
                object(B),
                object(C),
                object(D),
                object(5),
                load(A, P),
                copy(B, A),
                copy(C, B),
                copy(D, C),
                add(5, D, A),
                store(P, 5),
            ],
            Terminator::Return(None),
        )]);
        homes(&mut f);
        for value in [A, B, C, D, 5] {
            assert_eq!(home_of(&f, value), Some(FIRST_HOME), "%{value}");
        }
        assert!(homes_hold(&f));
    }

    #[test]
    fn a_name_that_crosses_a_call_does_not_take_its_copies_along() {
        // a = *p; g(); t = a; r = t + t; *p = r: `a` is memory, `t` is not.
        let mut f = function(vec![block(
            0,
            vec![
                object(A),
                object(B),
                object(C),
                load(A, P),
                call(),
                copy(B, A),
                add(C, B, B),
                store(P, C),
            ],
            Terminator::Return(None),
        )]);
        homes(&mut f);
        assert_eq!(home_of(&f, A), None);
        assert!(home_of(&f, B).is_some() && home_of(&f, C).is_some());
    }

    fn ternary(dest: u32, a: u32, b: u32, c: u32) -> Instruction {
        let mut inst = op(IntrinsicOp::TernaryLogic512, dest, &[a, b, c]);
        if let Instruction::Intrinsic { args, .. } = &mut inst {
            args.push(Operand::Const(IrConst::I64(0x96)));
        }
        inst
    }

    #[test]
    fn a_tied_result_takes_the_register_it_is_computed_in() {
        // r = f(a, b, c) with all three dead behind it: in place, in `a`.
        let build = |a_lives: bool| {
            let mut instructions = vec![
                object(A),
                object(B),
                object(C),
                object(D),
                load(A, P),
                load(B, P),
                load(C, P),
                ternary(D, A, B, C),
                store(P, D),
            ];
            if a_lives {
                instructions.push(store(P, A));
            }
            let mut f = function(vec![block(0, instructions, Terminator::Return(None))]);
            homes(&mut f);
            f
        };
        let f = build(false);
        assert_eq!(home_of(&f, D), home_of(&f, A));
        // With `a` read later the result needs a register of its own, and
        // not the one of `b` or `c`, although both are free: `a` has to be
        // brought there first, and that would overwrite them.
        let f = build(true);
        let result = home_of(&f, D);
        assert!(result.is_some());
        for source in [A, B, C] {
            assert_ne!(result, home_of(&f, source), "%{source}");
        }
    }

    #[test]
    fn a_copy_that_is_written_again_keeps_its_own_register() {
        // t = a; t = t + t; c = t + a: the second write of `t` happens
        // while `a` lives, whatever the first one was.
        let mut f = function(vec![block(
            0,
            vec![
                object(A),
                object(B),
                object(C),
                load(A, P),
                copy(B, A),
                add(B, B, B),
                add(C, B, A),
                store(P, C),
            ],
            Terminator::Return(None),
        )]);
        homes(&mut f);
        assert!(home_of(&f, A).is_some() && home_of(&f, B).is_some());
        assert_ne!(home_of(&f, A), home_of(&f, B));
        // The other order of the two writes: the copy comes second.
        let mut f = function(vec![block(
            0,
            vec![
                object(A),
                object(B),
                object(C),
                load(A, P),
                load(B, P),
                add(C, B, A),
                copy(B, A),
                add(C, C, B),
                store(P, C),
                store(P, A),
            ],
            Terminator::Return(None),
        )]);
        homes(&mut f);
        assert!(home_of(&f, A).is_some() && home_of(&f, B).is_some());
        assert_ne!(home_of(&f, A), home_of(&f, B));
    }

    #[test]
    fn any_other_mention_leaves_the_object_in_memory() {
        let escapes: Vec<(&str, Instruction)> = vec![
            (
                "a lane",
                Instruction::GetElementPtr {
                    dest: Value(60),
                    base: Value(A),
                    offset: Operand::Const(IrConst::I64(4)),
                    ty: IrType::I32,
                },
            ),
            (
                "a part",
                Instruction::Memcpy {
                    dest: Value(P),
                    src: Value(A),
                    size: 32,
                },
            ),
            (
                "an operation without a row",
                op(IntrinsicOp::Cast512to256, 61, &[A]),
            ),
            (
                "the count of a shift",
                op(IntrinsicOp::Psllidi512, B, &[B, A]),
            ),
            (
                "a copy onto itself",
                Instruction::Memcpy {
                    dest: Value(A),
                    src: Value(A),
                    size: 64,
                },
            ),
        ];
        for (what, escape) in escapes {
            let mut f = function(vec![block(
                0,
                vec![
                    object(A),
                    object(B),
                    load(A, P),
                    load(B, P),
                    escape,
                    store(P, B),
                ],
                Terminator::Return(None),
            )]);
            homes(&mut f);
            assert_eq!(home_of(&f, A), None, "{what}");
            assert!(home_of(&f, B).is_some(), "{what}");
        }
        // The operand of a terminator.
        let mut f = function(vec![block(
            0,
            vec![object(A), load(A, P)],
            Terminator::Return(Some(Operand::Value(Value(A)))),
        )]);
        assert_eq!(homes(&mut f), []);
    }

    #[test]
    fn other_sizes_volatile_objects_and_parameters_are_not_candidates() {
        let volatile = Instruction::Alloca {
            dest: Value(B),
            ty: IrType::Ptr,
            size: 64,
            align: 64,
            volatile: false,
            semantic_volatile: true,
        };
        let mut f = function(vec![block(
            0,
            vec![
                object(A),
                volatile,
                sized(C, 32),
                object(D),
                load(A, P),
                load(B, P),
                load(D, P),
                store(P, A),
                store(P, B),
                store(P, D),
            ],
            Terminator::Return(None),
        )]);
        f.param_alloca_values = vec![Value(D)];
        homes(&mut f);
        assert!(home_of(&f, A).is_some());
        assert_eq!(home_of(&f, B), None);
        assert_eq!(home_of(&f, C), None);
        assert_eq!(home_of(&f, D), None);
    }

    #[test]
    fn a_value_live_across_a_call_stays_in_memory() {
        // a = *p; b = *p; *p = b; g(); *p = a
        let mut f = function(vec![block(
            0,
            vec![
                object(A),
                object(B),
                load(A, P),
                load(B, P),
                store(P, B),
                call(),
                store(P, A),
            ],
            Terminator::Return(None),
        )]);
        homes(&mut f);
        assert_eq!(home_of(&f, A), None, "live across the call");
        assert!(home_of(&f, B).is_some(), "dead in front of it");
    }

    #[test]
    fn a_copy_to_or_from_memory_becomes_a_store() {
        // a = *p (memcpy); *q = a (memcpy); a = a + a
        let mut f = function(vec![block(
            0,
            vec![object(A), copy(A, P), copy(P, A), add(A, A, A)],
            Terminator::Return(None),
        )]);
        homes(&mut f);
        assert!(home_of(&f, A).is_some());
        let copies: Vec<_> = f.blocks[0].instructions[1..3]
            .iter()
            .map(|inst| match inst {
                Instruction::Intrinsic {
                    op: IntrinsicOp::Storeu512,
                    dest: None,
                    dest_ptr: Some(dest),
                    args,
                } => (dest.0, args.clone()),
                other => panic!("not a copy: {other:?}"),
            })
            .collect();
        assert_eq!(
            copies,
            [
                (A, vec![Operand::Value(Value(P))]),
                (P, vec![Operand::Value(Value(A))])
            ]
        );
    }

    #[test]
    fn liveness_follows_the_loop() {
        // B0: acc = *p            B1: t = *p; acc = acc + t; if .. B1 else B2
        // B2: *p = acc
        let mut f = function(vec![
            block(
                0,
                vec![object(A), object(B), load(A, P)],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![load(B, P), add(A, A, B)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(P)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(2, vec![store(P, A)], Terminator::Return(None)),
        ]);
        homes(&mut f);
        // The accumulator is live around the loop while `t` is written.
        assert!(home_of(&f, A).is_some() && home_of(&f, B).is_some());
        assert_ne!(home_of(&f, A), home_of(&f, B));
    }

    #[test]
    fn a_call_in_the_loop_takes_the_accumulator_out() {
        let mut f = function(vec![
            block(
                0,
                vec![object(A), object(B), load(A, P)],
                Terminator::Branch(BlockId(1)),
            ),
            block(
                1,
                vec![call(), load(B, P), add(A, A, B)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(P)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(2, vec![store(P, A)], Terminator::Return(None)),
        ]);
        homes(&mut f);
        assert_eq!(home_of(&f, A), None);
        assert!(home_of(&f, B).is_some());
    }

    #[test]
    fn seventeen_values_alive_at_once_leave_one_in_memory() {
        let values: Vec<u32> = (1..=17).collect();
        let mut instructions: Vec<Instruction> = values.iter().map(|v| object(*v)).collect();
        instructions.extend(values.iter().map(|v| load(*v, P)));
        instructions.extend(values.iter().map(|v| store(P, *v)));
        let mut f = function(vec![block(0, instructions, Terminator::Return(None))]);
        let homes = homes(&mut f);
        assert_eq!(homes.len(), 16);
        let mut registers: Vec<u8> = homes.iter().map(|(_, r)| *r).collect();
        registers.sort_unstable();
        assert_eq!(registers, (16..32).collect::<Vec<u8>>());
        assert_eq!(home_of(&f, 17), None);
    }

    #[test]
    fn the_registers_do_not_depend_on_hash_order() {
        let build = || {
            let mut f = function(vec![block(
                0,
                vec![
                    object(7),
                    object(3),
                    object(11),
                    load(7, P),
                    load(3, P),
                    add(11, 7, 3),
                    store(P, 11),
                    store(P, 7),
                ],
                Terminator::Return(None),
            )]);
            homes(&mut f)
        };
        let first = build();
        assert_eq!(first, [(3, 17), (7, 16), (11, 17)]);
        for _ in 0..8 {
            assert_eq!(build(), first);
        }
    }

    #[test]
    fn exotic_control_leaves_the_function_alone() {
        let goto = Instruction::InlineAsm {
            template: "jmp %l0".into(),
            outputs: Vec::new(),
            inputs: Vec::new(),
            clobbers: Vec::new(),
            operand_types: Vec::new(),
            goto_labels: vec![("out".into(), BlockId(1))],
            input_symbols: Vec::new(),
            seg_overrides: Vec::new(),
        };
        let mut f = function(vec![
            block(
                0,
                vec![object(A), load(A, P), goto, store(P, A)],
                Terminator::Branch(BlockId(1)),
            ),
            block(1, Vec::new(), Terminator::Return(None)),
        ]);
        assert_eq!(homes(&mut f), []);
    }

    #[test]
    fn bits_are_a_set() {
        let mut bits = Bits::new(130);
        for at in [0, 63, 64, 129] {
            bits.insert(at);
        }
        assert_eq!(bits.iter().collect::<Vec<_>>(), [0, 63, 64, 129]);
        bits.remove(63);
        assert!(!bits.contains(63) && bits.contains(64));
        let mut other = Bits::new(130);
        other.insert(5);
        assert!(other.union(&bits));
        assert!(!other.union(&bits));
        assert_eq!(other.iter().collect::<Vec<_>>(), [0, 5, 64, 129]);
        let mut gone = Bits::new(130);
        gone.insert(64);
        gone.insert(7);
        other.subtract(&gone);
        assert_eq!(other.iter().collect::<Vec<_>>(), [0, 5, 129]);
    }

    fn asm(template: &str, inputs: &[(&str, u32)]) -> Instruction {
        Instruction::InlineAsm {
            template: template.into(),
            outputs: Vec::new(),
            inputs: inputs
                .iter()
                .map(|(constraint, value)| {
                    ((*constraint).into(), Operand::Value(Value(*value)), None)
                })
                .collect(),
            clobbers: vec!["memory".into()],
            operand_types: vec![IrType::I64; inputs.len()],
            goto_labels: Vec::new(),
            input_symbols: vec![None; inputs.len()],
            seg_overrides: Vec::new(),
        }
    }

    #[test]
    fn an_asm_statement_keeps_the_homes_only_when_it_is_no_code() {
        // a = *p; <asm>; *p = a
        let across = |statement: Instruction| {
            let mut f = function(vec![block(
                0,
                vec![object(A), load(A, P), statement, store(P, A)],
                Terminator::Return(None),
            )]);
            homes(&mut f);
            home_of(&f, A).is_some()
        };
        // The barrier forms: nothing is assembled, and what is staged is
        // staged in a general register or not at all.
        assert!(across(asm("", &[])));
        assert!(across(asm("", &[("r,m", P)])));
        assert!(across(asm(" ;\n", &[("g", P), ("m", P)])));
        // A template is code, whatever it says.
        assert!(!across(asm("nop", &[])));
        assert!(!across(asm("# only a comment", &[])));
        // A vector class or a register variable may be one of the homes.
        assert!(!across(asm("", &[("x", P)])));
        assert!(!across(asm("", &[("v", P)])));
        assert!(!across(asm("", &[("{xmm20}", P)])));
    }

    #[test]
    fn the_object_an_asm_statement_names_stays_in_memory() {
        let mut f = function(vec![block(
            0,
            vec![
                object(A),
                object(B),
                load(A, P),
                load(B, P),
                asm("", &[("m", A)]),
                store(P, A),
                store(P, B),
            ],
            Terminator::Return(None),
        )]);
        homes(&mut f);
        assert_eq!(home_of(&f, A), None);
        assert!(home_of(&f, B).is_some());
    }

    #[test]
    fn values_that_meet_at_a_join_share_what_they_can() {
        // B0: a = *p; if .. B1 else B2
        // B1: b = a + a; goto B3        B2: b = *p; goto B3
        // B3: *p = b; *p = a
        let mut f = function(vec![
            block(
                0,
                vec![object(A), object(B), load(A, P)],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(P)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(1, vec![add(B, A, A)], Terminator::Branch(BlockId(3))),
            block(2, vec![load(B, P)], Terminator::Branch(BlockId(3))),
            block(3, vec![store(P, B), store(P, A)], Terminator::Return(None)),
        ]);
        homes(&mut f);
        // `a` is read behind the join, so `b` cannot have its register on
        // either arm.
        assert!(home_of(&f, A).is_some() && home_of(&f, B).is_some());
        assert_ne!(home_of(&f, A), home_of(&f, B));
        assert!(homes_hold(&f));
    }

    #[test]
    fn a_use_that_appears_later_is_caught_before_code_generation() {
        let mut f = function(vec![block(
            0,
            vec![object(A), load(A, P), store(P, A)],
            Terminator::Return(None),
        )]);
        homes(&mut f);
        assert!(home_of(&f, A).is_some());
        assert!(homes_hold(&f));
        // A later stage takes the address, copies a part, passes it on.
        for intruder in [
            Instruction::GetElementPtr {
                dest: Value(60),
                base: Value(A),
                offset: Operand::Const(IrConst::I64(4)),
                ty: IrType::I32,
            },
            Instruction::Memcpy {
                dest: Value(P),
                src: Value(A),
                size: 64,
            },
            op(IntrinsicOp::Cast512to256, 61, &[A]),
            op(IntrinsicOp::Psllidi512, 62, &[P, A]),
        ] {
            let mut edited = f.clone();
            edited.blocks[0].instructions.push(intruder);
            edited.blocks[0].source_spans.push(Span::dummy());
            assert!(!homes_hold(&edited));
        }
        let mut edited = f.clone();
        edited.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(A))));
        assert!(!homes_hold(&edited));
    }

    /// A 64-byte structure that is copied in and out looks like a vector to
    /// the rules above. Without a 512-bit operation nothing says that the
    /// function may execute AVX-512, so it gets no register.
    #[test]
    fn copies_alone_are_no_licence_for_the_registers() {
        let build = |with_vector_code: bool| {
            let mut instructions = vec![object(A), object(B), copy(A, P), copy(P, A)];
            if with_vector_code {
                instructions.extend([load(B, P), store(P, B)]);
            }
            let mut f = function(vec![block(0, instructions, Terminator::Return(None))]);
            homes(&mut f);
            f
        };
        let plain = build(false);
        assert_eq!(plain.zmm_homes, []);
        assert!(matches!(
            plain.blocks[0].instructions[2],
            Instruction::Memcpy { .. }
        ));
        let vector = build(true);
        assert!(home_of(&vector, A).is_some() && home_of(&vector, B).is_some());
    }

    #[test]
    fn a_naked_function_is_left_alone() {
        let mut f = function(vec![block(
            0,
            vec![object(A), load(A, P), store(P, A)],
            Terminator::Return(None),
        )]);
        f.is_naked = true;
        assert_eq!(homes(&mut f), []);
    }

    #[test]
    fn more_objects_than_the_limit_are_cut_not_refused() {
        let count = MAX_CANDIDATES as u32 + 40;
        let pointer = count + 10;
        let mut instructions: Vec<Instruction> = (1..=count).map(object).collect();
        for value in 1..=count {
            instructions.push(load(value, pointer));
            instructions.push(store(pointer, value));
        }
        let mut f = function(vec![block(0, instructions, Terminator::Return(None))]);
        f.next_value_id = 3 * count;
        let homes = homes(&mut f);
        // One value at a time is alive: every candidate that is looked at
        // gets the first register.
        assert_eq!(homes.len(), MAX_CANDIDATES);
        assert!(homes.iter().all(|(_, register)| *register == FIRST_HOME));
        assert_eq!(home_of(&f, count), None);
        assert!(homes_hold(&f));
    }

    /// [`runs_other_code`] takes a `Memcpy` and an `Intrinsic` for code of
    /// the function itself. Their emitters must not learn to call: the
    /// registers of the homes belong to whoever is called.
    #[test]
    fn the_inline_lowerings_emit_no_call() {
        let sources = [
            (
                "x86/codegen/intrinsics.rs",
                include_str!("x86/codegen/intrinsics.rs"),
            ),
            (
                "x86/codegen/intrinsics_simd.rs",
                include_str!("x86/codegen/intrinsics_simd.rs"),
            ),
            (
                "x86/codegen/memory.rs",
                include_str!("x86/codegen/memory.rs"),
            ),
        ];
        let mut calls = Vec::new();
        for (file, text) in sources {
            for (at, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or_default();
                if ["\"    call", "MachInst::Call", "CallTarget::"]
                    .iter()
                    .any(|needle| code.contains(needle))
                {
                    calls.push(format!("{file}:{}: {}", at + 1, line.trim()));
                }
            }
        }
        // `DoBuiltinApply`, which `defeats_the_analysis` names.
        assert_eq!(calls.len(), 1, "{calls:#?}");
        assert!(
            calls[0].starts_with("x86/codegen/intrinsics.rs:") && calls[0].contains("call *%r11"),
            "{calls:#?}"
        );
        assert!(defeats_the_analysis(&Instruction::Intrinsic {
            dest: None,
            op: IntrinsicOp::DoBuiltinApply,
            dest_ptr: None,
            args: Vec::new(),
        }));
    }
}
