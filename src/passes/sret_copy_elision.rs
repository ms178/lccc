//! Retarget a structure-returning call at the aggregate copy it feeds.
//!
//! A C statement whose initializer is a structure-returning call lowers to a
//! hidden (sret) temporary that the call fills in, followed by an aggregate
//! copy into the object the program actually named:
//!
//! ```text
//! %tmp  = alloca 24                 # hidden return slot
//! %obj  = alloca 24                 # the C object
//! call  f(%tmp)                     # writes the struct into %tmp
//! memcpy %obj <- %tmp, 24           # the C assignment
//! call  g(..., %obj, ...)           # %obj passed by value
//! ```
//!
//! When the hidden slot is written exactly once and read exactly once — by the
//! copy itself — the copy moves bytes that the call could have written into
//! their final home directly.  Retargeting the sret argument at the destination
//! and deleting the copy leaves the destination holding the same bytes at the
//! same point in the program, and the hidden slot unread.
//!
//! This is not a micro-optimisation in the corpus that motivated it: zstd's
//! sequence decoder runs it once per decoded sequence, and the 24-byte
//! store/reload pair dominated the hot loop (~15 % of L9 decode samples in
//! `ZSTD_decompressSequences_default`; the caller then passed the copied
//! struct by value to the inlined `ZSTD_execSequence`).
//!
//! # Why forwarding here is not the same as forwarding a copy to its source
//!
//! `aggregate_copy_forward` forwards *reads* of a copy to the source, which is
//! illegal once the source is written again.  This pass never lets a reader see
//! the source: it moves the *writer*, so every later access still goes through
//! the destination object.  The callee consequently writes the caller's object
//! exactly as it previously wrote the hidden slot, which is why no escape
//! analysis is needed — only the proof that the hidden slot is dead after the
//! copy.
//!
//! # Soundness conditions
//!
//! For `memcpy %D <- %S, n` with `%S` written by an sret call in the same block:
//!
//! 1. both objects are allocas of exactly `n` bytes, so the copy covers the
//!    whole destination and the whole source (no partial-object aliasing);
//! 2. `%S` is mentioned exactly twice in the function: as the call's hidden
//!    pointer argument and as this copy's source.  Anything else — a load, a
//!    second copy, a nested sret call, an inline-asm operand, a return — keeps
//!    the value alive and the transform is declined;
//! 3. the call precedes the copy in the same block, so the bytes being copied
//!    are the bytes the call wrote (a call in another block would need a
//!    dominance argument, and this pass deliberately does not make one).
//!
//! Nothing is required of the destination: it may be read, written, passed by
//! pointer or left untouched afterwards, because its contents after the copy
//! are bit-identical either way.
//!
//! # Second phase: forwarding a copy whose reads span blocks
//!
//! A by-value struct argument that the inliner kept as a parameter copy has a
//! different shape — the copy is followed by reads in many blocks:
//!
//! ```text
//! %copy = alloca 24
//! memcpy %copy <- %obj, 24
//! ... load %copy[0] in block B1, load %copy[8] in block B2, ...
//! ```
//!
//! `aggregate_copy_forward` forwards these to `%obj` only when every read is
//! in the copy's own block, because it proves the source unmodified with a
//! block-local scan. A dominance argument extends that to the general case,
//! and it needs no aliasing or reachability analysis:
//!
//! * **every write to the source dominates the copy.** Then a write can only
//!   re-execute by passing through the copy again, so each read in the copy's
//!   dominance region sees the value that was copied — the same value the
//!   forwarding read now sees.
//! * **every read of the copy is dominated by the copy** (or follows it in
//!   the copy's block), so no read can precede the initialization.
//! * the copy is the destination object's **only** write, so nothing else can
//!   make a read observe a different value than the source's.
//!
//! Under those conditions every load through the copied object can read the
//! source instead and the copy disappears. This is the shape zstd's decoder
//! produces for `ZSTD_execSequence(sequence)` after inlining, where the copy
//! was 15 % of all L9 decode samples.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{Instruction, IrFunction, Operand};

/// Bytes of a single `alloca`, keyed by the alloca's value id.
fn alloca_sizes(func: &IrFunction) -> FxHashMap<u32, usize> {
    let mut sizes = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, size, .. } = inst {
                sizes.insert(dest.0, *size);
            }
        }
    }
    sizes
}

/// Every operand position of one instruction, memory destinations included.
///
/// `Instruction::for_each_used_value` already reports the addresses an
/// instruction reads or writes (`Memcpy` destination, `Store` pointer,
/// `Call` arguments); the extra match arm below records which of those
/// positions *write* through the pointer, because a written hidden slot has a
/// different lifetime than a read one.
fn note_writes(inst: &Instruction, writes: &mut FxHashMap<u32, usize>) {
    match inst {
        Instruction::Store { ptr, .. } => *writes.entry(ptr.0).or_insert(0) += 1,
        Instruction::Memcpy { dest, .. } => *writes.entry(dest.0).or_insert(0) += 1,
        Instruction::Call { info, .. } => {
            if info.is_sret {
                if let Some(OperandValue(v)) = info.args.first().map(operand_value) {
                    *writes.entry(v).or_insert(0) += 1;
                }
            }
        }
        Instruction::InlineAsm { .. }
        | Instruction::Intrinsic { .. }
        | Instruction::AtomicRmw { .. } => {
            // An opaque transfer whose memory effects this pass does not
            // model: any value it mentions is treated as both read and
            // written, so a hidden slot that reaches one is never forwarded.
            let mut mark = |v: u32| {
                *writes.entry(v).or_insert(0) += 1;
            };
            inst.for_each_used_value(&mut mark);
        }
        _ => {}
    }
}

/// A copy of `Operand::Value(..)`, so the match above can be written inline.
#[derive(Clone, Copy)]
struct OperandValue(u32);

fn operand_value(op: &crate::ir::reexports::Operand) -> OperandValue {
    match op {
        crate::ir::reexports::Operand::Value(v) => OperandValue(v.0),
        _ => OperandValue(u32::MAX),
    }
}

/// Forward sret calls into the aggregate destination they are copied to.
///
/// Returns the number of copies removed.  The function is rewritten in place;
/// every edit is index-stable per block.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    if func.blocks.is_empty() {
        return 0;
    }
    let sizes = alloca_sizes(func);
    if sizes.is_empty() {
        return 0;
    }

    // Mentions and writes per value. A mention is any operand position; the
    // hidden slot must have exactly one mention that reads it (the copy) and
    // one that writes it (the sret call), which the rules below pin down.
    let mut mentions: FxHashMap<u32, usize> = FxHashMap::default();
    let mut writes: FxHashMap<u32, usize> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| *mentions.entry(v).or_insert(0) += 1);
            note_writes(inst, &mut writes);
        }
        // A value returned from the function or named by a terminator is
        // live even though it appears in no instruction.
        block.terminator.for_each_used_value(|v| {
            *mentions.entry(v).or_insert(0) += 1;
            *writes.entry(v).or_insert(0) += 1;
        });
    }

    // (block, instruction index) of the sret call that fills each hidden slot.
    let mut sret_sites: FxHashMap<u32, (usize, usize)> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Instruction::Call { info, .. } = inst {
                if info.is_sret {
                    if let Some(OperandValue(v)) = info.args.first().map(operand_value) {
                        sret_sites.insert(v, (bi, ii));
                    }
                }
            }
        }
    }
    if sret_sites.is_empty() {
        return forward_dominated_copies(func);
    }

    // Plan the edits: (block, index of the copy, index of the sret call,
    // destination alloca value).
    let mut removals: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    let mut retargets: Vec<(usize, usize, u32)> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::Memcpy { dest, src, size } = inst else {
                continue;
            };
            let (d, s) = (dest.0, src.0);

            if d == s {
                continue;
            }
            if sizes.get(&d) != Some(size) || sizes.get(&s) != Some(size) {
                continue;
            }
            // The hidden slot is mentioned exactly twice — the call's pointer
            // argument and this copy's source — and written only by the call.
            if mentions.get(&s).copied().unwrap_or(0) != 2 {
                continue;
            }
            if writes.get(&s).copied().unwrap_or(0) != 1 {
                continue;
            }
            let Some(&(cb, ci)) = sret_sites.get(&s) else {
                continue;
            };
            if cb != bi || ci >= ii {
                continue;
            }
            retargets.push((cb, ci, d));
            removals.entry(bi).or_default().push(ii);
        }
    }
    if retargets.is_empty() {
        return forward_dominated_copies(func);
    }

    for (bi, ci, d) in retargets {
        if let Instruction::Call { info, .. } = &mut func.blocks[bi].instructions[ci] {
            info.args[0] = crate::ir::reexports::Operand::Value(crate::ir::reexports::Value(d));
        }
    }
    let mut removed = 0;
    for (bi, indices) in removals {
        let block = &mut func.blocks[bi];
        let spans_parallel = block.source_spans.len() == block.instructions.len();
        for ii in indices.into_iter().rev() {
            block.instructions.remove(ii);
            if spans_parallel {
                block.source_spans.remove(ii);
            }
            removed += 1;
        }
    }
    removed + forward_dominated_copies(func)
}

/// Pointer derivation map: value -> (alloca root, constant offset path).
///
/// `Some(path)` means the value is derived from `root` through a chain of
/// constant-offset `GetElementPtr`s and `Copy`s; `None` means it is derived
/// from `root` but by a path this pass cannot enumerate. Values absent from
/// the map are unrelated to any alloca, which is the conservative answer the
/// walk below needs.
fn deriv_map(func: &IrFunction) -> FxHashMap<u32, (u32, Option<Vec<i64>>)> {
    let mut d: FxHashMap<u32, (u32, Option<Vec<i64>>)> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                d.insert(dest.0, (dest.0, Some(Vec::new())));
            }
        }
    }
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                let derived = match inst {
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } => d.get(&base.0).map(|(root, path)| {
                        let next = match (path, offset) {
                            (Some(p), Operand::Const(c)) => c.to_i64().map(|o| {
                                let mut p = p.clone();
                                p.push(o);
                                p
                            }),
                            _ => None,
                        };
                        (dest.0, (*root, next))
                    }),
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(src),
                    } => d
                        .get(&src.0)
                        .map(|(root, path)| (dest.0, (*root, path.clone()))),
                    _ => None,
                };
                if let Some((v, val)) = derived {
                    if d.get(&v) != Some(&val) {
                        d.insert(v, val);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            return d;
        }
    }
}

/// What a use of a copied object is.
#[derive(Clone)]
enum ObjectUse {
    /// A `Load` through a constant offset path: re-rootable at the source.
    Readable(Vec<i64>),
    /// The object read as another whole-object copy's source: the bytes must
    /// be present, but nothing writes them.
    Pinned,
    /// Any other observation of the object (a call argument, a store through
    /// it, a return): the copy's bytes must stay where they are, and the root
    /// can never serve as a forwarding source.
    Opaque,
}

/// One entry in an object's event log.
enum Event {
    /// A whole-object copy into the object: every byte is redefined.
    Reinit(usize, usize),
    /// A store into some of the object's bytes, in `(block, instruction)`.
    /// It writes, it never reads.
    Write(usize, usize),
    /// Anything else that can touch the object and might read it as well: a
    /// call argument, an inline-asm operand, a pointer used as data.
    Clobber(usize, usize),
    /// A read of the object at one instruction, and how it observes it.
    Read(usize, usize, ObjectUse),
}

impl Event {
    fn block(&self) -> usize {
        match self {
            Event::Reinit(bi, _) | Event::Read(bi, _, _) | Event::Clobber(bi, _) => *bi,
            Event::Write(bi, _) => *bi,
        }
    }
}

/// A whole-object copy and the object it fills.
struct CopySite {
    block: usize,
    inst: usize,
    source_root: u32,
    /// Uses in this block that this copy feeds, in program order.
    owned: Vec<(usize, ObjectUse)>,
    deletable: bool,
}

/// Forward aggregate copies into the reads they feed (see the module docs).
///
/// The decision is per copy site, because one alloca is routinely reused by
/// several copies in different blocks (a per-iteration temporary): each copy
/// owns the uses between itself and the next write to the object in its block.
/// A copy is removed only when every use it owns is a load it can re-root at
/// the source, and every use of the object is attributable to a copy — so the
/// object keeps holding the right bytes everywhere else.
///
/// Returns the number of copies removed.
fn forward_dominated_copies(func: &mut IrFunction) -> usize {
    use crate::ir::reexports::{IrConst, Value};

    let mut allocas: FxHashMap<u32, (usize, usize, bool)> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                size,
                align,
                volatile,
                semantic_volatile,
                ..
            } = inst
            {
                allocas.insert(dest.0, (*size, *align, *volatile || *semantic_volatile));
            }
        }
    }
    if allocas.is_empty() {
        return 0;
    }
    let deriv = deriv_map(func);

    // Event log per object, in program order. A *re-initialization* is a
    // whole-object copy into the object: it redefines every byte. A *clobber*
    // is anything else that can change the bytes: a copy that fills only part
    // of the object, a store through a derived pointer, a call argument, an
    // inline-asm operand. A read is attributable only to the re-initialization
    // that most recently precedes it with no clobber in between; after a
    // partial write the object no longer holds what the copy put there, so its
    // bytes are unaccounted for and the object is left alone.
    let mut events: FxHashMap<u32, Vec<Event>> = FxHashMap::default();
    let mut rejected: FxHashSet<u32> = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let mut used = Vec::with_capacity(8);
            inst.for_each_used_value(|v| used.push(v));
            // A `Memcpy` names its destination twice when it is a self-copy;
            // it is one access, judged once.
            used.sort_unstable();
            used.dedup();
            for v in used {
                let Some((root, path)) = deriv.get(&v) else {
                    continue;
                };
                let whole = matches!(path, Some(p) if p.is_empty())
                    && allocas.get(root).is_some_and(|&(size, _, _)| {
                        matches!(inst, Instruction::Memcpy { size: m, .. } if *m == size)
                    });
                let event = match inst {
                    // Path definitions create no new observation.
                    Instruction::GetElementPtr { base, .. } if base.0 == v => continue,
                    Instruction::Copy {
                        src: Operand::Value(src),
                        ..
                    } if src.0 == v => continue,
                    // Copying into the object redefines it; copying it out
                    // only reads it.
                    Instruction::Memcpy { dest, src, .. } if dest.0 == v => {
                        if src.0 == v {
                            // Copying an object onto itself redefines nothing:
                            // it reads the bytes that are already there.
                            Event::Read(bi, ii, ObjectUse::Pinned)
                        } else if whole {
                            Event::Reinit(bi, ii)
                        } else {
                            Event::Write(bi, ii)
                        }
                    }
                    Instruction::Memcpy { .. } => Event::Read(bi, ii, ObjectUse::Pinned),
                    // A load the pass cannot re-root reads the object just as
                    // much as one it can: it must be owned by the copy too.
                    Instruction::Load { ptr, .. } if ptr.0 == v => match path {
                        Some(p) => Event::Read(bi, ii, ObjectUse::Readable(p.clone())),
                        None => Event::Read(bi, ii, ObjectUse::Opaque),
                    },
                    // A store through a derived pointer only defines bytes;
                    // the value it stores is computed before it runs, so the
                    // store itself observes nothing.
                    Instruction::Store { ptr, .. } if ptr.0 == v => Event::Write(bi, ii),
                    _ => Event::Clobber(bi, ii),
                };
                events.entry(*root).or_default().push(event);
            }
        }
        block.terminator.for_each_used_value(|v| {
            if let Some((root, _)) = deriv.get(&v) {
                rejected.insert(*root);
            }
        });
    }

    // Replay each object's log. Every read must be attributable to the copy
    // that currently defines the object, and a clobber leaves it undefined, so
    // an object whose bytes are ever read while undefined is left alone.
    // Every event that can change an object's bytes, in program order, with a
    // flag for the whole-object copies among them (the only writes a snapshot
    // can be checked against by position alone). The forwarding rule below
    // needs the full list to prove that a copied source still holds the bytes
    // each re-rooted read consumes.
    let mut write_positions: FxHashMap<u32, Vec<(usize, usize, bool)>> = FxHashMap::default();
    let mut owned_by: FxHashMap<(usize, usize), Vec<(usize, ObjectUse)>> = FxHashMap::default();
    let mut attributable: FxHashSet<u32> = FxHashSet::default();
    // A root observed through anything but a whole-object copy may be written
    // at any time (a store through a derived pointer, a call argument — libc
    // `memset` is a plain call — an inline-asm operand), so it can never serve
    // as the *source* of a forwarded read.
    let mut opaque_roots: FxHashSet<u32> = FxHashSet::default();
    for (&root, log) in &events {
        let mut current: Option<(usize, usize)> = None;
        let mut complete = true;
        let mut here = usize::MAX;
        for event in log {
            // Attribution is block-local: a copy in another block does not
            // define the bytes this block reads, and a back edge can run the
            // reading block first.
            if event.block() != here {
                here = event.block();
                current = None;
            }
            match event {
                Event::Reinit(bi, ii) => {
                    write_positions
                        .entry(root)
                        .or_default()
                        .push((*bi, *ii, true));
                    current = Some((*bi, *ii));
                }
                Event::Write(bi, ii) => {
                    write_positions
                        .entry(root)
                        .or_default()
                        .push((*bi, *ii, false));
                    // Only the bytes it names change; the rest still holds the
                    // copy's bytes, so the copy is not dead merely because a
                    // store follows it -- but no read is attributable to it
                    // any more.
                    current = None;
                }
                Event::Clobber(bi, ii) => {
                    write_positions
                        .entry(root)
                        .or_default()
                        .push((*bi, *ii, false));
                    // The object may be written here, and it may be read.
                    match current {
                        Some(owner) => {
                            // Reads the bytes the copy put there, in a way that
                            // cannot be re-rooted at the source.
                            owned_by
                                .entry(owner)
                                .or_default()
                                .push((*ii, ObjectUse::Opaque));
                        }
                        // Reached with no copy defining the object: those bytes
                        // come from somewhere this pass does not model, so the
                        // object cannot be reasoned about at all.
                        None => complete = false,
                    }
                    opaque_roots.insert(root);
                    current = None;
                }
                Event::Read(_, ui, kind) => {
                    if matches!(kind, ObjectUse::Opaque) {
                        opaque_roots.insert(root);
                    }
                    match current {
                        Some(owner) => owned_by.entry(owner).or_default().push((*ui, kind.clone())),
                        None => complete = false,
                    }
                }
            }
        }
        if complete {
            attributable.insert(root);
        }
    }

    let mut sites: Vec<CopySite> = Vec::new();
    for (&root, ws) in &write_positions {
        if rejected.contains(&root) || !attributable.contains(&root) {
            continue;
        }
        let Some(&(size, align, volat)) = allocas.get(&root) else {
            continue;
        };
        if volat {
            continue;
        }
        for &(bi, ii, whole) in ws {
            // Only a whole-object copy can move the object elsewhere.
            if !whole {
                continue;
            }
            let Instruction::Memcpy {
                src, size: csize, ..
            } = &func.blocks[bi].instructions[ii]
            else {
                continue;
            };
            if *csize != size {
                continue;
            }
            let Some((src_root, Some(spath))) = deriv.get(&src.0) else {
                continue;
            };
            if !spath.is_empty() || *src_root == root {
                continue;
            }
            let Some(&(ssize, salign, svolat)) = allocas.get(src_root) else {
                continue;
            };
            if ssize != size || salign != align || svolat {
                continue;
            }
            // The source must stay byte-identical for as long as any forwarded
            // read can run. Whole-object copies are checked below by program
            // order; everything else that can write through a derived pointer
            // marks the source opaque, and an opaque source is never used.
            if opaque_roots.contains(src_root) || rejected.contains(src_root) {
                continue;
            }
            // Every write to the SOURCE precedes this copy on every path, so
            // the bytes it copies are still there when the copy runs.
            let src_ok = write_positions.get(src_root).is_none_or(|sw| {
                sw.iter()
                    .filter(|&&(_, _, whole)| whole)
                    .all(|&(wb, wi, _)| {
                        if wb == bi {
                            wi < ii
                        } else {
                            precedes(func, wb, bi)
                        }
                    })
            });
            if !src_ok {
                continue;
            }
            let owned: Vec<(usize, ObjectUse)> = owned_by
                .get(&(bi, ii))
                .map(|v| v.iter().map(|(k, u)| (*k, u.clone())).collect())
                .unwrap_or_default();
            // Every read the copy feeds must be a load this pass can re-root,
            // and the source must still hold the bytes it copied by the time
            // the last of those loads runs.
            let deletable = owned
                .iter()
                .all(|(_, kind)| matches!(kind, ObjectUse::Readable(_)))
                && source_holds_copied_bytes(*src_root, bi, ii, &owned, &write_positions);

            sites.push(CopySite {
                block: bi,
                inst: ii,
                source_root: *src_root,
                owned,
                deletable,
            });
        }
    }
    if sites.is_empty() {
        return 0;
    }

    let mut redirect: FxHashMap<(usize, usize), (u32, Vec<i64>)> = FxHashMap::default();
    let mut removed_sites: FxHashSet<(usize, usize)> = FxHashSet::default();
    for site in &sites {
        if !site.deletable {
            continue;
        }
        removed_sites.insert((site.block, site.inst));
        for (ui, kind) in &site.owned {
            if let ObjectUse::Readable(offsets) = kind {
                redirect.insert((site.block, *ui), (site.source_root, offsets.clone()));
            }
        }
    }

    if removed_sites.is_empty() {
        return 0;
    }
    let mut removed = 0usize;
    for bi in 0..func.blocks.len() {
        let block = &mut func.blocks[bi];
        let spans_parallel = block.source_spans.len() == block.instructions.len();
        let old = std::mem::take(&mut block.instructions);
        let old_spans = std::mem::take(&mut block.source_spans);
        let mut out: Vec<Instruction> = Vec::with_capacity(old.len());
        let mut spans: Vec<crate::common::source::Span> = Vec::with_capacity(old.len());
        for (ii, inst) in old.into_iter().enumerate() {
            if removed_sites.contains(&(bi, ii)) {
                removed += 1;
                continue;
            }
            let Some((src_root, offsets)) = redirect.get(&(bi, ii)) else {
                out.push(inst);
                if spans_parallel {
                    spans.push(old_spans[ii]);
                }
                continue;
            };
            let mut base = Value(*src_root);
            for off in offsets {
                let dest = Value(func.next_value_id);
                func.next_value_id += 1;
                out.push(Instruction::GetElementPtr {
                    dest,
                    base,
                    offset: Operand::Const(IrConst::I64(*off)),
                    ty: crate::common::types::IrType::I64,
                });
                if spans_parallel {
                    spans.push(old_spans[ii]);
                }
                base = dest;
            }
            let mut inst = inst;
            if let Instruction::Load { ptr, .. } = &mut inst {
                *ptr = base;
            }
            out.push(inst);
            if spans_parallel {
                spans.push(old_spans[ii]);
            }
        }
        func.blocks[bi].instructions = out;
        if spans_parallel {
            func.blocks[bi].source_spans = spans;
        }
    }
    removed
}

/// Do the bytes the copy at `(block, copy)` took from `source` still hold when
/// every read re-rooted at `source` runs?
///
/// The reads all lie in the copy's own block, after it (attribution is
/// block-local), so only a write to the source inside that window can make a
/// re-rooted read observe something the copy did not see. `positions` lists
/// every event that can write the source, in program order, so the first one
/// after the copy decides: it must not be in the copy's block (an unrelated
/// block cannot run between two instructions of this one) or it must lie
/// after the last forwarded read.
///
/// This is the rule the first version of the pass was missing: it compared the
/// copy only against *whole-object* copies into the source, so a field store,
/// a partial copy or an aliased store between the copy and the read was
/// invisible and the read came back with the newer value.
fn source_holds_copied_bytes(
    source: u32,
    block: usize,
    copy: usize,
    owned: &[(usize, ObjectUse)],
    write_positions: &FxHashMap<u32, Vec<(usize, usize, bool)>>,
) -> bool {
    let Some(&last_read) = owned.iter().map(|(index, _)| index).max() else {
        return true;
    };
    let Some(positions) = write_positions.get(&source) else {
        return true;
    };
    let next = positions.partition_point(|&(b, i, _)| (b, i) <= (block, copy));
    match positions.get(next) {
        Some(&(b, i, _)) => b != block || i > last_read,
        None => true,
    }
}

/// Does block `a` necessarily execute before block `b` on every path to `b`?
///
/// True when `a` strictly dominates `b`. `CfgAnalysis::idom` uses `usize::MAX`
/// for blocks unreachable from entry, so the walk reports false instead of
/// indexing that sentinel — an unreachable block must never license a rewrite.
fn precedes(func: &IrFunction, a: usize, b: usize) -> bool {
    use crate::ir::analysis::CfgAnalysis;
    if a == b || a >= func.blocks.len() || b >= func.blocks.len() {
        return false;
    }
    let cfg = CfgAnalysis::build(func);
    let mut cur = b;
    for _ in 0..cfg.idom.len() {
        if cur >= cfg.idom.len() {
            return false;
        }
        let parent = cfg.idom[cur];
        if parent == usize::MAX || parent == cur {
            return false;
        }
        if parent == a {
            return true;
        }
        cur = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::instruction::CallInfo;
    use crate::ir::reexports::{BasicBlock, BlockId, Operand, Terminator, Value};

    fn alloca(v: u32, size: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(v),
            ty: IrType::Ptr,
            size,
            align: 8,
            volatile: false,
            semantic_volatile: false,
        }
    }

    fn sret_call(dest: u32, slot: u32, arg: u32) -> Instruction {
        Instruction::Call {
            func: "make".into(),
            info: CallInfo {
                dest: Some(Value(dest)),
                args: vec![Operand::Value(Value(slot)), Operand::Value(Value(arg))],
                arg_types: vec![IrType::Ptr, IrType::Ptr],
                return_type: IrType::U64,
                is_variadic: false,
                num_fixed_args: 2,
                struct_arg_sizes: vec![None, None],
                struct_arg_aligns: vec![None, None],
                struct_arg_classes: vec![vec![], vec![]],
                struct_arg_riscv_float_classes: vec![None, None],
                struct_arg_is_f128_sse: vec![false, false],
                ret_is_f128_sse: false,
                is_sret: true,
                is_fastcall: false,
                regparm: None,
                is_pure: false,
                is_const: false,
                ret_eightbyte_classes: vec![],
                assume_library: false,
            },
        }
    }

    /// A non-sret call that reads through `ptr` (an opaque use of the object).
    fn plain_call(dest: u32, ptr: u32) -> Instruction {
        let mut inst = sret_call(dest, ptr, ptr);
        if let Instruction::Call { info, .. } = &mut inst {
            info.is_sret = false;
            info.args = vec![Operand::Value(Value(ptr))];
            info.arg_types = vec![IrType::Ptr];
            info.num_fixed_args = 1;
            info.struct_arg_sizes = vec![None];
            info.struct_arg_aligns = vec![None];
            info.struct_arg_classes = vec![vec![]];
            info.struct_arg_riscv_float_classes = vec![None];
            info.struct_arg_is_f128_sse = vec![false];
        }
        inst
    }

    fn func_with(instructions: Vec<Instruction>) -> IrFunction {
        let mut func = IrFunction::new("f".into(), IrType::I64, vec![], false);
        func.next_value_id = 100;
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        func
    }

    #[test]
    fn retargets_single_use_hidden_slot() {
        let mut func = func_with(vec![
            alloca(0, 24),
            alloca(1, 24),
            alloca(2, 8),
            sret_call(3, 0, 2),
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(0),
                size: 24,
            },
            plain_call(4, 1),
        ]);
        assert_eq!(run(&mut func), 1);
        let call = &func.blocks[0].instructions[3];
        match call {
            Instruction::Call { info, .. } => {
                assert_eq!(info.args[0], Operand::Value(Value(1)));
            }
            other => panic!("expected the sret call, got {other:?}"),
        }
        assert!(
            !func.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Memcpy { .. }))
        );
    }

    fn multi_block(blocks: Vec<Vec<Instruction>>) -> IrFunction {
        let mut func = IrFunction::new("g".into(), IrType::I64, vec![], false);
        func.next_value_id = 1000;
        for (i, instructions) in blocks.into_iter().enumerate() {
            func.blocks.push(BasicBlock {
                label: BlockId(i as u32),
                instructions,
                terminator: Terminator::Return(None),
                source_spans: vec![],
            });
        }
        func
    }

    #[test]
    fn forwards_copy_whose_reads_follow_in_its_own_block() {
        let mut func = multi_block(vec![vec![
            alloca(0, 24),
            alloca(1, 24),
            alloca(2, 24),
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(2),
                size: 24,
            },
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(0),
                size: 24,
            },
            Instruction::Load {
                volatile: false,
                dest: Value(3),
                ptr: Value(1),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
            },
        ]]);
        assert_eq!(run(&mut func), 1);
        let loads: Vec<&Instruction> = func.blocks[0]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Load { .. }))
            .collect();
        match loads[0] {
            Instruction::Load { ptr, .. } => assert_eq!(ptr.0, 0),
            other => panic!("expected the forwarded load, got {other:?}"),
        }
    }

    fn load(v: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            volatile: false,
            dest: Value(v),
            ptr: Value(ptr),
            ty: IrType::U64,
            seg_override: AddressSpace::Default,
        }
    }

    fn store(ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(90)),
            ptr: Value(ptr),
            ty: IrType::U64,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    #[test]
    fn keeps_copy_when_a_store_writes_the_source_before_the_read() {
        // `d = src; src.a = 5; use(d)` -- re-rooting `use(d)` at `src` would
        // observe the store, so the copy has to stay.
        let mut func = multi_block(vec![vec![
            alloca(0, 16),
            alloca(1, 16),
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(1),
                size: 16,
            },
            store(1),
            load(2, 0),
        ]]);
        assert_eq!(run(&mut func), 0);
        let load = func.blocks[0]
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::Load { .. }))
            .expect("load survives");
        match load {
            Instruction::Load { ptr, .. } => assert_eq!(ptr.0, 0, "read stays on the copy"),
            other => panic!("expected the load, got {other:?}"),
        }
    }

    #[test]
    fn keeps_copy_when_a_partial_copy_writes_the_source_before_the_read() {
        // The same window, closed by a partial Memcpy into the source instead
        // of a store: only the whole-object case used to be modelled.
        let mut func = multi_block(vec![vec![
            alloca(0, 16),
            alloca(1, 16),
            alloca(3, 8),
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(1),
                size: 16,
            },
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(3),
                size: 8,
            },
            load(2, 0),
        ]]);
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn forwards_copy_when_the_source_is_written_only_after_the_read() {
        // `d = src; r = d.a; src.a = 5` -- the store cannot be observed
        // through the re-rooted read, so refusing here would cost the copy
        // for no reason.
        let mut func = multi_block(vec![vec![
            alloca(0, 16),
            alloca(1, 16),
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(1),
                size: 16,
            },
            load(2, 0),
            store(1),
        ]]);
        assert_eq!(run(&mut func), 1);
        let load = func.blocks[0]
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::Load { .. }))
            .expect("load survives");
        match load {
            Instruction::Load { ptr, .. } => assert_eq!(ptr.0, 1, "read re-rooted at the source"),
            other => panic!("expected the forwarded load, got {other:?}"),
        }
    }

    #[test]
    fn forwards_copy_when_the_source_is_written_in_another_block() {
        // A store in block 1 cannot run between two instructions of block 0,
        // so it does not invalidate the window the copy is read in.
        let mut func = multi_block(vec![
            vec![
                alloca(0, 16),
                alloca(1, 16),
                Instruction::Memcpy {
                    dest: Value(0),
                    src: Value(1),
                    size: 16,
                },
                load(2, 0),
            ],
            vec![store(1)],
        ]);
        assert_eq!(run(&mut func), 1);
    }

    #[test]
    fn keeps_copies_when_a_call_writes_the_source() {
        // `copy = *p; memset(p, 0, sizeof *p); use(copy)` — the snapshot must
        // survive a call that writes the source.
        let mut func = multi_block(vec![vec![
            alloca(0, 24),
            alloca(1, 24),
            alloca(2, 24),
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(2),
                size: 24,
            },
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(0),
                size: 24,
            },
            plain_call(3, 2),
            Instruction::Load {
                volatile: false,
                dest: Value(4),
                ptr: Value(1),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
            },
        ]]);
        // One copy still goes: the snapshot itself (`v1 <- v0`) is forwarded to
        // `v0`, which the call does not write, while `v0 <- v2` stays — so the
        // load still observes the pre-memset bytes.
        assert_eq!(run(&mut func), 1);
        let load = func.blocks[0]
            .instructions
            .iter()
            .find(|i| matches!(i, Instruction::Load { .. }))
            .expect("load survives");
        match load {
            Instruction::Load { ptr, .. } => assert_eq!(ptr.0, 0),
            other => panic!("expected the forwarded load, got {other:?}"),
        }
    }

    #[test]
    fn keeps_copy_when_a_call_reads_the_object() {
        let mut func = multi_block(vec![vec![
            alloca(0, 24),
            alloca(1, 24),
            alloca(2, 24),
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(2),
                size: 24,
            },
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(0),
                size: 24,
            },
            plain_call(3, 1),
        ]]);
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn keeps_copy_when_a_read_lives_in_another_block() {
        let mut func = multi_block(vec![
            vec![
                alloca(0, 24),
                alloca(1, 24),
                alloca(2, 24),
                Instruction::Memcpy {
                    dest: Value(0),
                    src: Value(2),
                    size: 24,
                },
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(0),
                    size: 24,
                },
            ],
            vec![Instruction::Load {
                volatile: false,
                dest: Value(3),
                ptr: Value(1),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
            }],
        ]);
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn keeps_copy_when_hidden_slot_is_read_again() {
        let mut func = func_with(vec![
            alloca(0, 24),
            alloca(1, 24),
            alloca(2, 8),
            sret_call(3, 0, 2),
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(0),
                size: 24,
            },
            plain_call(4, 1),
            Instruction::Load {
                volatile: false,
                dest: Value(5),
                ptr: Value(0),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
            },
        ]);
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn keeps_copy_when_sizes_differ() {
        let mut func = func_with(vec![
            alloca(0, 32),
            alloca(1, 24),
            alloca(2, 8),
            sret_call(3, 0, 2),
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(0),
                size: 24,
            },
            plain_call(4, 1),
        ]);
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn keeps_copy_when_the_source_is_overwritten_after_it() {
        let mut func = func_with(vec![
            alloca(0, 24),
            alloca(1, 24),
            alloca(2, 8),
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(0),
                size: 24,
            },
            sret_call(3, 0, 2),
            plain_call(4, 1),
        ]);
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn keeps_copy_when_a_partial_copy_rewrites_the_destination() {
        // `r = d; r.field = x; use(r)` — the copy fills 32 bytes, a later
        // 10-byte copy redefines only the first field, and the read of the
        // second field still needs the bytes the first copy put there.
        let mut func = multi_block(vec![vec![
            alloca(0, 32),
            alloca(1, 32),
            alloca(2, 32),
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(2),
                size: 32,
            },
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(1),
                size: 10,
            },
            Instruction::Load {
                volatile: false,
                dest: Value(3),
                ptr: Value(0),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
            },
        ]]);
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn keeps_copy_when_a_load_cannot_be_rerooted() {
        // A read through a value-offset path is still a read: the copy must
        // stay, even though the load cannot be re-rooted at the source.
        let mut func = multi_block(vec![
            vec![
                alloca(0, 24),
                alloca(1, 24),
                alloca(2, 24),
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(2),
                    size: 24,
                },
            ],
            vec![
                Instruction::GetElementPtr {
                    dest: Value(3),
                    base: Value(1),
                    offset: Operand::Value(Value(4)),
                    ty: IrType::Ptr,
                },
                Instruction::Load {
                    volatile: false,
                    dest: Value(5),
                    ptr: Value(3),
                    ty: IrType::U64,
                    seg_override: AddressSpace::Default,
                },
            ],
        ]);
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn forwards_each_copy_of_a_reused_temporary() {
        // The same temporary is refilled twice in one block: each copy owns
        // the reads up to the next whole-object copy, and both are forwarded.
        let mut func = multi_block(vec![vec![
            alloca(0, 24),
            alloca(1, 24),
            alloca(2, 24),
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(2),
                size: 24,
            },
            Instruction::Load {
                volatile: false,
                dest: Value(3),
                ptr: Value(0),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
            },
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(1),
                size: 24,
            },
            Instruction::Load {
                volatile: false,
                dest: Value(4),
                ptr: Value(0),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
            },
        ]]);
        assert_eq!(run(&mut func), 2);
        let ptrs: Vec<u32> = func.blocks[0]
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Load { ptr, .. } => Some(ptr.0),
                _ => None,
            })
            .collect();
        assert_eq!(ptrs, vec![2, 1]);
    }

    #[test]
    fn keeps_copy_a_partial_write_separates_from_its_read() {
        // A partial copy of the destination before the read leaves the rest of
        // the object undefined, so neither copy site can be forwarded.
        let mut func = multi_block(vec![vec![
            alloca(0, 32),
            alloca(1, 32),
            alloca(2, 32),
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(2),
                size: 32,
            },
            Instruction::Memcpy {
                dest: Value(0),
                src: Value(2),
                size: 32,
            },
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(0),
                size: 8,
            },
            Instruction::Load {
                volatile: false,
                dest: Value(3),
                ptr: Value(1),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
            },
        ]]);
        assert_eq!(run(&mut func), 0);
    }

    #[test]
    fn keeps_copy_when_hidden_slot_also_written_by_a_store() {
        let mut func = func_with(vec![
            alloca(0, 24),
            alloca(1, 24),
            alloca(2, 8),
            sret_call(3, 0, 2),
            Instruction::Store {
                volatile: false,
                val: Operand::Const(crate::ir::reexports::IrConst::I64(1)),
                ptr: Value(0),
                ty: IrType::U64,
                seg_override: AddressSpace::Default,
            },
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(0),
                size: 24,
            },
            plain_call(4, 1),
        ]);
        assert_eq!(run(&mut func), 0);
    }
}
