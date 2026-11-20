//! One frame slot for local objects that are never live together.
//!
//! # The shape
//!
//! ```c
//! switch (info->query) {
//! case AMDGPU_INFO_FW_VERSION: {
//!     struct drm_amdgpu_info_firmware fw_info;
//!     ...
//!     return copy_to_user(out, &fw_info, min(size, sizeof(fw_info))) ? -EFAULT : 0;
//! }
//! case AMDGPU_INFO_DEV_INFO: {
//!     struct drm_amdgpu_info_device *dev_info;
//!     ...
//! ```
//!
//! An ioctl handler is a switch whose arms each own a struct, and one call
//! runs one arm. Every such object used to get a slot of its own for the
//! whole function: Linux's `amdgpu_info_ioctl` had 1536 bytes of them in a
//! 2152-byte frame (GCC 16.2: 752; the kernel warns above 2048), and each
//! call touched stack it had no use for.
//!
//! # When two objects may share
//!
//! The IR has no lifetime markers, so "the scope has ended" is not a fact
//! this pass can use. What it can use is the control-flow graph:
//!
//! * nothing can touch an object before an instruction that names it has
//!   run — the address has no other source — so an object *starts* at its
//!   uses;
//! * an object whose address ESCAPES may be touched at any point reachable
//!   from a use. Escaping is everything but the four visible accesses: a
//!   load or a store through the address, either end of a `Memcpy`, and a
//!   `GetElementPtr` off it whose result is defined once (the uses of that
//!   result are then uses of the object, at any depth). A call argument, a
//!   stored or copied pointer, an asm operand, a compare: all escapes;
//! * an object whose every access is visible is live only between two of
//!   them: at points reachable from a use that can still reach one.
//!
//! One visible access is not as local as it looks: a `long double` load.
//! The backends keep the place a `long double` came from and may read it
//! again where the value is used, so such a load counts as an escape (see
//! `reread_at_its_uses`).
//!
//! Two objects conflict when one of them is used at a point where the other
//! is live. If neither is, no execution has both alive at once: on any run,
//! take the object whose first use comes second — the other is not live
//! there, so it is either not started (and by symmetry never starts while
//! this one is live) or finished. Objects without a conflict get one slot,
//! as large and as aligned as the largest of them.
//!
//! Everything is decided on blocks, not instructions, which only adds
//! conflicts: two objects used in one block never share.
//!
//! # What it leaves alone
//!
//! A function that can be re-entered other than along its edges (`setjmp`,
//! `vfork`, `getcontext`, a label a nested function jumps to) keeps every
//! object apart, and so do parameters, over-aligned and `volatile` objects,
//! and `-O0` (one variable, one slot, for the debugger). Such a function is
//! recognised the way the rest of the backend recognises it
//! (`liveness::is_returns_twice_call`: GCC's list of names), so a function
//! that merely carries `__attribute__((returns_twice))` under another name
//! is not; that attribute is parsed and not applied anywhere yet. With a stack protector,
//! buffers share with buffers and the rest with the rest, so the ProPolice
//! order — buffers next to the canary — holds. Past a size budget only the
//! largest objects are considered; the others keep their slots.
//!
//! Kill switch: `CCC_DISABLE_PASSES=alloca_sharing`.

use crate::common::fx_hash::FxHashMap;
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis::{FlatAdj, build_cfg};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{BlockId, Instruction, IrFunction, Operand};

/// A local object that would otherwise get a frame slot of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SharedCandidate {
    pub(super) id: u32,
    pub(super) size: i64,
    pub(super) align: i64,
}

/// Objects considered at most; the rest keep their own slots.
const MAX_OBJECTS: usize = 1024;
/// Upper bound on `objects x blocks`, the cost of the reachability sweeps.
const MAX_OBJECT_BLOCKS: usize = 1 << 22;

/// `CCC_DISABLE_PASSES=alloca_sharing` (or `all`): every object keeps a slot
/// of its own.
pub(super) fn enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        !crate::passes::pass_disabled(
            std::env::var("CCC_DISABLE_PASSES").unwrap_or_default(),
            "alloca_sharing",
        )
    });
    *ENABLED
}

/// Whether control can enter a block of `func` other than along a CFG edge:
/// a second return of `setjmp`/`vfork`, or a non-local `goto` from a nested
/// function. Reachability says nothing about such a function.
pub(super) fn has_irregular_control(func: &IrFunction) -> bool {
    func.blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .any(|inst| {
            crate::backend::liveness::is_returns_twice_call(inst)
                || matches!(
                    inst,
                    Instruction::Intrinsic {
                        op: IntrinsicOp::BuiltinSetjmp,
                        ..
                    } | Instruction::NonlocalGotoSave { .. }
                )
        })
}

/// What the function does with one object.
#[derive(Default)]
struct ObjectUses {
    /// Blocks with a use of the object or of an address derived from it,
    /// ascending and distinct.
    blocks: Vec<u32>,
    /// The address reaches something this scan does not follow.
    escaped: bool,
}

/// The candidate each followed address belongs to.
///
/// A value is FOLLOWED when its one definition is a GEP: every use of it is
/// then a use of whatever its base is, at any depth. Phi elimination and web
/// coalescing can give one id several definitions; such a value is not a
/// derivation this scan can follow, and the GEP that defines it is an escape
/// of its base instead.
struct Derivations {
    /// Followed value -> its base.
    base: FxHashMap<u32, u32>,
    /// Followed value -> position of its object in the candidate list, or
    /// `NO_OBJECT` when its chain of bases does not end at a candidate.
    object: FxHashMap<u32, u32>,
}

/// `Derivations::object` entry of an address into no candidate.
const NO_OBJECT: u32 = u32::MAX;

impl Derivations {
    fn new(func: &IrFunction, index: &FxHashMap<u32, u32>) -> Self {
        let mut defs: FxHashMap<u32, (u32, u32)> = FxHashMap::default();
        for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
            if let Instruction::GetElementPtr { dest, base, .. } = inst {
                defs.entry(dest.0).or_insert((base.0, 0));
            }
        }
        if !defs.is_empty() {
            for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
                if let Some(dest) = inst.dest()
                    && let Some(entry) = defs.get_mut(&dest.0)
                {
                    entry.1 += 1;
                }
            }
        }
        let base: FxHashMap<u32, u32> = defs
            .into_iter()
            .filter(|&(_, (_, definitions))| definitions == 1)
            .map(|(dest, (base, _))| (dest, base))
            .collect();

        // Walk each chain to its end once, and remember the answer for
        // every value on the way — "no object" included, so a long chain
        // off a pointer that is no candidate is walked once too. A chain is
        // as long as the code makes it (there is no depth at which a derived
        // address stops being one), so what bounds the walk is the number of
        // followed values: more steps than that is a cycle, which no
        // candidate can be on, since a candidate has no base.
        let mut object: FxHashMap<u32, u32> = FxHashMap::default();
        let mut path: Vec<u32> = Vec::new();
        for &start in base.keys() {
            path.clear();
            let mut value = start;
            let found = loop {
                if let Some(&candidate) = index.get(&value) {
                    break candidate;
                }
                if let Some(&known) = object.get(&value) {
                    break known;
                }
                match base.get(&value) {
                    Some(&next) if path.len() <= base.len() => {
                        path.push(value);
                        value = next;
                    }
                    _ => break NO_OBJECT,
                }
            };
            for &step in &path {
                object.insert(step, found);
            }
        }
        Self { base, object }
    }

    /// The candidate `value` is an address into, if it is a followed one.
    fn object_of(&self, value: u32) -> Option<u32> {
        self.object
            .get(&value)
            .copied()
            .filter(|&object| object != NO_OBJECT)
    }

    /// Whether `value`'s one definition is a GEP.
    fn follows(&self, value: u32) -> bool {
        self.base.contains_key(&value)
    }
}

/// Whether a value of type `ty` loaded from memory may be read from that
/// memory again where the VALUE is used, instead of where it was loaded.
///
/// A `long double` is. Every backend remembers where one was loaded from
/// (`CodegenState::track_f128_load`) so that a later cast, compare, return
/// or call argument can fetch all sixteen bytes from the original location
/// rather than from a narrower working copy, and nothing ends that binding
/// before the function does. The object therefore has a reader at every use
/// of the loaded value, which no instruction naming the object shows. Those
/// uses all lie behind the load, so treating the load as an escape — alive
/// wherever the load leads — covers each of them.
fn reread_at_its_uses(ty: IrType) -> bool {
    ty.is_long_double()
}

/// Record every use of every candidate; `index` maps an alloca to its
/// position in the candidate list.
fn collect_uses(func: &IrFunction, index: &FxHashMap<u32, u32>, count: usize) -> Vec<ObjectUses> {
    let derived = Derivations::new(func, index);
    let object_of = |value: u32| -> Option<usize> {
        index
            .get(&value)
            .copied()
            .or_else(|| derived.object_of(value))
            .map(|object| object as usize)
    };

    let mut uses: Vec<ObjectUses> = (0..count).map(|_| ObjectUses::default()).collect();
    for (block, bb) in func.blocks.iter().enumerate() {
        let block = block as u32;
        let mut touch = |value: u32, escapes: bool| {
            if let Some(object) = object_of(value) {
                let entry = &mut uses[object];
                if entry.blocks.last() != Some(&block) {
                    entry.blocks.push(block);
                }
                entry.escaped |= escapes;
            }
        };
        for inst in &bb.instructions {
            match inst {
                Instruction::Load {
                    ptr,
                    ty,
                    seg_override,
                    ..
                } => touch(
                    ptr.0,
                    *seg_override != AddressSpace::Default || reread_at_its_uses(*ty),
                ),
                Instruction::Store {
                    val,
                    ptr,
                    seg_override,
                    ..
                } => {
                    touch(ptr.0, *seg_override != AddressSpace::Default);
                    // The ADDRESS stored somewhere is out of sight.
                    if let Operand::Value(v) = val {
                        touch(v.0, true);
                    }
                }
                Instruction::GetElementPtr {
                    dest, base, offset, ..
                } => {
                    touch(base.0, !derived.follows(dest.0));
                    if let Operand::Value(v) = offset {
                        touch(v.0, true);
                    }
                }
                Instruction::Memcpy { dest, src, .. } => {
                    touch(dest.0, false);
                    touch(src.0, false);
                }
                // Every other reader is an escape: the allow-list above is
                // the whole of what "visible access" means.
                other => other.for_each_used_value(|v| touch(v, true)),
            }
        }
        bb.terminator.for_each_used_value(|v| touch(v, true));
    }
    uses
}

/// Set the bit of every block in `seeds` and of every block `adj` reaches
/// from them.
fn mark_reachable(adj: &FlatAdj, seeds: &[u32], bits: &mut [u64], work: &mut Vec<u32>) {
    work.clear();
    for &seed in seeds {
        let (word, bit) = (seed as usize / 64, 1u64 << (seed % 64));
        if bits[word] & bit == 0 {
            bits[word] |= bit;
            work.push(seed);
        }
    }
    while let Some(block) = work.pop() {
        for &next in adj.row(block as usize) {
            let (word, bit) = (next as usize / 64, 1u64 << (next % 64));
            if bits[word] & bit == 0 {
                bits[word] |= bit;
                work.push(next);
            }
        }
    }
}

/// The class of each candidate, numbered from 0 in order of first
/// appearance: candidates of one class may share a slot. With nothing to
/// share, every candidate is its own class.
pub(super) fn share_classes(func: &IrFunction, candidates: &[SharedCandidate]) -> Vec<u32> {
    let count = candidates.len();
    let blocks = func.blocks.len();
    let apart = || (0..count as u32).collect::<Vec<u32>>();
    if count < 2 || blocks < 2 {
        return apart();
    }

    // Largest first: that is the order they are packed in, and the order in
    // which the budget drops them.
    let mut order: Vec<u32> = (0..count as u32).collect();
    order.sort_by_key(|&i| {
        let c = &candidates[i as usize];
        (std::cmp::Reverse(c.size), std::cmp::Reverse(c.align), i)
    });
    let considered = count.min(MAX_OBJECTS).min(MAX_OBJECT_BLOCKS / blocks);
    if considered < 2 {
        return apart();
    }

    let index: FxHashMap<u32, u32> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id, i as u32))
        .collect();
    let uses = collect_uses(func, &index, count);
    let label_to_idx: FxHashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label, i))
        .collect();
    let (preds, succs) = build_cfg(func, &label_to_idx);

    // Row `r` of `live`: the blocks in which the r-th considered object may
    // be alive. Row `r` of `used`: the blocks that use it.
    let words = blocks.div_ceil(64);
    let mut live = vec![0u64; considered * words];
    let mut used = vec![0u64; considered * words];
    let mut back = vec![0u64; words];
    let mut work: Vec<u32> = Vec::new();
    for (row, &object) in order[..considered].iter().enumerate() {
        let object_uses = &uses[object as usize];
        let bits = &mut live[row * words..(row + 1) * words];
        mark_reachable(&succs, &object_uses.blocks, bits, &mut work);
        if !object_uses.escaped {
            back.fill(0);
            mark_reachable(&preds, &object_uses.blocks, &mut back, &mut work);
            for (word, back) in bits.iter_mut().zip(&back) {
                *word &= back;
            }
        }
        let bits = &mut used[row * words..(row + 1) * words];
        for &block in &object_uses.blocks {
            bits[block as usize / 64] |= 1u64 << (block % 64);
        }
    }

    // First fit, largest first: a class is as large as its first member. A
    // class keeps the union of its members' rows, so the test against all of
    // them is two intersections: the object joins when it is used nowhere a
    // member may be alive, and no member is used where the object may be.
    let disjoint = |a: &[u64], b: &[u64]| a.iter().zip(b).all(|(x, y)| x & y == 0);
    let mut class_live: Vec<u64> = Vec::new();
    let mut class_used: Vec<u64> = Vec::new();
    let mut class_count = 0usize;
    let mut class_of = vec![u32::MAX; count];
    for (row, &object) in order[..considered].iter().enumerate() {
        let object_live = &live[row * words..(row + 1) * words];
        let object_used = &used[row * words..(row + 1) * words];
        let fit = (0..class_count).find(|&class| {
            let rows = class * words..(class + 1) * words;
            disjoint(object_used, &class_live[rows.clone()])
                && disjoint(object_live, &class_used[rows])
        });
        let class = fit.unwrap_or_else(|| {
            class_live.resize(class_live.len() + words, 0);
            class_used.resize(class_used.len() + words, 0);
            class_count += 1;
            class_count - 1
        });
        let rows = class * words..(class + 1) * words;
        for (word, object) in class_live[rows.clone()].iter_mut().zip(object_live) {
            *word |= object;
        }
        for (word, object) in class_used[rows].iter_mut().zip(object_used) {
            *word |= object;
        }
        class_of[object as usize] = class as u32;
    }
    if class_count == considered {
        return apart();
    }

    // Renumber by first appearance, the objects past the budget apart.
    let mut renumbered: FxHashMap<u32, u32> = FxHashMap::default();
    let mut next = 0u32;
    class_of
        .iter()
        .map(|&class| {
            if class == u32::MAX {
                next += 1;
                next - 1
            } else {
                *renumbered.entry(class).or_insert_with(|| {
                    next += 1;
                    next - 1
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, CallInfo, IrConst, Terminator, Value};

    fn alloca(id: u32, size: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(id),
            ty: IrType::I64,
            size,
            align: 0,
            volatile: false,
            semantic_volatile: false,
        }
    }
    fn store(ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Const(IrConst::I64(1)),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }
    fn load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }
    /// `callee(&object)`: the address escapes.
    fn pass(ptr: u32) -> Instruction {
        Instruction::Call {
            func: "callee".into(),
            info: CallInfo {
                args: vec![Operand::Value(Value(ptr))],
                arg_types: vec![IrType::Ptr],
                return_type: IrType::Void,
                num_fixed_args: 1,
                struct_arg_sizes: vec![None],
                ..CallInfo::default()
            },
        }
    }
    fn gep(dest: u32, base: u32) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset: Operand::Const(IrConst::I64(8)),
            ty: IrType::Ptr,
        }
    }
    fn jump(to: u32) -> Terminator {
        Terminator::Branch(BlockId(to))
    }
    fn fork(a: u32, b: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Const(IrConst::I32(1)),
            true_label: BlockId(a),
            false_label: BlockId(b),
        }
    }
    fn ret() -> Terminator {
        Terminator::Return(None)
    }
    /// Blocks labelled by position; block 0 holds the allocas of `objects`.
    fn function(
        objects: &[(u32, usize)],
        blocks: Vec<(Vec<Instruction>, Terminator)>,
    ) -> IrFunction {
        let mut f = IrFunction::new("f".into(), IrType::Void, vec![], false);
        for (index, (mut instructions, terminator)) in blocks.into_iter().enumerate() {
            if index == 0 {
                let mut entry: Vec<Instruction> =
                    objects.iter().map(|&(id, size)| alloca(id, size)).collect();
                entry.append(&mut instructions);
                instructions = entry;
            }
            f.blocks.push(BasicBlock {
                label: BlockId(index as u32),
                instructions,
                terminator,
                source_spans: vec![],
                source_inline_ids: Vec::new(),
            });
        }
        f.next_label = f.blocks.len() as u32;
        f
    }
    fn classes(f: &IrFunction, objects: &[(u32, usize)]) -> Vec<u32> {
        let candidates: Vec<SharedCandidate> = objects
            .iter()
            .map(|&(id, size)| SharedCandidate {
                id,
                size: size as i64,
                align: 8,
            })
            .collect();
        share_classes(f, &candidates)
    }

    /// The ioctl shape: each arm hands its own struct to a call and returns.
    /// The addresses escape, and still no run has two of them: one slot.
    #[test]
    fn objects_of_exclusive_arms_share_even_when_they_escape() {
        let objects = [(1, 440), (2, 200), (3, 96)];
        let f = function(
            &objects,
            vec![
                (vec![], fork(1, 2)),
                (vec![store(1), pass(1)], ret()),
                (vec![], fork(3, 4)),
                (vec![pass(2)], ret()),
                (vec![pass(3)], ret()),
            ],
        );
        assert_eq!(classes(&f, &objects), [0, 0, 0]);
        assert!(!has_irregular_control(&f));
    }

    /// The arms meet again and the function goes on: the objects still share,
    /// because each is only ever started on its own arm.
    #[test]
    fn a_join_behind_the_arms_does_not_part_them() {
        let objects = [(1, 64), (2, 64)];
        let f = function(
            &objects,
            vec![
                (vec![], fork(1, 2)),
                (vec![pass(1)], jump(3)),
                (vec![pass(2)], jump(3)),
                (vec![], ret()),
            ],
        );
        assert_eq!(classes(&f, &objects), [0, 0]);
    }

    /// An escaped object is live wherever its first use leads, so an object
    /// used behind it — or in the same block — keeps out of its slot. The
    /// two behind it share with each other: the visible one is over before
    /// the other starts.
    #[test]
    fn an_escaped_object_conflicts_with_everything_behind_it() {
        let objects = [(1, 64), (2, 64), (3, 64)];
        let f = function(
            &objects,
            vec![
                (vec![pass(1)], jump(1)),
                (vec![store(2), load(10, 2)], jump(2)),
                (vec![pass(3), pass(1)], ret()),
            ],
        );
        assert_eq!(classes(&f, &objects), [0, 1, 1]);
    }

    /// An object whose every access is visible ends at its last one: the
    /// next object down the same path takes its slot. A use in between, or a
    /// loop that can bring the first back, keeps them apart.
    #[test]
    fn a_visible_object_ends_at_its_last_access() {
        let objects = [(1, 64), (2, 64)];
        let sequential = function(
            &objects,
            vec![
                (vec![store(1)], jump(1)),
                (vec![gep(20, 1), load(10, 20)], jump(2)),
                (vec![store(2)], jump(3)),
                (vec![load(11, 2)], ret()),
            ],
        );
        assert_eq!(classes(&sequential, &objects), [0, 0]);

        let sandwiched = function(
            &objects,
            vec![
                (vec![store(1)], jump(1)),
                (vec![store(2)], jump(2)),
                (vec![load(10, 1), load(11, 2)], ret()),
            ],
        );
        assert_eq!(classes(&sandwiched, &objects), [0, 1]);

        let looped = function(
            &objects,
            vec![
                (vec![], jump(1)),
                (vec![store(1), load(10, 1)], jump(2)),
                (vec![store(2), load(11, 2)], fork(1, 3)),
                (vec![], ret()),
            ],
        );
        assert_eq!(classes(&looped, &objects), [0, 1]);
    }

    /// A derived address this scan cannot follow — defined twice, copied,
    /// or computed from the address as an offset — makes the object escape,
    /// so a later object down the path does not take its slot.
    #[test]
    fn an_address_out_of_sight_is_an_escape() {
        let objects = [(1, 64), (2, 64)];
        let tail = |first: Vec<Instruction>| {
            function(
                &objects,
                vec![(first, jump(1)), (vec![store(2), load(11, 2)], ret())],
            )
        };
        // Followed: the object is over before block 1.
        assert_eq!(
            classes(&tail(vec![gep(20, 1), load(10, 20)]), &objects),
            [0, 0]
        );
        let copied = Instruction::Copy {
            dest: Value(21),
            src: Operand::Value(Value(1)),
        };
        assert_eq!(classes(&tail(vec![copied]), &objects), [0, 1]);
        // The same derived id defined a second time.
        assert_eq!(
            classes(&tail(vec![gep(20, 1), gep(20, 1), load(10, 20)]), &objects),
            [0, 1]
        );
        let as_offset = Instruction::GetElementPtr {
            dest: Value(22),
            base: Value(30),
            offset: Operand::Value(Value(1)),
            ty: IrType::Ptr,
        };
        assert_eq!(classes(&tail(vec![as_offset]), &objects), [0, 1]);
        // The address itself stored.
        let stored = Instruction::Store {
            val: Operand::Value(Value(1)),
            ptr: Value(31),
            ty: IrType::Ptr,
            seg_override: AddressSpace::Default,
            volatile: false,
        };
        assert_eq!(classes(&tail(vec![stored]), &objects), [0, 1]);
    }

    /// A derived address is one at any depth: a store through the end of a
    /// 100-link chain is a store into the object, so the object is alive
    /// from its first store to that one, and the object used in between
    /// stays out of its slot.
    #[test]
    fn a_long_chain_of_derived_addresses_is_followed_to_its_object() {
        let objects = [(1, 64), (2, 64)];
        let mut chain = vec![gep(100, 1)];
        for link in 1..100 {
            chain.push(gep(100 + link, 99 + link));
        }
        let f = function(
            &objects,
            vec![
                (vec![store(1)], jump(1)),
                (chain.clone(), jump(2)),
                (vec![store(2), load(11, 2)], jump(3)),
                (vec![load(10, 199)], ret()),
            ],
        );
        assert_eq!(classes(&f, &objects), [0, 1]);
        // The same chain off something that is no candidate touches nothing.
        let mut loose = chain;
        loose[0] = gep(100, 50);
        let f = function(
            &objects,
            vec![
                (vec![store(1)], jump(1)),
                (loose, jump(2)),
                (vec![store(2), load(11, 2)], jump(3)),
                (vec![load(10, 199)], ret()),
            ],
        );
        assert_eq!(classes(&f, &objects), [0, 0]);
    }

    /// A class is tested as a whole: the third object fits neither member's
    /// slot alone being free, it has to miss both.
    #[test]
    fn an_object_joins_a_class_only_when_it_misses_every_member() {
        let objects = [(1, 64), (2, 64), (3, 64)];
        // 1 and 2 are on exclusive arms; 3 is used behind the join, after
        // the escaped 2 but with 1 long over.
        let f = function(
            &objects,
            vec![
                (vec![], fork(1, 2)),
                (vec![store(1), load(10, 1)], jump(3)),
                (vec![pass(2)], jump(3)),
                (vec![store(3), load(11, 3)], ret()),
            ],
        );
        assert_eq!(classes(&f, &objects), [0, 0, 1]);
    }

    /// A `long double` read out of an object may be fetched from the object
    /// again at any later use of the value, so the object does not end at
    /// the load: what follows down the path stays out of its slot. The same
    /// load of any other type ends the object there.
    #[test]
    fn a_long_double_load_keeps_the_object_alive_behind_it() {
        let objects = [(1, 64), (2, 64)];
        let load_of = |ty: IrType| Instruction::Load {
            dest: Value(10),
            ptr: Value(1),
            ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        };
        let shape = |ty: IrType| {
            function(
                &objects,
                vec![
                    (vec![store(1)], jump(1)),
                    (vec![load_of(ty)], jump(2)),
                    (vec![store(2), load(11, 2)], ret()),
                ],
            )
        };
        assert_eq!(classes(&shape(IrType::F128), &objects), [0, 1]);
        assert_eq!(classes(&shape(IrType::F64), &objects), [0, 0]);
        assert_eq!(classes(&shape(IrType::I64), &objects), [0, 0]);
    }

    /// Classes are numbered by first appearance and packed largest first:
    /// the two small objects of exclusive arms join the large ones there.
    #[test]
    fn classes_are_packed_largest_first_and_numbered_in_order() {
        let objects = [(1, 8), (2, 400), (3, 16), (4, 200)];
        let f = function(
            &objects,
            vec![
                (vec![], fork(1, 2)),
                (vec![pass(1), pass(2)], ret()),
                (vec![pass(3), pass(4)], ret()),
            ],
        );
        // 400 and 200 share. 16 is used beside 200 and 8 beside 400, so
        // neither joins them; they share a second, 16-byte slot.
        assert_eq!(classes(&f, &objects), [0, 1, 0, 1]);
    }

    /// `setjmp` returns a second time from anywhere: the graph is not the
    /// whole truth, and the caller is told to keep every object apart.
    #[test]
    fn a_second_return_is_irregular_control() {
        let setjmp = Instruction::Call {
            func: "setjmp".into(),
            info: CallInfo::default(),
        };
        let f = function(&[], vec![(vec![setjmp], jump(1)), (vec![], ret())]);
        assert!(has_irregular_control(&f));
    }

    /// With nothing to share, or one block, every object is its own class.
    #[test]
    fn nothing_to_share_leaves_every_object_apart() {
        let objects = [(1, 64), (2, 64)];
        let one_block = function(&objects, vec![(vec![pass(1), pass(2)], ret())]);
        assert_eq!(classes(&one_block, &objects), [0, 1]);
        let f = function(
            &objects,
            vec![(vec![pass(1)], jump(1)), (vec![pass(2)], ret())],
        );
        assert_eq!(classes(&f, &objects), [0, 1]);
        assert_eq!(classes(&f, &objects[..1]), [0]);
    }
}
