//! Pass a one-register aggregate as the register it is.
//!
//! # The shape
//!
//! ```c
//! struct bw_fixed { int64_t value; };
//! struct bw_fixed bw_mul(struct bw_fixed a, struct bw_fixed b);   /* extern */
//! x = bw_mul(bw_add(p, q), bw_int_to_fixed(n));
//! ```
//!
//! A by-value aggregate argument is a pointer to the bytes plus a size in this
//! IR, and the backend reads the bytes when it sets the call up. A value that
//! was just computed therefore needs an object to be read from: lowering and
//! `aggregate_sroa` build one image per argument,
//!
//! ```text
//! v = add p, q
//! I = alloca 8
//! store v -> I
//! call bw_mul(I by value, ...)
//! ```
//!
//! and each image is an alloca the call names, so it keeps a frame slot of its
//! own for the whole function. Linux's `calculate_bandwidth`
//! (drivers/gpu/drm/amd/display/dc/basics/dce_calcs.c) is fourteen hundred
//! such calls on `struct bw_fixed`: 972 slots, a 7832-byte frame against GCC
//! 16.2's 240 (and the kernel's 2048-byte `-Wframe-larger-than`), and a store
//! and a reload through memory around every argument. `struct fixed31_32`,
//! `pte_t`, `pmd_t`, `kuid_t` and every other one-word wrapper type travel the
//! same way.
//!
//! # The rewrite
//!
//! On x86-64 an aggregate of one INTEGER eightbyte is passed exactly as an
//! integer of its size is: in the next general register, or in the next
//! eight-byte stack slot when none is left (psABI 3.2.3; there is no second
//! eightbyte to make the all-or-nothing rule matter). So when the image is
//! nothing but a carrier — one alloca of 1, 2, 4 or 8 bytes, written once with
//! one integer or pointer store that covers it, and read only by this one
//! argument, later in the store's block — the argument becomes the stored
//! value itself and the image is deleted. (The value is defined before the
//! store, so it is defined at the call; the store is the image's only writer,
//! so the call read exactly that value.) The value then lives where every
//! other value lives: in a register, or in a spill slot the allocator shares.
//!
//! # Why last
//!
//! A call that passes a scalar where the callee declares an aggregate
//! parameter is the same machine call, but not the same IR call: the inliner
//! and the interprocedural passes pair arguments with parameters by position
//! and would bind a value where they expect an address. This rewrite is
//! therefore the last IR transform of the pipeline; only code generation
//! follows, and code generation reads the call alone.
//!
//! Kill switch: `CCC_DISABLE_PASSES=byvalscalar`.

use crate::common::fx_hash::FxHashMap;
use crate::common::types::{AddressSpace, EightbyteClass, IrType};
use crate::ir::reexports::{Instruction, IrFunction, IrModule, Operand, Value};

/// An image that may be a carrier: where it is, and what was seen of it.
/// Positions are `(block index, instruction index)`.
struct Image {
    alloca: (usize, usize),
    size: usize,
    /// Uses of the alloca's address, of any kind.
    uses: u32,
    /// The one whole-object store, with the value and the type it stores.
    store: Option<((usize, usize), Operand, IrType)>,
    /// The one by-value argument, with its index in the call.
    argument: Option<((usize, usize), usize)>,
    /// Something this rewrite cannot account for names the image.
    refused: bool,
}

/// Run on every defined function; returns the number of arguments rewritten.
pub(crate) fn run(module: &mut IrModule) -> usize {
    module.for_each_function(run_function)
}

fn carried_type(ty: IrType, size: usize) -> bool {
    ty.size() == size
        && matches!(
            ty,
            IrType::I8
                | IrType::U8
                | IrType::I16
                | IrType::U16
                | IrType::I32
                | IrType::U32
                | IrType::I64
                | IrType::U64
                | IrType::Ptr
        )
}

pub(crate) fn run_function(func: &mut IrFunction) -> usize {
    // Candidate images: fixed-size, non-volatile allocas of one register.
    let mut images: FxHashMap<u32, Image> = FxHashMap::default();
    for (block, bb) in func.blocks.iter().enumerate() {
        for (alloca_index, inst) in bb.instructions.iter().enumerate() {
            if let Instruction::Alloca {
                dest,
                size,
                volatile: false,
                semantic_volatile: false,
                ..
            } = inst
                && matches!(*size, 1 | 2 | 4 | 8)
            {
                images.insert(
                    dest.0,
                    Image {
                        alloca: (block, alloca_index),
                        size: *size,
                        uses: 0,
                        store: None,
                        argument: None,
                        refused: false,
                    },
                );
            }
        }
    }
    if images.is_empty() {
        return 0;
    }

    // Every use of every candidate, counted through the canonical visitor so
    // no instruction kind can name an image unseen; the two uses this rewrite
    // understands are then recognised by shape.
    for (block, bb) in func.blocks.iter().enumerate() {
        for (index, inst) in bb.instructions.iter().enumerate() {
            inst.for_each_used_value(|id| {
                if let Some(image) = images.get_mut(&id) {
                    image.uses += 1;
                }
            });
            match inst {
                Instruction::Store {
                    val,
                    ptr,
                    ty,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                } => {
                    if let Some(image) = images.get_mut(&ptr.0) {
                        if carried_type(*ty, image.size) && image.store.is_none() {
                            image.store = Some(((block, index), *val, *ty));
                        } else {
                            image.refused = true;
                        }
                    }
                    // The image's ADDRESS stored somewhere is an escape.
                    if let Operand::Value(v) = val
                        && let Some(image) = images.get_mut(&v.0)
                    {
                        image.refused = true;
                    }
                }
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    for (arg_index, arg) in info.args.iter().enumerate() {
                        let Operand::Value(v) = arg else { continue };
                        let Some(image) = images.get_mut(&v.0) else {
                            continue;
                        };
                        let one_integer_eightbyte = info.struct_arg_sizes.get(arg_index).copied()
                            == Some(Some(image.size))
                            && info
                                .struct_arg_classes
                                .get(arg_index)
                                .is_some_and(|c| c.as_slice() == [EightbyteClass::Integer])
                            && !info
                                .struct_arg_is_f128_sse
                                .get(arg_index)
                                .copied()
                                .unwrap_or(false)
                            && info
                                .struct_arg_fp_agg_classes
                                .get(arg_index)
                                .is_none_or(Option::is_none);
                        if one_integer_eightbyte && image.argument.is_none() {
                            image.argument = Some(((block, index), arg_index));
                        } else {
                            image.refused = true;
                        }
                    }
                }
                _ => {}
            }
        }
        bb.terminator.for_each_used_value(|id| {
            if let Some(image) = images.get_mut(&id) {
                image.refused = true;
            }
        });
    }

    // A carrier: exactly the store and the argument, the store first in the
    // block they share.
    let mut rewrites: Vec<(
        &Image,
        (usize, usize),
        (usize, usize),
        usize,
        Operand,
        IrType,
    )> = Vec::new();
    for image in images.values() {
        if image.refused || image.uses != 2 {
            continue;
        }
        let (Some((store, value, ty)), Some((call, arg_index))) = (image.store, image.argument)
        else {
            continue;
        };
        if store.0 == call.0 && store.1 < call.1 {
            rewrites.push((image, store, call, arg_index, value, ty));
        }
    }
    if rewrites.is_empty() {
        return 0;
    }

    let mut doomed: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for &(image, store, call, arg_index, value, ty) in &rewrites {
        if let Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } =
            &mut func.blocks[call.0].instructions[call.1]
        {
            info.args[arg_index] = value;
            info.arg_types[arg_index] = ty;
            info.struct_arg_sizes[arg_index] = None;
            if let Some(align) = info.struct_arg_aligns.get_mut(arg_index) {
                *align = None;
            }
            if let Some(classes) = info.struct_arg_classes.get_mut(arg_index) {
                classes.clear();
            }
        }
        doomed
            .entry(image.alloca.0)
            .or_default()
            .push(image.alloca.1);
        doomed.entry(store.0).or_default().push(store.1);
    }
    let count = rewrites.len();
    for (block, mut indices) in doomed {
        indices.sort_unstable();
        indices.dedup();
        let bb = &mut func.blocks[block];
        let spans = bb.source_spans.len() == bb.instructions.len();
        let inline_ids = bb.source_inline_ids.len() == bb.instructions.len();
        for &index in indices.iter().rev() {
            bb.instructions.remove(index);
            if spans {
                bb.source_spans.remove(index);
            }
            if inline_ids {
                bb.source_inline_ids.remove(index);
            }
        }
    }
    count
}

/// The image of one argument, by value id, for the tests below.
#[cfg(test)]
fn image_of(func: &IrFunction, value: Value) -> bool {
    func.blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .any(|i| matches!(i, Instruction::Alloca { dest, .. } if *dest == value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, CallInfo, IrConst, Terminator};

    fn alloca(dest: u32, size: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::I64,
            size,
            align: 0,
            volatile: false,
            semantic_volatile: false,
        }
    }
    fn store(val: Operand, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Store {
            val,
            ptr: Value(ptr),
            ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }
    fn value(id: u32) -> Operand {
        Operand::Value(Value(id))
    }
    fn producer(dest: u32) -> Instruction {
        Instruction::Copy {
            dest: Value(dest),
            src: Operand::Const(IrConst::I64(i64::from(dest))),
        }
    }
    /// `callee(args...)`, each argument `(operand, by-value size, classes)`.
    fn call(args: &[(Operand, Option<usize>, &[EightbyteClass])]) -> Instruction {
        Instruction::Call {
            func: "callee".into(),
            info: CallInfo {
                args: args.iter().map(|a| a.0).collect(),
                arg_types: args.iter().map(|_| IrType::Ptr).collect(),
                return_type: IrType::Void,
                num_fixed_args: args.len(),
                struct_arg_sizes: args.iter().map(|a| a.1).collect(),
                struct_arg_aligns: args.iter().map(|a| a.1).collect(),
                struct_arg_classes: args.iter().map(|a| a.2.to_vec()).collect(),
                struct_arg_fp_agg_classes: vec![None; args.len()],
                struct_arg_is_f128_sse: vec![false; args.len()],
                ..CallInfo::default()
            },
        }
    }
    fn function(blocks: Vec<Vec<Instruction>>) -> IrFunction {
        let mut f = IrFunction::new("caller".into(), IrType::Void, vec![], false);
        let last = blocks.len() - 1;
        f.next_label = blocks.len() as u32;
        f.next_value_id = 100;
        for (index, instructions) in blocks.into_iter().enumerate() {
            f.blocks.push(BasicBlock {
                label: BlockId(index as u32),
                instructions,
                terminator: if index == last {
                    Terminator::Return(None)
                } else {
                    Terminator::Branch(BlockId(index as u32 + 1))
                },
                source_spans: vec![],
                source_inline_ids: Vec::new(),
            });
        }
        f
    }
    fn the_call(f: &IrFunction) -> &CallInfo {
        f.blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .find_map(|i| match i {
                Instruction::Call { info, .. } => Some(info),
                _ => None,
            })
            .expect("a call")
    }
    const INT: &[EightbyteClass] = &[EightbyteClass::Integer];

    /// `bw_mul(bw_add(p, q), bw_int_to_fixed(n))`: both images are carriers.
    /// The arguments become the values, in their stored types, and nothing of
    /// the images is left.
    #[test]
    fn a_carrier_image_becomes_the_value_it_carries() {
        let mut f = function(vec![vec![
            producer(1),
            alloca(2, 8),
            store(value(1), 2, IrType::I64),
            producer(3),
            alloca(4, 4),
            store(Operand::Const(IrConst::I32(7)), 4, IrType::U32),
            call(&[(value(2), Some(8), INT), (value(4), Some(4), INT)]),
        ]]);
        assert_eq!(run_function(&mut f), 2);
        let info = the_call(&f);
        assert_eq!(info.args, vec![value(1), Operand::Const(IrConst::I32(7))]);
        assert_eq!(info.arg_types, vec![IrType::I64, IrType::U32]);
        assert_eq!(info.struct_arg_sizes, vec![None, None]);
        assert_eq!(info.struct_arg_aligns, vec![None, None]);
        assert!(info.struct_arg_classes.iter().all(Vec::is_empty));
        assert!(!image_of(&f, Value(2)) && !image_of(&f, Value(4)));
        assert_eq!(
            f.blocks[0].instructions.len(),
            3,
            "two producers and the call"
        );
        let mut errors = vec![];
        crate::passes::verify::verify_function(&f, "byvalscalar", &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(run_function(&mut f), 0, "idempotent");
    }

    /// Everything that makes the image more than a carrier leaves the call as
    /// it was: a second reader, a second writer, a store that does not cover
    /// it, a non-integer store, another class or size, a use in another block,
    /// an escaping address, the store behind the call.
    #[test]
    fn an_image_that_is_more_than_a_carrier_stays() {
        let load = |ptr: u32| Instruction::Load {
            dest: Value(90),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
            volatile: false,
        };
        let one = |extra: Vec<Instruction>,
                   by_value: (Option<usize>, &'static [EightbyteClass])| {
            let mut block = vec![producer(1), alloca(2, 8), store(value(1), 2, IrType::I64)];
            block.extend(extra);
            block.push(call(&[(value(2), by_value.0, by_value.1)]));
            function(vec![block])
        };
        let cases: Vec<(&str, IrFunction)> = vec![
            ("a second reader", one(vec![load(2)], (Some(8), INT))),
            (
                "a second writer",
                one(vec![store(value(1), 2, IrType::I64)], (Some(8), INT)),
            ),
            (
                "its address stored",
                one(
                    vec![alloca(5, 8), store(value(2), 5, IrType::Ptr)],
                    (Some(8), INT),
                ),
            ),
            (
                "an SSE eightbyte",
                one(vec![], (Some(8), &[EightbyteClass::Sse])),
            ),
            ("a plain pointer argument", one(vec![], (None, &[]))),
            ("a size the call does not name", one(vec![], (Some(4), INT))),
            (
                "a store that does not cover it",
                function(vec![vec![
                    producer(1),
                    alloca(2, 8),
                    store(value(1), 2, IrType::I32),
                    call(&[(value(2), Some(8), INT)]),
                ]]),
            ),
            (
                "a floating store",
                function(vec![vec![
                    producer(1),
                    alloca(2, 8),
                    store(value(1), 2, IrType::F64),
                    call(&[(value(2), Some(8), INT)]),
                ]]),
            ),
            (
                "the store behind the call",
                function(vec![vec![
                    producer(1),
                    alloca(2, 8),
                    call(&[(value(2), Some(8), INT)]),
                    store(value(1), 2, IrType::I64),
                ]]),
            ),
            (
                "the call in another block than the store",
                function(vec![
                    vec![producer(1), alloca(2, 8), store(value(1), 2, IrType::I64)],
                    vec![call(&[(value(2), Some(8), INT)])],
                ]),
            ),
            (
                "two arguments naming one image",
                function(vec![vec![
                    producer(1),
                    alloca(2, 8),
                    store(value(1), 2, IrType::I64),
                    call(&[(value(2), Some(8), INT), (value(2), Some(8), INT)]),
                ]]),
            ),
            (
                "a sixteen-byte image",
                function(vec![vec![
                    producer(1),
                    alloca(2, 16),
                    store(value(1), 2, IrType::I64),
                    call(&[(value(2), Some(16), &[EightbyteClass::Integer; 2])]),
                ]]),
            ),
        ];
        for (name, mut f) in cases {
            let before = format!("{f:?}");
            assert_eq!(run_function(&mut f), 0, "{name}");
            assert_eq!(format!("{f:?}"), before, "{name}: untouched");
        }
    }

    /// The image is usually allocated in the entry block and used in another:
    /// what has to share a block is the store and the call.
    #[test]
    fn the_alloca_may_sit_in_another_block() {
        let mut f = function(vec![
            vec![alloca(2, 8), alloca(4, 8)],
            vec![
                producer(1),
                store(value(1), 2, IrType::I64),
                producer(3),
                store(value(3), 4, IrType::Ptr),
                call(&[(value(2), Some(8), INT), (value(4), Some(8), INT)]),
            ],
        ]);
        assert_eq!(run_function(&mut f), 2);
        assert!(f.blocks[0].instructions.is_empty(), "both allocas are gone");
        let info = the_call(&f);
        assert_eq!(info.args, vec![value(1), value(3)]);
        assert_eq!(info.arg_types, vec![IrType::I64, IrType::Ptr]);
        assert_eq!(f.blocks[1].instructions.len(), 3);
    }

    /// The instruction side tables shrink with the instructions they describe.
    #[test]
    fn source_tables_follow_the_deleted_instructions() {
        let mut f = function(vec![vec![
            producer(1),
            alloca(2, 8),
            store(value(1), 2, IrType::I64),
            call(&[(value(2), Some(8), INT)]),
        ]]);
        let span = |n: u32| crate::common::source::Span::new(n, n + 1, 0);
        f.blocks[0].source_spans = (0..4).map(span).collect();
        assert_eq!(run_function(&mut f), 1);
        assert_eq!(f.blocks[0].source_spans, vec![span(0), span(3)]);
    }
}
