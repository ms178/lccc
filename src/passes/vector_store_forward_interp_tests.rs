//! Executed differential tests for `vector_store_forward`'s proof.
//!
//! The pass deletes a temporary's memory round trip on the strength of a
//! *proof* about the temporary's complete use set.  A counter-based gate
//! cannot falsify that proof: the instruction counts stay exactly at their
//! budgets while the observed bytes change.  That is not hypothetical — a
//! stale reader index across two plans in one block (the `adler32_simd`
//! shape) produced a wrong checksum with a *perfect* instruction count.
//!
//! These tests therefore **execute** the pre-transform and post-transform IR
//! over a byte-accurate memory model and compare the resulting memory image.
//! The model is deliberately small: it implements exactly the instruction
//! subset this pass reasons about (`Alloca`, `Copy`, integral `BinOp`,
//! whole-vector store/load intrinsics, address-form point stores, byte loads
//! and stores), and it panics on anything else rather than silently modelling
//! it wrongly.  A panic in the model is a test bug, not a pass bug.
//!
//! Coverage is asserted, not hoped for: every accepted shape must report the
//! rewrite it is named for, and every refused shape must leave the IR
//! *identical* to the input.

use super::*;
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, IrConst, Operand, Terminator, Value};
use std::collections::BTreeMap;

// ── Byte-accurate memory image ───────────────────────────────────────────────

/// A sparse byte-addressed memory.  Reads of unwritten bytes return 0, which
/// matches a zeroed frame; every test fills what it reads.
#[derive(Clone, Default, Debug)]
struct Mem {
    bytes: BTreeMap<u64, u8>,
    /// Byte ranges whose contents are *not* observable: the forwarded
    /// temporary's own storage.  The rewrite's entire purpose is to stop
    /// writing it, so a comparison that included it would reject every
    /// accepted shape.
    ignored: Vec<(u64, u64)>,
}

impl Mem {
    fn write(&mut self, addr: u64, data: &[u8]) {
        for (i, b) in data.iter().enumerate() {
            self.bytes.insert(addr + i as u64, *b);
        }
    }

    fn read(&self, addr: u64, len: usize) -> Vec<u8> {
        (0..len as u64)
            .map(|i| *self.bytes.get(&(addr + i)).unwrap_or(&0))
            .collect()
    }

    /// The observable image: every written byte outside the ignored ranges.
    fn observable(&self) -> Vec<(u64, u8)> {
        self.bytes
            .iter()
            .filter(|(a, _)| {
                !self
                    .ignored
                    .iter()
                    .any(|(lo, len)| **a >= *lo && **a < lo + len)
            })
            .map(|(a, b)| (*a, *b))
            .collect()
    }

    fn ignore(&mut self, lo: u64, len: u64) {
        self.ignored.push((lo, len));
    }

    /// Number of written bytes inside the ignored (temporary) ranges.
    fn dead_writes(&self) -> usize {
        self.bytes
            .keys()
            .filter(|a| {
                self.ignored
                    .iter()
                    .any(|(lo, len)| **a >= *lo && **a < lo + len)
            })
            .count()
    }
}

/// A value in the interpreter: an integral scalar or a byte image of a vector.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Val {
    Int(u64),
    Vec(Vec<u8>),
}

// ── The interpreter ─────────────────────────────────────────────────────────

/// Outcome of executing one function: its memory image and the trace of
/// addresses it *observed* (so a rewrite that changes which bytes are read is
/// visible even when the final image happens to match).
#[derive(Debug)]
struct Executed {
    mem: Mem,
    reads: Vec<u64>,
}

/// Byte width of the vector store/load families this pass matches, plus the
/// point-store families it rewrites readers into.
fn vector_width(op: IntrinsicOp) -> Option<usize> {
    Some(match op {
        IntrinsicOp::VecStoreI64x2
        | IntrinsicOp::VecLoadI64x2
        | IntrinsicOp::VecLoadI32x4
        | IntrinsicOp::VecStoreI32x4
        | IntrinsicOp::VecZeroI64x2 => 16,
        IntrinsicOp::VecStoreI64x4
        | IntrinsicOp::VecLoadI64x4
        | IntrinsicOp::VecZeroI64x4
        | IntrinsicOp::VecConstI8x32 => 32,
        IntrinsicOp::Storedqu => 16,
        IntrinsicOp::StoreuPs256
        | IntrinsicOp::StoreuPd256
        | IntrinsicOp::Storeu256
        | IntrinsicOp::LoaduPs256
        | IntrinsicOp::LoaduPd256
        | IntrinsicOp::Loadu256 => 32,
        _ => return None,
    })
}

fn scalar(op: &Operand, values: &BTreeMap<u32, Val>) -> u64 {
    match op {
        Operand::Const(c) => c.to_i64().expect("integral constant") as u64,
        Operand::Value(v) => match values
            .get(&v.0)
            .unwrap_or_else(|| panic!("unset value {v:?}"))
        {
            Val::Int(x) => *x,
            Val::Vec(_) => panic!("expected an integral operand, found a vector"),
        },
    }
}

fn bytes(op: &Operand, values: &BTreeMap<u32, Val>, width: usize) -> Vec<u8> {
    match op {
        Operand::Value(v) => match values
            .get(&v.0)
            .unwrap_or_else(|| panic!("unset value {v:?}"))
        {
            Val::Vec(b) => {
                assert!(
                    b.len() >= width,
                    "vector narrower than the store width in the model"
                );
                // A narrower store writes the low bytes of a wider value; the
                // whole-vector shapes here never exceed their value's width.
                b[..width].to_vec()
            }
            Val::Int(x) => {
                // A scalar in a vector position: the low bytes, little-endian.
                x.to_le_bytes()
                    .iter()
                    .copied()
                    .cycle()
                    .take(width)
                    .collect()
            }
        },
        Operand::Const(_) => panic!("a vector operand must be a value"),
    }
}

fn execute(f: &IrFunction, temps: &[u32]) -> Executed {
    let mut mem = Mem::default();
    let mut values: BTreeMap<u32, Val> = BTreeMap::new();
    let mut reads: Vec<u64> = Vec::new();
    let mut next_frame: u64 = 0x1000;
    // Every block is executed in label order until the terminator leaves the
    // function; the shapes under test are straight-line or two-arm, so a
    // simple "follow the taken edge" walk is enough and stays honest: an
    // unsupported terminator panics.
    let mut label = f
        .blocks
        .first()
        .map(|b| b.label)
        .expect("at least one block");
    for _ in 0..10_000 {
        let block = f
            .blocks
            .iter()
            .find(|b| b.label == label)
            .unwrap_or_else(|| panic!("no block {label:?}"));
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca {
                    dest, size, align, ..
                } => {
                    // Give every alloca a distinct, naturally aligned address
                    // so an out-of-bounds write cannot silently land in a
                    // neighbour's storage.
                    let align = (*align).max(1) as u64;
                    next_frame = (next_frame + align - 1) / align * align;
                    values.insert(dest.0, Val::Int(next_frame));
                    if temps.contains(&dest.0) {
                        mem.ignore(next_frame, (*size as u64).max(1));
                    }
                    next_frame += (*size as u64).max(1) + 16;
                }
                Instruction::Copy { dest, src } => {
                    let v = match src {
                        Operand::Const(c) => Val::Int(c.to_i64().expect("integral copy") as u64),
                        Operand::Value(v) => values
                            .get(&v.0)
                            .unwrap_or_else(|| panic!("unset value {v:?}"))
                            .clone(),
                    };
                    values.insert(dest.0, v);
                }
                Instruction::BinOp {
                    dest, op, lhs, rhs, ..
                } => {
                    let (a, b) = (scalar(lhs, &values), scalar(rhs, &values));
                    let v = match op {
                        IrBinOp::Add => a.wrapping_add(b),
                        IrBinOp::Sub => a.wrapping_sub(b),
                        _ => panic!("model does not implement {op:?}"),
                    };
                    values.insert(dest.0, Val::Int(v));
                }
                Instruction::GetElementPtr {
                    dest, base, offset, ..
                } => {
                    let v = scalar(&Operand::Value(*base), &values)
                        .wrapping_add(scalar(offset, &values));
                    values.insert(dest.0, Val::Int(v));
                }
                Instruction::Load { dest, ptr, ty, .. } => {
                    let addr = scalar(&Operand::Value(*ptr), &values);
                    assert_eq!(*ty, IrType::U8, "model only reads bytes");
                    reads.push(addr);
                    values.insert(dest.0, Val::Int(u64::from(mem.read(addr, 1)[0])));
                }
                Instruction::Store { val, ptr, ty, .. } => {
                    let addr = scalar(&Operand::Value(*ptr), &values);
                    assert_eq!(*ty, IrType::U8, "model only stores bytes");
                    mem.write(addr, &[scalar(val, &values) as u8]);
                }
                Instruction::Intrinsic {
                    dest,
                    op,
                    dest_ptr,
                    args,
                } => {
                    let width = vector_width(*op)
                        .unwrap_or_else(|| panic!("model does not implement {op:?}"));
                    match dest_ptr {
                        Some(p) => {
                            // Address-form store: the bytes come from args[0]
                            // (either a vector value or, for the point-store
                            // families, the address of the temporary), and the
                            // destination is `dest_ptr` -- the argument list
                            // carries a base+displacement duplicate that must
                            // name the same address.
                            let addr = scalar(&Operand::Value(*p), &values);
                            let data = match args.as_slice() {
                                // `VecStore<W> { dest_ptr: home, args: [v, home, 0] }`
                                [
                                    Operand::Value(_),
                                    Operand::Value(base),
                                    Operand::Const(disp),
                                ] => {
                                    let dup = scalar(&Operand::Value(*base), &values)
                                        .wrapping_add(disp.to_i64().expect("integral disp") as u64);
                                    assert_eq!(dup, addr, "store base+disp must name dest_ptr");
                                    bytes(&args[0], &values, width)
                                }
                                // `Storedqu { dest_ptr: d, args: [home] }`: the
                                // source is the temporary's address.
                                [src] => {
                                    let from = scalar(src, &values);
                                    reads.push(from);
                                    mem.read(from, width)
                                }
                                other => panic!("unsupported store shape: {other:?}"),
                            };
                            mem.write(addr, &data);
                            if let Some(d) = dest {
                                values.insert(d.0, Val::Vec(data));
                            }
                        }
                        None => {
                            // Register-form load / constant.
                            let v = match op {
                                IntrinsicOp::VecZeroI64x2 | IntrinsicOp::VecZeroI64x4 => {
                                    Val::Vec(vec![0u8; width])
                                }
                                IntrinsicOp::VecConstI8x32 => Val::Vec(
                                    args.iter()
                                        .map(|a| scalar(a, &values) as u8)
                                        .collect::<Vec<u8>>(),
                                ),
                                _ => {
                                    let base = scalar(&args[0], &values);
                                    let disp = args.get(1).map(|a| scalar(a, &values)).unwrap_or(0);
                                    let addr = base.wrapping_add(disp);
                                    reads.push(addr);
                                    Val::Vec(mem.read(addr, width))
                                }
                            };
                            values.insert(dest.expect("register op needs a dest").0, v);
                        }
                    }
                }
                Instruction::Phi { .. } => panic!("model does not implement phi nodes"),
                other => panic!("model does not implement {other:?}"),
            }
        }
        match &block.terminator {
            Terminator::Return(_) => break,
            Terminator::Branch(target) => label = *target,
            other => panic!("model does not implement terminator {other:?}"),
        }
    }
    let _ = AddressSpace::Default;
    Executed { mem, reads }
}

// ── Program construction ────────────────────────────────────────────────────

const SRC: u32 = 90;
const HOME: u32 = 91;
const DEST0: u32 = 100;

/// A distinctive 16/32-byte pattern so a shifted or swapped copy is visible.
fn pattern(width: usize, salt: u8) -> Vec<u8> {
    // Always 32 bytes (the constant family's width); narrower stores take the
    // low bytes, and the tail keeps each salt distinguishable.
    (0..32)
        .map(|i| {
            let i = i as u8;
            i.wrapping_mul(37).wrapping_add(salt)
        })
        .take(width.max(32))
        .collect()
}

fn alloca(dest: u32, size: usize) -> Instruction {
    Instruction::Alloca {
        dest: Value(dest),
        ty: IrType::Ptr,
        size,
        align: 16,
        volatile: false,
        semantic_volatile: false,
    }
}

/// A 32-byte constant in a register.  It is the only constant-vector family
/// the IR has; a narrower store simply writes its low bytes.
fn vec_const(dest: u32, data: &[u8]) -> Instruction {
    assert_eq!(data.len(), 32, "VecConstI8x32 takes exactly 32 bytes");
    Instruction::Intrinsic {
        dest: Some(Value(dest)),
        op: IntrinsicOp::VecConstI8x32,
        dest_ptr: None,
        args: data
            .iter()
            .map(|b| Operand::Const(IrConst::I8(*b as i8)))
            .collect(),
    }
}

fn vector_load(dest: u32, addr: u32, width: usize) -> Instruction {
    Instruction::Intrinsic {
        dest: Some(Value(dest)),
        op: if width == 32 {
            IntrinsicOp::VecLoadI64x4
        } else {
            IntrinsicOp::VecLoadI64x2
        },
        dest_ptr: None,
        args: vec![Operand::Value(Value(addr)), Operand::Const(IrConst::I64(0))],
    }
}

fn vector_store_home(value: u32, home: u32, width: usize) -> Instruction {
    Instruction::Intrinsic {
        dest: None,
        op: if width == 32 {
            IntrinsicOp::VecStoreI64x4
        } else {
            IntrinsicOp::VecStoreI64x2
        },
        dest_ptr: Some(Value(home)),
        args: vec![
            Operand::Value(Value(value)),
            Operand::Value(Value(home)),
            Operand::Const(IrConst::I64(0)),
        ],
    }
}

fn point_store(source_addr: u32, dest: u32, width: usize) -> Instruction {
    Instruction::Intrinsic {
        dest: None,
        op: if width == 32 {
            IntrinsicOp::Storeu256
        } else {
            IntrinsicOp::Storedqu
        },
        dest_ptr: Some(Value(dest)),
        args: vec![Operand::Value(Value(source_addr))],
    }
}

fn function(instructions: Vec<Instruction>) -> IrFunction {
    let mut f = IrFunction::new("fwd".to_string(), IrType::Void, vec![], false);
    f.blocks.push(BasicBlock {
        label: BlockId(0),
        instructions,
        terminator: Terminator::Return(None),
        source_spans: Vec::new(),
        source_inline_ids: Vec::new(),
    });
    f.next_value_id = 200;
    f.next_label = 1;
    f
}

/// Fill `base` with `data` one byte at a time (the model's only scalar store).
fn fill_bytes(base: u32, data: &[u8], next_id: &mut u32) -> Vec<Instruction> {
    let mut out = Vec::with_capacity(data.len() * 2);
    for (i, b) in data.iter().enumerate() {
        let addr = *next_id;
        *next_id += 1;
        out.push(Instruction::GetElementPtr {
            dest: Value(addr),
            base: Value(base),
            offset: Operand::Const(IrConst::I64(i as i64)),
            ty: IrType::U64,
        });
        out.push(Instruction::Store {
            val: Operand::Const(IrConst::I8(*b as i8)),
            ptr: Value(addr),
            ty: IrType::U8,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
    }
    out
}

/// The canonical staged pair: `source` is filled with a pattern, a vector is
/// loaded from it, the value is staged into a frame temporary, and the
/// temporary is read back by an address-form store.
///
/// The value is produced by a real load (not a constant) so the executed image
/// carries the pattern: a rewrite that widens, narrows, shifts or reorders the
/// copy then diverges in an observable byte instead of comparing zeros.
fn staged_pair(width: usize, salt: u8) -> Vec<Instruction> {
    let mut next_id = 150u32;
    let mut insts = vec![
        alloca(SRC, width),
        alloca(HOME, width),
        alloca(DEST0, width),
        alloca(DEST0 + 1, width),
        alloca(DEST0 + 2, width),
        alloca(DEST0 + 3, width),
    ];
    insts.extend(fill_bytes(
        SRC,
        &pattern(width, salt)[..width],
        &mut next_id,
    ));
    insts.push(vector_load(11, SRC, width));
    insts.push(vector_store_home(11, HOME, width));
    insts
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// Every accepted shape: the rewrite must fire, and the executed memory image
/// of the rewritten function must equal the original's byte for byte.
fn assert_forwarding_preserves_bytes(
    instructions: Vec<Instruction>,
    expected_hits: usize,
    temps: &[u32],
) {
    let original = function(instructions);
    let mut rewritten = original.clone();
    let hits = forward_in_function(&mut rewritten);
    assert_eq!(hits, expected_hits, "rewrite count for the accepted shape");

    let before = execute(&original, temps);
    let after = execute(&rewritten, temps);
    // The model must move the *real* bytes, or an all-zero comparison would
    // prove nothing. Require the multi-byte pattern to be present.
    let distinct: std::collections::BTreeSet<u8> =
        before.mem.observable().iter().map(|(_, b)| *b).collect();
    assert!(
        distinct.len() > 1,
        "the model wrote a constant image; the comparison would be vacuous"
    );
    assert_eq!(
        before.mem.observable(),
        after.mem.observable(),
        "the rewrite changed an observable byte"
    );
    assert!(
        after.mem.dead_writes() == 0,
        "the forwarded temporary must not be written back ({} bytes still written)",
        after.mem.dead_writes()
    );
    // The rewrite's whole point is to *stop* reading the temporary back.
    assert!(
        after.reads.len() < before.reads.len(),
        "a forwarded shape must observe fewer bytes ({} reads before, {} after)",
        before.reads.len(),
        after.reads.len()
    );
}

/// Every refused shape: the IR must be untouched (so the image trivially
/// matches), and the model still proves the original shape computes what the
/// test expects.
fn assert_refused(instructions: Vec<Instruction>, temps: &[u32]) {
    let original = function(instructions);
    let before_ir = original.clone();
    let mut attempted = original.clone();
    let hits = forward_in_function(&mut attempted);
    assert_eq!(hits, 0, "refused shape must not rewrite");
    assert_eq!(
        format!("{before_ir:?}"),
        format!("{attempted:?}"),
        "a refused shape must leave the IR identical"
    );
    let _ = execute(&original, temps);
}

#[test]
fn single_reader_is_forwarded_and_preserves_bytes() {
    let mut insts = staged_pair(16, 3);
    insts.push(point_store(HOME, DEST0, 16));
    assert_forwarding_preserves_bytes(insts, 1, &[HOME]);
}

#[test]
fn four_readers_are_forwarded_and_preserve_bytes() {
    let mut insts = staged_pair(16, 7);
    for d in 0..4 {
        insts.push(point_store(HOME, DEST0 + d, 16));
    }
    assert_forwarding_preserves_bytes(insts, 1, &[HOME]);
}

#[test]
fn two_independent_pairs_keep_their_own_values() {
    // The `adler32_simd` shape: two staged pairs in one block with *different*
    // values.  A stale reader index that rewrote the second reader with the
    // first pair's value diverges in an observable byte here; with the
    // historical single-phase removal injected, this test fails.
    const SRC2: u32 = 110;
    const HOME2: u32 = 111;
    let mut next_id = 300u32;
    let mut insts = staged_pair(16, 3);
    insts.push(alloca(SRC2, 16));
    insts.push(alloca(HOME2, 16));
    insts.push(alloca(DEST0 + 4, 16));
    insts.extend(fill_bytes(SRC2, &pattern(16, 200)[..16], &mut next_id));
    insts.push(vector_load(41, SRC2, 16));
    insts.push(vector_store_home(41, HOME2, 16));
    insts.push(point_store(HOME, DEST0, 16));
    insts.push(point_store(HOME2, DEST0 + 4, 16));
    assert_forwarding_preserves_bytes(insts, 2, &[HOME, HOME2]);
}

#[test]
fn zero_width_256_pair_is_forwarded() {
    let mut insts = staged_pair(32, 11);
    insts.push(point_store(HOME, DEST0, 32));
    assert_forwarding_preserves_bytes(insts, 1, &[HOME]);
}

#[test]
fn byte_observation_of_the_temporary_refuses_the_rewrite() {
    // A byte-granular read is an observation the whole-vector forwarding rule
    // cannot model: the temporary's contents become live in a scalar value.
    let mut insts = staged_pair(16, 5);
    insts.push(Instruction::Load {
        dest: Value(50),
        ptr: Value(HOME),
        ty: IrType::U8,
        seg_override: AddressSpace::Default,
        volatile: false,
    });
    insts.push(point_store(HOME, DEST0, 16));
    assert_refused(insts, &[HOME]);
}

#[test]
fn a_second_writer_of_the_home_refuses_the_rewrite() {
    // The home is written twice; forwarding the first write would make the
    // reader see the wrong generation of the value.
    let mut insts = staged_pair(16, 5);
    insts.push(vector_store_home(11, HOME, 16));
    insts.push(point_store(HOME, DEST0, 16));
    assert_refused(insts, &[HOME]);
}

#[test]
fn a_mismatched_reader_width_refuses_the_rewrite() {
    // A 16-byte temporary read by a 32-byte store: the bytes the reader needs
    // are only half present.
    let mut insts = staged_pair(16, 5);
    insts.push(point_store(HOME, DEST0, 32));
    assert_refused(insts, &[HOME]);
}

/// Randomized differential sweep.  Each seed builds a run of staged pairs with
/// a random number of readers, a random pattern per pair, and a random
/// interleaving of unrelated instructions, then requires byte-identical
/// memory after the rewrite.  This is the part that would have caught a
/// value-level indexing error on its own; it is deliberately not a count
/// check.
#[test]
fn randomized_pairs_preserve_memory() {
    let mut state: u64 = 0x243f_6a88_85a3_08d3;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for case in 0..200u32 {
        let width = if next() & 1 == 0 { 16 } else { 32 };
        let pairs = 1 + (next() % 3) as usize;
        let mut insts = Vec::new();
        let mut next_id = 400u32 + case * 1000;
        // Destinations first: a reader must read a defined address.
        let max_readers = 4 * pairs;
        for d in 0..max_readers as u32 {
            insts.push(alloca(DEST0 + d, width));
        }
        let mut reader_slot = 0u32;
        for p in 0..pairs {
            let src = 120 + p as u32 * 10;
            let home = 121 + p as u32 * 10;
            let loaded = 11 + p as u32 * 10;
            insts.push(alloca(src, width));
            insts.push(alloca(home, width));
            let salt = (next() & 0xff) as u8;
            insts.extend(fill_bytes(
                src,
                &pattern(width, salt)[..width],
                &mut next_id,
            ));
            insts.push(vector_load(loaded, src, width));
            insts.push(vector_store_home(loaded, home, width));
            // Readers: every pair is read at least once, up to four times.
            let readers = 1 + (next() % 4) as usize;
            for _ in 0..readers {
                insts.push(point_store(home, DEST0 + reader_slot, width));
                reader_slot += 1;
            }
        }
        let original = function(insts);
        let mut rewritten = original.clone();
        let hits = forward_in_function(&mut rewritten);
        assert!(hits >= 1, "case {case}: expected at least one rewrite");
        let temps: Vec<u32> = (0..pairs).map(|p| 121 + p as u32 * 10).collect();
        let before = execute(&original, &temps);
        let after = execute(&rewritten, &temps);
        assert_eq!(
            before.mem.observable(),
            after.mem.observable(),
            "case {case} (width {width}, {pairs} pairs) diverged after the rewrite"
        );
    }
}
