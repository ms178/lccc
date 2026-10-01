//! Loop induction-variable spill coalescing.
//!
//! lccc's register allocator is a windowed linear scan: it assigns physical
//! registers to values defined *within* the window, and a value whose web spans
//! most of the function keeps its originating stack slot. In the LZ4
//! match-extension loop that makes the induction variable `ip` round-trip
//! through memory on every iteration:
//!
//! ```asm
//! .LBB14:
//!     movq 112(%rsp), %rcx        ; <- reload ip from a stack slot
//!     movzbl (%rcx), %r8d
//!     cmpb (%rbx), %r8b
//!     jne .LBB16
//! .LBB15:
//!     addq $1, %rcx
//!     movq  %rcx, 112(%rsp)       ; <- store ip back to the same slot
//!     addq $1, %rbx
//!     cmpq 56(%rsp), %rcx
//!     jb   .LBB14
//! ```
//!
//! Seven instructions per iteration, three of which touch memory. The store and
//! the reload are a *store-to-load forwarding round trip on the loop's own
//! recurrence*: `ip` cannot advance faster than the ~5-cycle forward, so the IV
//! chain — not the two byte loads — sets the loop's throughput. Every oracle
//! keeps both pointers in registers.
//!
//! Note what is NOT happening here: `%rcx` is already live from the reload
//! through the increment to the store. The allocator never lost the register; it
//! re-synchronised memory and register on every iteration for no reason. So
//! this pass needs no register allocation at all — it hoists the reload out of
//! the loop and sinks the store to the exits:
//!
//! ```asm
//!     movq 112(%rsp), %rcx        ; <- new: once, on the entry edge
//! .LBB14:
//!     movzbl (%rcx), %r8d
//!     cmpb (%rbx), %r8b
//!     jne .Lcciv_st0              ; <- redirected through a store block
//! .LBB15:
//!     addq $1, %rcx
//!     addq $1, %rbx
//!     cmpq 56(%rsp), %rcx
//!     jb   .LBB14
//!     movq  %rcx, 112(%rsp)       ; <- new: the normal exit
//!     ...
//! .Lcciv_st0:                     ; <- new: the early exit
//!     movq  %rcx, 112(%rsp)
//!     jmp .LBB16
//! ```
//!
//! # Why it is safe
//!
//! The rewrite is **transparent to the loop body**: the register holds the same
//! value at every program point inside the loop, before and after. Per iteration
//! the original establishes `R = mem[slot]`, updates it, and writes it back; the
//! rewrite establishes `R` once in the preheader and carries it. Because nothing
//! inside the loop writes the slot the two agree at loop entry, and every update
//! in between is shared. So the obligations are all about the slot, not the
//! register:
//!
//! 1. **The slot's value at loop entry is its value in the preheader.** Nothing
//!    between the two writes it (they are adjacent lines), nothing in the loop
//!    writes it, and its address is never taken so nothing can alias it.
//! 2. **Nothing else in the loop reads the slot.** After the first iteration the
//!    slot holds the *initial* value, so a second reader would see a stale IV.
//! 3. **Every exit writes the slot** with the final register value.
//! 4. **The reload is the header's first instruction and the header has a single
//!    fallthrough entry**, which is what makes the hoist a move across a block
//!    boundary with nothing in between.
//!
//! The one obligation the *register* side imposes is that no instruction in the
//! loop may clobber `R` behind the pass's back. In the original the next
//! iteration's reload would repair any stray write; in the rewrite nothing does.
//! [`is_iv_chain_step`] decides that, and it is deliberately narrow.

use super::super::types::*;
use super::helpers::*;
use super::relay_and_lea::{function_range, split_two_operands};

/// `%rsp`'s family id ([`register_family_fast`] order: rax rcx rdx rbx rsp rbp).
const REG_RSP: RegId = 4;
/// `%rbp`'s family id.
const REG_RBP: RegId = 5;

/// Byte width of the slot traffic this pass rewrites.
///
/// Only a register-for-register `mov` of the same width on both ends is
/// handled, so the hoisted load and the sunk store write exactly the bits the
/// original pair did. A narrower store would zero-extend on the way back and
/// change the value the reload reads; a wider one would introduce bits the
/// original never produced.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotWidth {
    W32,
    W64,
}

impl SlotWidth {
    fn mnemonic(self) -> &'static str {
        match self {
            SlotWidth::W32 => "movl",
            SlotWidth::W64 => "movq",
        }
    }

    /// The AT&T register token for this width, e.g. `%rcx` or `%ecx`.
    fn reg(self, fam: RegId) -> &'static str {
        let row = match self {
            SlotWidth::W32 => 1,
            SlotWidth::W64 => 0,
        };
        REG_NAMES[row][fam as usize]
    }

    fn from_mnemonic(m: &str) -> Option<Self> {
        match m {
            "movl" => Some(SlotWidth::W32),
            "movq" => Some(SlotWidth::W64),
            _ => None,
        }
    }
}

/// A `mov` between a frame slot and a register, parsed.
#[derive(Clone, Copy)]
struct SlotMov<'a> {
    width: SlotWidth,
    /// `disp(%rsp)` / `disp(%rbp)` — the slot, verbatim, so the load and the
    /// store are compared by text rather than by a re-parsed number.
    slot: &'a str,
    reg: RegId,
    /// `true` when the register is the source (the store form).
    is_store: bool,
}

/// The frame-relative memory operand of `op`, if it is exactly one.
///
/// The displacement is optional and may be negative, so the reference is found
/// by locating the parenthesis rather than by matching a prefix. Anything that
/// is not literally `disp(%rsp)` or `disp(%rbp)` is rejected — in particular
/// `(%rax)`, `(%rax,%rbx,4)`, `disp(%rip)` and a segment override — because
/// this pass hoists the access out of the loop, which is only sound for a
/// location the frame fixes.
fn as_frame_slot(op: &str) -> Option<&str> {
    let open = op.find('(')?;
    if !op.ends_with(')') || op[open + 1..].contains('(') {
        return None;
    }
    if &op[open + 1..op.len() - 1] != "%rsp" && &op[open + 1..op.len() - 1] != "%rbp" {
        return None;
    }
    // The displacement must be a plain integer literal, so that two spellings
    // of the same slot can never be compared as different addresses.
    let disp = &op[..open];
    if !disp.is_empty() && disp.parse::<i64>().is_err() {
        return None;
    }
    Some(op)
}

/// Parse `mov{l,q} disp(%rsp|%rbp), %reg` and its mirror.
///
/// The slot must be frame-relative to `%rsp` or `%rbp`. Anything else —
/// `(%rax)`, a segment override, a RIP-relative operand — is rejected rather
/// than guessed at, because this pass hoists the access out of the loop and
/// that is only sound for a location whose address the frame fixes.
fn parse_slot_mov<'a>(t: &'a str) -> Option<SlotMov<'a>> {
    let (mnem, rest) = t.split_once(' ')?;
    let width = SlotWidth::from_mnemonic(mnem)?;
    let (src, dst) = split_two_operands(rest)?;
    let as_reg = |op: &str| -> Option<RegId> {
        let fam = register_family_fast(op);
        if fam == REG_NONE || fam > REG_GP_MAX {
            None
        } else {
            Some(fam)
        }
    };
    if let (Some(slot), Some(reg)) = (as_frame_slot(src), as_reg(dst)) {
        Some(SlotMov {
            width,
            slot,
            reg,
            is_store: false,
        })
    } else if let (Some(reg), Some(slot)) = (as_reg(src), as_frame_slot(dst)) {
        Some(SlotMov {
            width,
            slot,
            reg,
            is_store: true,
        })
    } else {
        None
    }
}

/// True when `t` uses `slot` as a memory operand.
///
/// `112(%rsp)` is the slot; `1112(%rsp)` and `112(%rsp,%rax,1)` are different
/// addresses and must not match, so the reference has to close immediately.
fn mentions_slot(t: &str, slot: &str) -> bool {
    let mut from = 0usize;
    while let Some(rel) = t[from..].find(slot) {
        let at = from + rel;
        // `1112(%rsp)` contains `112(%rsp)` as a substring but names a
        // different address, so a match whose preceding byte continues the
        // displacement literal is not a reference to our slot.
        if at > 0 && matches!(t.as_bytes()[at - 1], b'0'..=b'9' | b'-') {
            from = at + 1;
            continue;
        }
        let after = &t[at + slot.len()..];
        // The reference closes right here: either the operand ends (`)`), the
        // operand list continues (`,`), or the line does.
        if after.is_empty() || after.starts_with(')') || after.starts_with(',') {
            return true;
        }
        from = at + 1;
    }
    false
}

/// True when `t` takes the address of `slot` (`leaq 112(%rsp), %rax`).
///
/// If the address is never formed, no pointer can name the slot, and therefore
/// no store *through* a pointer inside the loop can alias it. That is what makes
/// "nothing in the loop writes the slot" a whole-program statement rather than
/// a statement about the text we happened to scan.
fn takes_slot_address(t: &str, slot: &str) -> bool {
    t.starts_with("lea") && mentions_slot(t, slot)
}

/// True when `t` moves the frame base, which would relocate the slot.
fn perturbs_frame(t: &str) -> bool {
    if t.starts_with("leave")
        || t.starts_with("enter")
        || t.starts_with("push")
        || t.starts_with("pop")
    {
        return true;
    }
    // A write to the frame base relocates the slot. Read from the destination
    // operand rather than from `get_dest_reg`, whose classification is not
    // meaningful for every instruction kind.
    split_two_operands(t.split_once(' ').map_or("", |(_, r)| r))
        .is_some_and(|(_, dst)| dst == "%rsp" || dst == "%rbp")
}

/// Is `t` a step of the induction variable's own chain?
///
/// The pass carries the register across the back edge instead of reloading it,
/// so it must own every write to that family inside the loop. In the original
/// the next iteration's reload silently repaired any stray write; in the rewrite
/// nothing does, so an unrecognised writer is a miscompile.
///
/// Accepted: a binary op whose destination is the family and whose only source
/// is an immediate or the family itself (`addq $1, %rcx`, `leaq 8(%rcx), %rcx`).
/// That is the shape the allocator emits for an IV update. Everything else —
/// `movq %rax, %rcx`, `popq %rcx`, `imulq`, a call, inline asm — returns false.
fn is_iv_chain_step(info: &LineInfo, t: &str, fam: RegId) -> bool {
    // Opaque to any analysis: it may clobber the family for its own reasons.
    if info.kind == LineKind::InlineAsm {
        return false;
    }
    let Some((mnem, rest)) = t.split_once(' ') else {
        return false;
    };
    let wide = match mnem {
        "addq" | "subq" | "leaq" | "andq" | "orrq" => true,
        "addl" | "subl" | "leal" | "andl" | "orl" => false,
        _ => return false,
    };
    let Some((src, dst)) = split_two_operands(rest) else {
        return false;
    };
    let width = if wide { SlotWidth::W64 } else { SlotWidth::W32 };
    if dst != width.reg(fam) {
        return false;
    }
    // The source must be an immediate or the family itself. A memory source is
    // rejected: it could alias the slot this pass stops maintaining.
    if src.starts_with('$') {
        return true;
    }
    if src == SlotWidth::W64.reg(fam) || src == SlotWidth::W32.reg(fam) {
        return true;
    }
    // `disp(%rcx)`: a memory source based on the family itself. It cannot
    // alias the slot, because the slot is `%rsp`/`%rbp`-relative and the family
    // was already established not to be a frame base. An INDEX register is
    // rejected: `disp(%rcx,%rax,8)` reads an arbitrary address.
    src.find('(').is_some_and(|open| {
        src[open + 1..]
            .strip_suffix(')')
            .is_some_and(|inner| !inner.contains(',') && inner.ends_with(SlotWidth::W64.reg(fam)))
    })
}

/// The block whose label is `name`, if the function has one.
fn label_block(
    store: &LineStore,
    infos: &[LineInfo],
    blocks: &[Block],
    name: &str,
) -> Option<usize> {
    blocks.iter().position(|b| {
        b.label
            .and_then(|l| label_name(infos[l].trimmed(store.get(l))))
            .is_some_and(|n| n == name)
    })
}

/// A basic block: a half-open line range plus its resolved successors.
#[derive(Clone)]
struct Block {
    /// Index of the label line, when the block opens with one.
    label: Option<usize>,
    /// Index of the block's first line, including the label when there is one.
    start: usize,
    /// One past the last line, up to but excluding the next block's label.
    end: usize,
    /// Line index of the terminator, when the block ends in one.
    term: Option<usize>,
    /// The block a conditional branch jumps to. Recorded separately from the
    /// fallthrough because "the branch leaves the loop" and "some successor
    /// leaves the loop" are different questions, and answering the second one
    /// for a conditional branch silently turns its own BACK EDGE into an exit.
    branch_target: Option<usize>,
    succs: Vec<usize>,
    /// Set when a branch target could not be resolved inside the function. Any
    /// loop containing such an edge is rejected: its shape is not knowable.
    unresolved_branch: bool,
}

/// The label name on `t`, when the line is one.
fn label_name<'a>(t: &'a str) -> Option<&'a str> {
    let name = t.strip_suffix(':')?;
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    // A local label starts with `.`; a global one may start with a letter or
    // `_`. Anything else (a directive, a quoted string) is not a label.
    if name.starts_with('.')
        || name.starts_with('_')
        || name.starts_with(|c: char| c.is_alphabetic())
    {
        Some(name)
    } else {
        None
    }
}

/// The label name on line `i`, when the line is one.
fn label_at<'a>(store: &'a LineStore, infos: &[LineInfo], i: usize) -> Option<&'a str> {
    if infos[i].is_nop() || infos[i].kind == LineKind::Directive {
        return None;
    }
    label_name(infos[i].trimmed(store.get(i)))
}

/// Split `[lo, hi)` into basic blocks and resolve their successors.
fn build_blocks(store: &LineStore, infos: &[LineInfo], lo: usize, hi: usize) -> Vec<Block> {
    let mut labels: Vec<(usize, &str)> = Vec::new();
    for i in lo..hi {
        if let Some(name) = label_at(store, infos, i) {
            labels.push((i, name));
        }
    }

    let mut starts: Vec<usize> = labels.iter().map(|(i, _)| *i).collect();
    if starts.first() != Some(&lo) {
        starts.insert(0, lo);
    }

    let mut blocks: Vec<Block> = Vec::with_capacity(starts.len());
    for (b, &start) in starts.iter().enumerate() {
        let end = starts.get(b + 1).copied().unwrap_or(hi);
        let label = labels.iter().find(|(i, _)| *i == start).map(|(i, _)| *i);
        let mut term = None;
        for i in start..end {
            if infos[i].is_nop() || infos[i].kind == LineKind::Directive {
                continue;
            }
            term = Some(i);
        }
        blocks.push(Block {
            label,
            start,
            end,
            term,
            branch_target: None,
            succs: Vec::new(),
            unresolved_branch: false,
        });
    }

    for b in 0..blocks.len() {
        let Some(ti) = blocks[b].term else {
            // No terminator at all: an empty or directive-only block falls
            // through.
            if b + 1 < blocks.len() {
                blocks[b].succs.push(b + 1);
            }
            continue;
        };
        let t = infos[ti].trimmed(store.get(ti));
        let resolve = |target: &str, blocks: &mut Vec<Block>| -> Option<usize> {
            let hit = labels
                .iter()
                .position(|(_, n)| *n == target)
                .and_then(|p| blocks.iter().position(|x| x.label == Some(labels[p].0)))?;
            blocks[b].succs.push(hit);
            Some(hit)
        };
        match infos[ti].kind {
            LineKind::Ret => {}
            LineKind::Jmp | LineKind::JmpIndirect => match extract_jump_target(t) {
                Some(target) => {
                    let target = target.to_string();
                    blocks[b].branch_target = resolve(&target, &mut blocks);
                    if blocks[b].branch_target.is_none() {
                        blocks[b].unresolved_branch = true;
                    }
                }
                None => blocks[b].unresolved_branch = true,
            },
            LineKind::CondJmp => {
                let Some(target) = extract_jump_target(t) else {
                    blocks[b].unresolved_branch = true;
                    continue;
                };
                let target = target.to_string();
                blocks[b].branch_target = resolve(&target, &mut blocks);
                if blocks[b].branch_target.is_none() {
                    blocks[b].unresolved_branch = true;
                    continue;
                }
                if b + 1 < blocks.len() {
                    blocks[b].succs.push(b + 1);
                }
            }
            // A call returns; everything else falls through.
            _ => {
                if b + 1 < blocks.len() {
                    blocks[b].succs.push(b + 1);
                }
            }
        }
    }
    blocks
}

/// Dominator sets over the blocks reachable from the entry.
///
/// Returns `dom[b]` = the blocks that dominate `b`, including `b` itself, and
/// the reachability vector. Unreachable blocks get an empty set and are
/// excluded from every loop this pass considers.
fn dominators(blocks: &[Block], entry: usize) -> (Vec<Vec<usize>>, Vec<Vec<usize>>, Vec<bool>) {
    let n = blocks.len();
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (b, blk) in blocks.iter().enumerate() {
        for &s in &blk.succs {
            if s < n && s != b {
                preds[s].push(b);
            }
        }
    }

    // Reverse postorder from the entry (iterative, to avoid deep recursion).
    let mut visited = vec![false; n];
    let mut post: Vec<usize> = Vec::with_capacity(n);
    let mut stack = vec![(entry, 0usize)];
    visited[entry] = true;
    while let Some((b, next)) = stack.pop() {
        if next < blocks[b].succs.len() {
            stack.push((b, next + 1));
            let s = blocks[b].succs[next];
            if !visited[s] {
                visited[s] = true;
                stack.push((s, 0));
            }
        } else {
            post.push(b);
        }
    }
    post.reverse();

    let mut dom: Vec<Vec<usize>> = vec![Vec::new(); n];
    dom[entry] = vec![entry];
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &post {
            if b == entry {
                continue;
            }
            let mut new_set: Option<Vec<usize>> = None;
            for &p in &preds[b] {
                if !visited[p] {
                    continue;
                }
                new_set = Some(match new_set {
                    None => dom[p].clone(),
                    Some(cur) => dom[p].iter().copied().filter(|x| cur.contains(x)).collect(),
                });
            }
            let Some(mut s) = new_set else { continue };
            s.push(b);
            s.sort_unstable();
            s.dedup();
            if s != dom[b] {
                dom[b] = s;
                changed = true;
            }
        }
    }
    (dom, preds, visited)
}

/// A candidate loop, already reduced to block indices.
struct Loop {
    header: usize,
    members: Vec<bool>,
}

impl Loop {
    fn contains(&self, b: usize) -> bool {
        self.members.get(b).copied().unwrap_or(false)
    }
}

/// Run the pass. Returns `true` when the unit was structurally rewritten, which
/// invalidates every cached line index the driver holds.
pub(super) fn promote_loop_iv_spill(store: &mut LineStore, infos: &mut Vec<LineInfo>) -> bool {
    let trace = std::env::var("CCC_TRACE_IVSPILL").is_ok();
    let len = store.len();
    if len < 8 {
        return false;
    }
    let mut i = 0usize;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        let Some((fstart, fend)) = function_range(store, infos, i) else {
            i += 1;
            continue;
        };
        if trace {
            eprintln!("[ivspill] function at {fstart}..{fend}");
        }
        if try_function(store, infos, fstart, fend) {
            return true;
        }
        i = fend.max(i + 1);
    }
    false
}

/// Attempt one rewrite in `[fstart, fend)`; return `true` if the unit changed.
fn try_function(
    store: &mut LineStore,
    infos: &mut Vec<LineInfo>,
    fstart: usize,
    fend: usize,
) -> bool {
    let blocks = build_blocks(store, infos, fstart, fend);
    if blocks.len() < 3 {
        return false;
    }
    let (dom, _preds, reachable) = dominators(&blocks, 0);

    // Back edges: (latch, header) where the header dominates the latch.
    //
    // ALL back edges to one header are collected BEFORE any loop is formed.
    // A per-latch natural loop is not the loop: with two latches, the arm that
    // reaches the OTHER latch is executed on every iteration but is in neither
    // natural loop, so its register writes and — far worse — its EXITS would be
    // invisible to the scan below. A path leaving the loop without passing
    // through a store block is a miscompile, so the loop is the union.
    let mut latches: Vec<Vec<usize>> = vec![Vec::new(); blocks.len()];
    for (latch, blk) in blocks.iter().enumerate() {
        if blk.unresolved_branch || !reachable[latch] {
            continue;
        }
        for &h in &blk.succs {
            if h != latch && reachable[h] && dom[latch].contains(&h) {
                latches[h].push(latch);
            }
        }
    }

    for header in 0..blocks.len() {
        if latches[header].is_empty() {
            continue;
        }
        // Union of the natural loops of every back edge to this header.
        let mut members = vec![false; blocks.len()];
        members[header] = true;
        for &latch in &latches[header] {
            members[latch] = true;
            let mut work = vec![latch];
            while let Some(b) = work.pop() {
                for (p, pb) in blocks.iter().enumerate() {
                    if members[p] || !reachable[p] || !pb.succs.contains(&b) {
                        continue;
                    }
                    members[p] = true;
                    work.push(p);
                }
            }
        }

        // A block the header reaches that is in NO natural loop is still
        // executed during an iteration, and the member scan does not cover it.
        // It needs no special case here, and that is worth stating rather than
        // leaving to inspection. Take such a block `X`, reached from some block
        // of the loop. Either:
        //
        //   * the edge to `X` is that block's TERMINATOR. The exit enumeration
        //     below tests `branch_target` against loop membership, so `X` is
        //     recorded as a jump exit and gets a store block, exactly as LZ4's
        //     `.LBB16` does; or
        //   * the edge to `X` is NOT the terminator -- a conditional early exit
        //     emitted ahead of an unconditional jump. The guard in
        //     `rewrite_loop` refuses that outright, because the exit model is
        //     stated over terminators.
        //
        // Those two cases are exhaustive, so an earlier draft's separate
        // "is `X` private to the loop" reachability analysis was redundant
        // machinery: it could not reject anything the terminator guard had not
        // already rejected, and it had no test that distinguished it from the
        // guard. `a_loop_private_block_is_handled_or_refused` pins both arms.

        let lp = Loop { header, members };
        if rewrite_loop(store, infos, &blocks, &reachable, fstart, fend, &lp) {
            return true;
        }
    }
    false
}

/// Report a rejected candidate when `CCC_TRACE_IVSPILL` is set.
///
/// Every guard in [`rewrite_loop`] is a `return reject(..)` rather than a bare
/// `false`, so "the pass did not fire" is always answerable with a reason
/// instead of a guess. That is what makes the narrowness of this pass auditable:
/// the guards are the specification, and a change in behaviour shows up as a
/// changed reason rather than as silence.
fn reject(why: &str) -> bool {
    if std::env::var("CCC_TRACE_IVSPILL").is_ok() {
        eprintln!("[ivspill] reject: {why}");
    }
    false
}

/// The pass's emission plan for one loop.
struct Plan {
    /// Line after which the exit sequence is emitted. That is the latch's
    /// terminator, so the sequence sits on the path the loop falls out on.
    tail: usize,
    /// Label of the block holding the fallthrough store, when there is one.
    fall_label: Option<String>,
    /// Extra `label / store / jmp target` triples for exits that branch to a
    /// block other than the fallthrough successor.
    extra: Vec<(String, String)>,
}

/// Apply the rewrite to `lp`, or return false having changed nothing.
#[allow(clippy::too_many_arguments)]
fn rewrite_loop(
    store: &mut LineStore,
    infos: &mut Vec<LineInfo>,
    blocks: &[Block],
    reachable: &[bool],
    fstart: usize,
    fend: usize,
    lp: &Loop,
) -> bool {
    let header = &blocks[lp.header];
    let Some(hlabel) = header.label else {
        return reject("header is the function's first block");
    };

    // ── Guard: exactly one predecessor edge from outside the loop, and it must
    // be a physical FALLTHROUGH into the header.
    //
    // The fallthrough requirement is what makes the hoist trivially sound: the
    // load lands immediately before the header's label and the reload it
    // replaces is the header's first instruction, so nothing sits between them
    // and no value can be clobbered in the gap.
    let mut outside_preds: Vec<usize> = (0..blocks.len())
        .filter(|&b| !lp.contains(b) && reachable[b] && blocks[b].succs.contains(&lp.header))
        .collect();
    outside_preds.sort_unstable();
    if outside_preds.len() != 1 {
        return reject("header does not have exactly one outside predecessor");
    }
    let pred = outside_preds[0];
    if blocks[pred].end != hlabel {
        return reject("the entry edge is not a physical fallthrough");
    }
    if let Some(ti) = blocks[pred].term {
        if matches!(
            infos[ti].kind,
            LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret
        ) {
            return reject("the predecessor does not fall through");
        }
    }

    // ── Find the reload: the FIRST real instruction of the header.
    //
    // Anything else first would mean the hoist is not adjacent to the reload it
    // replaces, and the value could be clobbered in between.
    let mut reload: Option<(usize, SlotMov<'_>)> = None;
    // `header.start` IS the label line when the block opens with one, and a
    // label is not an instruction: scanning it would abort the match on the
    // first line of every header.
    let body_start = usize::from(header.label == Some(header.start)) + header.start;
    for i in body_start..header.end {
        if infos[i].is_nop() || infos[i].kind == LineKind::Directive {
            continue;
        }
        match parse_slot_mov(infos[i].trimmed(store.get(i))) {
            Some(m) if !m.is_store => {
                reload = Some((i, m));
                break;
            }
            _ => return reject("the header does not open with the reload"),
        }
    }
    let Some((reload_at, load)) = reload else {
        return reject("no reload in the header");
    };
    let fam = load.reg;
    if fam == REG_RSP || fam == REG_RBP || fam == REG_NONE {
        return reject("the reload targets a frame base");
    }
    // Everything the emit phase needs is copied out here. `load.slot` borrows
    // the store, and holding that borrow across the mutating phase below would
    // make the pass unborrowcheckable; after this block nothing derived from
    // store text is still alive.
    let load_width = load.width;
    let slot: String = load.slot.to_string();

    // ── Find the single matching store inside the loop.
    let mut store_at: Option<usize> = None;
    for (b, blk) in blocks.iter().enumerate() {
        if !lp.contains(b) {
            continue;
        }
        for i in blk.start..blk.end {
            if infos[i].is_nop() || infos[i].kind == LineKind::Directive {
                continue;
            }
            let t = infos[i].trimmed(store.get(i));
            let Some(m) = parse_slot_mov(t) else { continue };
            if !m.is_store {
                continue;
            }
            if m.slot != slot.as_str() {
                continue;
            }
            if m.width != load_width || m.reg != fam {
                return reject("the store width or register does not match the reload");
            }
            if store_at.is_some() {
                return reject("more than one store to the slot inside the loop");
            }
            store_at = Some(i);
        }
    }
    let Some(store_at) = store_at else {
        return reject("no store to the slot inside the loop");
    };

    // ── Enumerate every edge that leaves the loop.
    //
    // The branch and the fallthrough of a conditional terminator are DIFFERENT
    // edges and are tested separately. Reading "some successor leaves the loop"
    // would classify the latch's own back edge as an exit, because the
    // fallthrough beside it does leave.
    let last = (0..blocks.len())
        .filter(|&b| lp.contains(b))
        .max()
        .unwrap_or(lp.header);
    let latch_term = blocks[last].term;
    let falls_out = latch_term.is_none_or(|ti| {
        !matches!(
            infos[ti].kind,
            LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret
        )
    });
    let fall_succ = last + 1;
    let fall_label = if falls_out {
        if fall_succ >= blocks.len() || lp.contains(fall_succ) || !reachable[fall_succ] {
            return reject("the fallthrough exit is not a resolvable block");
        }
        blocks[fall_succ].label.map(|_| ())
    } else {
        None
    };
    let fall_label_name = fall_label.map(|()| {
        blocks[fall_succ]
            .label
            .map(|i| store.get(i).trim_end().trim_end_matches(':').to_string())
            .unwrap_or_default()
    });

    let mut jump_exits: Vec<(usize, String)> = Vec::new();
    for (b, blk) in blocks.iter().enumerate() {
        if !lp.contains(b) || b == last {
            continue;
        }
        let Some(ti) = blk.term else { continue };
        let t = infos[ti].trimmed(store.get(ti));
        match infos[ti].kind {
            LineKind::Jmp => {
                let Some(target) = extract_jump_target(t) else {
                    return reject("unresolvable jmp");
                };
                if blk.succs.first().is_some_and(|&s| !lp.contains(s)) {
                    jump_exits.push((ti, target.to_string()));
                }
            }
            LineKind::CondJmp => {
                let Some(target) = extract_jump_target(t) else {
                    return reject("unresolvable conditional branch");
                };
                if blk.branch_target.is_some_and(|bt| !lp.contains(bt)) {
                    jump_exits.push((ti, target.to_string()));
                }
                if b + 1 < blocks.len() && !lp.contains(b + 1) {
                    // An exit that falls out of a block that is not the latch.
                    // Its store must land after this terminator too, and the
                    // emission point is per-terminator rather than per-loop, so
                    // this shape is left to a later iteration of the driver
                    // rather than half-handled here.
                    return reject("a non-latch block falls out of the loop");
                }
            }
            _ => {}
        }
    }
    if jump_exits.is_empty() && !falls_out {
        return reject("the loop has no exit");
    }

    // ── Guard: the loop must not perturb the frame, alias the slot, or clobber
    // the register family.
    for (b, blk) in blocks.iter().enumerate() {
        if !lp.contains(b) {
            continue;
        }
        for i in blk.start..blk.end {
            if infos[i].is_nop() || infos[i].kind == LineKind::Directive {
                continue;
            }
            let info = &infos[i];
            let t = info.trimmed(store.get(i));

            if takes_slot_address(t, &slot) {
                return reject("the loop takes the slot's address");
            }
            if i != reload_at && i != store_at && mentions_slot(t, &slot) {
                return reject("the slot is read or written elsewhere in the loop");
            }
            if perturbs_frame(t) {
                return reject("the loop moves the frame base");
            }
            // The exit analysis below reasons about each block's TERMINATOR.
            // A branch that is NOT the terminator -- a conditional early exit
            // emitted ahead of an unconditional jump -- leaves the loop on a
            // path no exit site covers, so the slot would not be written on it
            // and the loop would exit carrying the PREVIOUS iteration's value.
            // Refuse rather than reason about a shape the model cannot see.
            if matches!(info.kind, LineKind::Jmp | LineKind::CondJmp) && Some(i) != blk.term {
                let leaves_loop = match extract_jump_target(t) {
                    Some(name) => {
                        label_block(store, infos, blocks, name).is_none_or(|tb| !lp.contains(tb))
                    }
                    None => true,
                };
                if leaves_loop {
                    return reject("a non-terminator branch leaves the loop");
                }
            }
            match info.kind {
                // A call clobbers caller-saved families and can alias through a
                // pointer; a `ret` inside the loop means the slot's final value
                // is unobservable but the exit analysis does not model it.
                LineKind::Ret | LineKind::JmpIndirect | LineKind::InlineAsm | LineKind::Call => {
                    return reject("the loop contains a call, ret or inline asm");
                }
                _ => {}
            }
            if (info.reg_refs & (1u16 << fam)) != 0
                && writes_family_full(info, t, fam)
                && i != reload_at
                && i != store_at
                && !is_iv_chain_step(info, t, fam)
            {
                return reject("the loop clobbers the IV register outside its own chain");
            }
        }
    }

    // ── Guard: the slot's address is never taken anywhere in the function, so
    // no pointer can alias it. Checked function-wide, not just in the loop,
    // because a pointer formed before the loop is enough to break this.
    for i in fstart..fend {
        if infos[i].is_nop() || infos[i].kind == LineKind::Directive {
            continue;
        }
        if takes_slot_address(infos[i].trimmed(store.get(i)), &slot) {
            return reject("the slot's address is taken somewhere in the function");
        }
    }

    // ── Plan the emission.
    let indent = "    ";
    let reg_w = load_width.reg(fam);
    let store_text = format!("{indent}{} {}, {}", load_width.mnemonic(), reg_w, slot);

    // One fresh label per branch that must be redirected. Allocating them
    // BEFORE any rewrite is what keeps a branch and the block serving it from
    // drifting apart.
    let mut plan_labels: Vec<String> = Vec::new();
    for _ in &jump_exits {
        plan_labels.push(unique_exit_label(store, infos, "Lcciv_st"));
    }
    let fall_store_label = if falls_out {
        Some(unique_exit_label(store, infos, "Lcciv_st"))
    } else {
        None
    };

    let mut extra: Vec<(String, String)> = Vec::new();
    let mut branches: Vec<(usize, String)> = Vec::new();
    for ((line, target), label) in jump_exits.iter().zip(plan_labels.iter()) {
        // A branch that already targets the fallthrough successor needs no new
        // block: the store now sits immediately before that block's label, so
        // redirecting the branch to the store's own label routes it through the
        // store and into the original target with no extra jump at all. This is
        // the shape the LZ4 early exit has.
        if falls_out && Some(target.as_str()) == fall_label_name.as_deref() {
            branches.push((*line, fall_store_label.clone().unwrap_or_default()));
            continue;
        }
        branches.push((*line, label.clone()));
        extra.push((label.clone(), target.clone()));
    }

    let tail = latch_term.map(|t| t + 1).unwrap_or(hlabel + 1);
    let plan = Plan {
        tail,
        fall_label: fall_store_label,
        extra,
    };

    // ── Emit. Deletions and branch rewrites move no index, so they go first.
    mark_nop(&mut infos[reload_at]);
    mark_nop(&mut infos[store_at]);
    let mut rewrites: Vec<(usize, String)> = Vec::new();
    for (line, label) in &branches {
        let t = infos[*line].trimmed(store.get(*line));
        let Some(target) = extract_jump_target(t) else {
            return reject("branch rewrite lost its target");
        };
        rewrites.push((*line, t.replacen(target, label, 1)));
    }
    for (line, text) in rewrites {
        store.replace(line, text);
    }

    // The exit sequence, emitted at the latch's fallthrough. Ordering inside it
    // is load-bearing: each extra block ends in an unconditional jump so it can
    // never fall into the next one, and when extra blocks exist the latch's own
    // fallthrough must SKIP them, which costs one unconditional jump on a path
    // that leaves the loop exactly once.
    let mut exit_lines: Vec<String> = Vec::new();
    if !plan.extra.is_empty() && plan.fall_label.is_some() {
        exit_lines.push(format!("{indent}jmp {}", plan.fall_label.as_ref().unwrap()));
    }
    for (label, target) in &plan.extra {
        exit_lines.push(format!("{label}:"));
        exit_lines.push(store_text.clone());
        exit_lines.push(format!("{indent}jmp {target}"));
    }
    if let Some(label) = &plan.fall_label {
        exit_lines.push(format!("{label}:"));
        exit_lines.push(store_text.clone());
    }
    let load_text = format!("{indent}{} {}, {}", load_width.mnemonic(), slot, reg_w);

    // Both vecs grow together: the pass contract is `infos.len() == store.len()`,
    // and `infos` is indexed by the same positions. Highest index first, so
    // every index computed above stays valid.
    let has_exits = !exit_lines.is_empty();
    let mut inserts: Vec<(usize, Vec<String>)> = vec![(plan.tail, exit_lines)];
    if has_exits {
        inserts.push((hlabel, vec![load_text]));
    }
    inserts.sort_by(|a, b| b.0.cmp(&a.0));
    for (at, lines) in inserts {
        for (k, line) in lines.into_iter().enumerate() {
            store.insert_line(at + k, line.clone());
            infos.insert(at + k, classify_line(&line));
        }
    }
    true
}

/// A label name that appears nowhere in the unit.
///
/// The search is over the WHOLE unit rather than the function being rewritten,
/// because a collision is not a diagnostic: the assembler merges the two blocks
/// and keeps only the first predecessor set, which is a miscompile with nothing
/// on stderr. The cost is one linear scan of lines already in cache, paid once
/// per generated label.
///
/// **This is insurance, not a fix for an observed defect.** The original
/// rationale was that lccc inlines, so one unit can hold several textual copies
/// of a callee body and therefore of its `.LBB*` labels. That does not happen:
/// the emitter renumbers block labels per unit, so an inlined body gets fresh
/// numbers. Measured: 0 of 935 corpus assemblies contain a duplicated local
/// label, and a forced `always_inline` function called from two sites -- fully
/// inlined, no `f:` symbol, no remaining `call` -- also emits 8 distinct labels
/// and no duplicate. The scan stays because the consequence of being wrong is a
/// silent miscompile and the cost of being right is nil, but it must not be
/// cited as though the hazard had been observed.
fn unique_exit_label(store: &LineStore, infos: &[LineInfo], base: &str) -> String {
    let mut k = 0usize;
    loop {
        let name = format!(".{base}{k}");
        let taken = (0..store.len()).any(|i| {
            !infos[i].is_nop()
                && infos[i]
                    .trimmed(store.get(i))
                    .strip_suffix(':')
                    .is_some_and(|l| l == name)
        });
        if !taken {
            return name;
        }
        k += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collapse runs of spaces so assertions do not depend on operand padding.
    ///
    /// lccc aligns `movq  %rcx, 112(%rsp)` with two spaces. Asserting on the
    /// one-space spelling matches nothing at all, which turns every "this must
    /// survive" test into a silent `0 == 1` failure that looks like a pass bug.
    fn norm(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut prev_space = false;
        for c in s.chars() {
            let sp = c == ' ' || c == '\t';
            if sp {
                if !prev_space {
                    out.push(' ');
                }
            } else {
                out.push(c);
            }
            prev_space = sp;
        }
        out
    }

    /// Run the pass over `body` and return the resulting text, space-normalised.
    fn run(body: &str) -> String {
        let asm = format!("    .text\n{body}    .cfi_endproc\n");
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        promote_loop_iv_spill(&mut store, &mut infos);
        norm(
            &(0..store.len())
                .filter(|&i| !infos[i].is_nop())
                .map(|i| store.get(i).to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// A minimal well-formed function around a lz4-shaped loop.
    ///
    /// `guard` is spliced into the header and `body` into the latch, so both
    /// land INSIDE the loop. (An earlier version of this fixture appended
    /// `body` after the epilogue's `ret`, where it is dead code, and the four
    /// "an extra instruction inside the loop blocks the rewrite" tests passed
    /// without ever placing an extra instruction inside a loop.)
    fn lz4_like(guard: &str, body: &str) -> String {
        let body = if body.is_empty() {
            String::new()
        } else if body.ends_with('\n') {
            body.to_string()
        } else {
            format!("{body}\n")
        };
        format!(
            concat!(
                "    .globl main\n",
                "    .type main, @function\n",
                "main:\n",
                "    .cfi_startproc\n",
                "    subq $64, %rsp\n",
                "    movq %rdi, 112(%rsp)\n",
                "    movq %rsi, 56(%rsp)\n",
                "    movq %rdx, %rbx\n",
                "    .p2align 3\n",
                ".LBB14:\n",
                "    movq 112(%rsp), %rcx\n",
                "{guard}",
                "    jne .LBB16\n",
                ".LBB15:\n",
                "    addq $1, %rcx\n",
                "    movq  %rcx, 112(%rsp)\n",
                "    addq $1, %rbx\n",
                "    cmpq 56(%rsp), %rcx\n",
                "    jb   .LBB14\n",
                ".LBB16:\n",
                "    movq %rbx, %rsi\n",
                "    movq 112(%rsp), %rax\n",
                "    addq %rsp, %rax\n",
                "    leave\n",
                "    ret\n",
                "{body}",
            ),
            guard = guard,
            body = body
        )
    }

    const GUARD: &str = concat!("    movzbl (%rcx), %r8d\n", "    cmpb (%rbx), %r8b\n",);

    // ── The transformation ────────────────────────────────────────────────

    /// The target shape, end to end.
    ///
    /// The three assertions are the whole contract: the reload is hoisted onto
    /// the entry edge, the in-loop store is GONE, and the store reappears on
    /// the loop's fallthrough. A pass that only removed the store would leave
    /// the loop-carried value in memory and win nothing; one that only hoisted
    /// the load would drop the final value.
    #[test]
    fn the_lz4_loop_loses_its_store_forwarding_round_trip() {
        let out = run(&lz4_like(GUARD, ""));
        assert_eq!(
            out.matches("movq 112(%rsp), %rcx").count(),
            1,
            "exactly one reload survives, and it is the hoisted one:\n{out}"
        );
        assert_eq!(
            out.matches("movq %rcx, 112(%rsp)").count(),
            1,
            "exactly one store survives, and it is on an exit:\n{out}"
        );
        // The hoisted load must be BEFORE the header label, so it runs once.
        let load = out.find("movq 112(%rsp), %rcx").unwrap();
        let header = out.find(".LBB14:").unwrap();
        assert!(load < header, "the load must precede the header:\n{out}");
        // The store must be AFTER the latch's back edge, i.e. outside the loop.
        let store = out.find("movq %rcx, 112(%rsp)").unwrap();
        let back = out.find("jb .LBB14").unwrap();
        assert!(store > back, "the store must sit outside the loop:\n{out}");
        // The back edge itself must be retained; removing it stops iteration.
        assert!(
            out.contains("jb .LBB14"),
            "the back edge must survive:\n{out}"
        );
    }

    /// The conditional early exit must be routed through the store.
    ///
    /// This is the case that separates a correct rewrite from a plausible one.
    /// Leaving `jne .LBB16` alone makes the early exit skip the store, and the
    /// value of `ip` after a short match is then the PREVIOUS one — wrong
    /// output, no crash, no assembler complaint.
    ///
    /// It also pins the shape. The early exit already targets the loop's
    /// fallthrough successor, so the store block is placed immediately before
    /// that block's label and the branch is redirected to the store's own
    /// label. The store then falls straight through into the original target:
    /// the early exit costs ONE added `mov` and no extra jump, on a path that
    /// leaves the loop once.
    #[test]
    fn the_early_exit_is_routed_through_the_store_without_an_extra_jump() {
        let out = run(&lz4_like(GUARD, ""));
        assert!(
            !out.contains("jne .LBB16"),
            "the early exit must not still target the post-loop block:\n{out}"
        );
        let name = out
            .lines()
            .map(str::trim)
            .find_map(|l| l.strip_prefix(".Lcciv_st").map(|_| l.trim_end_matches(':')))
            .expect("an exit store block must be generated");
        assert!(
            out.contains(&format!("jne {name}")),
            "the early exit must branch to the store block {name}:\n{out}"
        );
        // label, store, then the original target by fallthrough.
        let at = out.find(&format!("{name}:")).unwrap();
        let after = &out[at..];
        let rest = after[name.len() + 1..].trim_start();
        assert!(
            rest.starts_with("movq %rcx, 112(%rsp)"),
            "the block must store first:\n{after}"
        );
        let after_store = rest["movq %rcx, 112(%rsp)".len()..].trim_start();
        assert!(
            after_store.starts_with(".LBB16:"),
            "the store must fall through into the original target:\n{after}"
        );
        assert!(
            !after_store.contains("jmp .LBB16"),
            "a jump here would be an avoidable cost on the exit path:\n{after}"
        );
    }

    // ── Rejections: each guard is load-bearing ────────────────────────────

    /// A second READER of the slot inside the loop must block the rewrite.
    ///
    /// After the first iteration the slot holds the value from BEFORE the loop,
    /// not the current `ip`, so a `cmpq 112(%rsp), %rcx` would compare against a
    /// stale value. This is the guard most likely to be dropped by anyone who
    /// thinks in terms of the register rather than the slot.
    #[test]
    fn a_second_reader_of_the_slot_in_the_loop_blocks_the_rewrite() {
        let guard = format!("{GUARD}    cmpq 112(%rsp), %rcx\n");
        let out = run(&lz4_like(&guard, ""));
        assert_eq!(
            out.matches("movq %rcx, 112(%rsp)").count(),
            1,
            "the in-loop store must survive when the slot has another reader:\n{out}"
        );
    }

    /// A second WRITER of the slot inside the loop must block it: two writers
    /// mean the value the reload reads is not the one the hoist captured.
    #[test]
    fn a_second_writer_of_the_slot_in_the_loop_blocks_the_rewrite() {
        let body = "    movq %rax, 112(%rsp)\n";
        let out = run(&lz4_like(GUARD, body));
        assert!(
            out.contains("movq %rcx, 112(%rsp)"),
            "the original in-loop store must survive:\n{out}"
        );
    }

    /// A writer of the IV register that is not part of its own chain must block
    /// the rewrite.
    ///
    /// This is the obligation the register side really imposes. In the original
    /// the next iteration's reload repairs any stray write to `%rcx`; in the
    /// rewrite nothing does, so `movq %rax, %rcx` inside the loop would leave
    /// the IV holding `%rax`'s value forever.
    #[test]
    fn a_foreign_writer_of_the_iv_register_blocks_the_rewrite() {
        let body = "    movq %rax, %rcx\n";
        let out = run(&lz4_like(GUARD, body));
        assert!(
            out.contains("movq %rcx, 112(%rsp)"),
            "the in-loop store must survive:\n{out}"
        );
    }

    /// A chain step IS accepted, so the guard above is not simply "reject every
    /// write". Without this the pass would be correct and useless.
    #[test]
    fn an_iv_chain_step_does_not_block_the_rewrite() {
        let guard = format!("{GUARD}    addq $2, %rcx\n");
        let out = run(&lz4_like(&guard, ""));
        assert_eq!(
            out.matches("movq %rcx, 112(%rsp)").count(),
            1,
            "addq $2, %rcx is an IV step and must not veto:\n{out}"
        );
    }

    /// A CALL inside the loop must block it: it clobbers caller-saved families
    /// and can alias the slot through a pointer.
    #[test]
    fn a_call_inside_the_loop_blocks_the_rewrite() {
        let guard = format!("{GUARD}    call helper\n");
        let out = run(&lz4_like(&guard, ""));
        assert!(
            out.contains("movq %rcx, 112(%rsp)"),
            "must not fire:\n{out}"
        );
    }

    /// Taking the slot's ADDRESS anywhere in the function must block it, even
    /// outside the loop: a pointer formed before the loop is enough for a store
    /// through it to alias the slot.
    #[test]
    fn taking_the_slot_address_anywhere_blocks_the_rewrite() {
        let body = "    leaq 112(%rsp), %r10\n";
        let out = run(&lz4_like(GUARD, body));
        assert!(
            out.contains("movq %rcx, 112(%rsp)"),
            "must not fire:\n{out}"
        );
    }

    /// A branch that is NOT a block's terminator but leaves the loop must be
    /// refused, because the exit model does not cover it.
    ///
    /// ```asm
    /// .LBB14:
    ///     movq 112(%rsp), %rcx      <- reload
    ///     testq %rbx, %rbx
    ///     jne .LBB15a              <- leaves the loop, mid-block
    ///     jmp .LBB15b              <- the block's actual terminator
    /// .LBB15a:                     <- executed during an iteration
    ///     addq $1, %rcx
    ///     jmp .Lout                 <- ... and leaves WITHOUT storing
    /// .LBB15b:
    ///     addq $1, %rcx
    ///     movq  %rcx, 112(%rsp)
    ///     jb .LBB14
    /// ```
    ///
    /// Only the LAST instruction of a block is treated as its terminator, so
    /// `jne .LBB15a` is invisible to the exit enumeration: no store is placed
    /// on that edge and the pass exits carrying the previous iteration's value.
    /// The bug is invisible in the output too — `.LBB15a` keeps its own store in
    /// variants of this shape, so the rewrite looks right and is wrong only when
    /// that block has no store of its own.
    #[test]
    fn a_mid_block_branch_out_of_the_loop_is_refused() {
        let asm = concat!(
            "    .globl main\n",
            "    .type main, @function\n",
            "main:\n",
            "    .cfi_startproc\n",
            "    subq $64, %rsp\n",
            "    movq %rdi, 112(%rsp)\n",
            "    movq %rsi, 56(%rsp)\n",
            "    movq %rdx, %rbx\n",
            ".LBB14:\n",
            "    movq 112(%rsp), %rcx\n",
            "    movzbl (%rcx), %r8d\n",
            "    testq %rbx, %rbx\n",
            "    jne .LBB15a\n",
            "    jmp .LBB15b\n",
            ".LBB15a:\n",
            "    addq $1, %rcx\n",
            "    jmp .Lout\n",
            ".LBB15b:\n",
            "    addq $1, %rcx\n",
            "    movq  %rcx, 112(%rsp)\n",
            "    cmpq 56(%rsp), %rcx\n",
            "    jb .LBB14\n",
            ".Lout:\n",
            "    movq %rbx, %rsi\n",
            "    leave\n",
            "    ret\n",
        );
        let out = run(asm);
        assert!(
            !out.contains(".Lcciv_st"),
            "the pass must refuse a mid-block branch out of the loop:\n{out}"
        );
        assert!(
            out.contains("movq 112(%rsp), %rcx") && out.contains("movq %rcx, 112(%rsp)"),
            "the original reload and store must both survive untouched:\n{out}"
        );
    }

    /// A loop-private block reached from the header is either handled or
    /// refused — never silently ignored.
    ///
    /// ```asm
    /// .LBB14:
    ///     movq 112(%rsp), %rcx    <- reload
    ///     jne .Lmid              <- a loop-private block
    ///     jmp .LBB15
    /// .Lmid:
    ///     addq $1, %rcx           <- IV work ...
    ///     jmp  .Lout              <- ... then leaves, storing nothing
    /// .LBB15:
    ///     addq $1, %rcx
    ///     movq  %rcx, 112(%rsp)
    ///     cmpq 56(%rsp), %rcx
    ///     jb .LBB14
    /// .Lout:
    /// ```
    ///
    /// `.Lmid` is in no natural loop: it is not a latch and reaches no latch,
    /// so the member scan never sees it. `jne .Lmid` is NOT the header's
    /// terminator, so the exit model cannot place a store on it — and the pass
    /// must refuse rather than hoist the reload and delete the latch's store
    /// while `.Lmid` exits having written nothing.
    #[test]
    fn a_loop_private_block_is_handled_or_refused() {
        let asm = concat!(
            "    .globl main\n",
            "    .type main, @function\n",
            "main:\n",
            "    .cfi_startproc\n",
            "    subq $64, %rsp\n",
            "    movq %rdi, 112(%rsp)\n",
            "    movq %rsi, 56(%rsp)\n",
            "    movq %rdx, %rbx\n",
            ".LBB14:\n",
            "    movq 112(%rsp), %rcx\n",
            "    movzbl (%rcx), %r8d\n",
            "    jne .Lmid\n",
            "    jmp .LBB15\n",
            ".Lmid:\n",
            "    addq $1, %rcx\n",
            "    jmp .Lout\n",
            ".LBB15:\n",
            "    addq $1, %rcx\n",
            "    movq  %rcx, 112(%rsp)\n",
            "    cmpq 56(%rsp), %rcx\n",
            "    jb .LBB14\n",
            ".Lout:\n",
            "    movq %rbx, %rsi\n",
            "    leave\n",
            "    ret\n",
        );
        let out = run(asm);
        assert!(
            !out.contains(".Lcciv_st"),
            "a mid-block branch to a loop-private block must block the rewrite:\n{out}"
        );
        assert!(
            out.contains("movq 112(%rsp), %rcx") && out.contains("movq %rcx, 112(%rsp)"),
            "the original reload and store must both survive:\n{out}"
        );
    }

    /// The same private-block SHAPE with the branch promoted to the header's
    /// terminator, which is the other arm: now the exit model does cover it and
    /// the pass must handle it.
    ///
    /// This is what keeps the previous test from being satisfied by refusing
    /// every header jump out of the loop — which would also be "correct" and
    /// would lose the entire optimization.
    #[test]
    fn a_terminator_branch_to_a_loop_private_block_gets_a_store() {
        let asm = concat!(
            "    .globl main\n",
            "    .type main, @function\n",
            "main:\n",
            "    .cfi_startproc\n",
            "    subq $64, %rsp\n",
            "    movq %rdi, 112(%rsp)\n",
            "    movq %rsi, 56(%rsp)\n",
            "    movq %rdx, %rbx\n",
            ".LBB14:\n",
            "    movq 112(%rsp), %rcx\n",
            "    movzbl (%rcx), %r8d\n",
            "    jne .Lmid\n",
            ".LBB15:\n",
            "    addq $1, %rcx\n",
            "    movq  %rcx, 112(%rsp)\n",
            "    cmpq 56(%rsp), %rcx\n",
            "    jb .LBB14\n",
            ".Lmid:\n",
            "    addq $1, %rcx\n",
            "    jmp .Lout\n",
            ".Lout:\n",
            "    movq %rbx, %rsi\n",
            "    leave\n",
            "    ret\n",
        );
        let out = run(asm);
        assert!(
            out.contains(".Lcciv_st"),
            "a terminator branch out of the loop must be handled, not refused:\n{out}"
        );
        assert_eq!(
            out.matches("movq 112(%rsp), %rcx").count(),
            1,
            "the reload must be hoisted exactly once:\n{out}"
        );
    }

    /// The complementary case, and the one LZ4 depends on: a header jump to a
    /// block that other code in the function also reaches IS an exit, and the
    /// pass must handle it rather than refuse.
    ///
    /// Without this half the previous test could be satisfied by simply
    /// refusing every header jump out of the loop, which would also be
    /// "correct" and would lose the entire optimization.
    #[test]
    fn a_shared_continuation_reached_from_the_header_is_treated_as_an_exit() {
        let asm = concat!(
            "    .globl main\n",
            "    .type main, @function\n",
            "main:\n",
            "    .cfi_startproc\n",
            "    subq $64, %rsp\n",
            "    movq %rdi, 112(%rsp)\n",
            "    movq %rsi, 56(%rsp)\n",
            "    movq %rdx, %rbx\n",
            "    cmpq 56(%rsp), %rbx\n", // <- also reaches .LBB16
            "    jae .LBB16\n",
            ".LBB14:\n",
            "    movq 112(%rsp), %rcx\n",
            "    movzbl (%rcx), %r8d\n",
            "    jne .LBB16\n",
            ".LBB15:\n",
            "    addq $1, %rcx\n",
            "    movq  %rcx, 112(%rsp)\n",
            "    cmpq 56(%rsp), %rcx\n",
            "    jb .LBB14\n",
            ".LBB16:\n",
            "    movq %rbx, %rsi\n",
            "    leave\n",
            "    ret\n",
        );
        let out = run(asm);
        assert!(
            out.contains(".Lcciv_st"),
            "a shared continuation must be handled as an exit, not refused:\n{out}"
        );
        assert_eq!(
            out.matches("movq 112(%rsp), %rcx").count(),
            1,
            "the reload must be hoisted exactly once:\n{out}"
        );
    }

    /// A `push`/`pop` or a write to the frame base inside the loop must block
    /// it: the slot's address would move under the hoisted load.
    #[test]
    fn a_frame_perturbation_inside_the_loop_blocks_the_rewrite() {
        for extra in ["    pushq %rax\n", "    addq $16, %rsp\n", "    leave\n"] {
            let guard = format!("{GUARD}{extra}");
            let out = run(&lz4_like(&guard, ""));
            assert!(
                out.contains("movq %rcx, 112(%rsp)"),
                "{extra:?} must block the rewrite:\n{out}"
            );
        }
    }

    /// The reload must be the header's FIRST instruction. If anything precedes
    /// it, the hoist is no longer adjacent to it and a value could be clobbered
    /// in the gap.
    #[test]
    fn a_header_that_does_not_open_with_the_reload_is_left_alone() {
        let asm = concat!(
            "    .globl main\n",
            "    .type main, @function\n",
            "main:\n",
            "    .cfi_startproc\n",
            "    subq $64, %rsp\n",
            "    movq %rdi, 112(%rsp)\n",
            "    movq %rsi, 56(%rsp)\n",
            ".LBB14:\n",
            "    movq %rbx, %rax\n", // <-- stands between the hoist and the reload
            "    movq 112(%rsp), %rcx\n",
            "    addq $1, %rcx\n",
            "    movq  %rcx, 112(%rsp)\n",
            "    cmpq 56(%rsp), %rcx\n",
            "    jb   .LBB14\n",
            "    leave\n",
            "    ret\n",
        );
        let out = run(asm);
        assert!(
            out.contains("movq 112(%rsp), %rcx"),
            "the in-loop reload must survive:\n{out}"
        );
    }

    /// Two outside predecessors mean the header is entered from two places, so
    /// "the value in the preheader" is not a single thing. The pass refuses
    /// rather than inventing a preheader block.
    #[test]
    fn a_header_with_two_outside_predecessors_is_left_alone() {
        let asm = concat!(
            "    .globl main\n",
            "    .type main, @function\n",
            "main:\n",
            "    .cfi_startproc\n",
            "    subq $64, %rsp\n",
            "    movq %rdi, 112(%rsp)\n",
            "    movq %rsi, 56(%rsp)\n",
            "    testq %rdi, %rdi\n",
            "    je .LBB14\n",
            "    movq %rsi, 112(%rsp)\n",
            "    jmp .LBB14\n",
            ".LBB14:\n",
            "    movq 112(%rsp), %rcx\n",
            "    addq $1, %rcx\n",
            "    movq  %rcx, 112(%rsp)\n",
            "    cmpq 56(%rsp), %rcx\n",
            "    jb   .LBB14\n",
            "    leave\n",
            "    ret\n",
        );
        let out = run(asm);
        assert!(
            out.contains("movq 112(%rsp), %rcx"),
            "the in-loop reload must survive:\n{out}"
        );
    }

    /// A store whose width or register differs from the reload is not the same
    /// slot traffic and must not be folded into it.
    #[test]
    fn a_store_of_a_different_width_is_left_alone() {
        let asm = concat!(
            "    .globl main\n",
            "    .type main, @function\n",
            "main:\n",
            "    .cfi_startproc\n",
            "    subq $64, %rsp\n",
            "    movq %rdi, 112(%rsp)\n",
            "    movq %rsi, 56(%rsp)\n",
            ".LBB14:\n",
            "    movq 112(%rsp), %rcx\n",
            "    addq $1, %rcx\n",
            "    movl  %ecx, 112(%rsp)\n", // <-- 32-bit store, 64-bit load
            "    cmpq 56(%rsp), %rcx\n",
            "    jb   .LBB14\n",
            "    leave\n",
            "    ret\n",
        );
        let out = run(asm);
        assert!(
            out.contains("movl %ecx, 112(%rsp)"),
            "a mismatched store must survive untouched:\n{out}"
        );
    }

    /// A reload whose source register is neither `%rsp` nor `%rbp` is not a
    /// frame slot, and hoisting it out of the loop would be unsound.
    #[test]
    fn a_non_frame_relative_reload_is_left_alone() {
        let asm = concat!(
            "    .globl main\n",
            "    .type main, @function\n",
            "main:\n",
            "    .cfi_startproc\n",
            "    movq %rdi, %rax\n",
            "    movq %rsi, 56(%rsp)\n",
            ".LBB14:\n",
            "    movq (%rax), %rcx\n",
            "    addq $1, %rcx\n",
            "    movq  %rcx, (%rax)\n",
            "    cmpq 56(%rsp), %rcx\n",
            "    jb   .LBB14\n",
            "    ret\n",
        );
        let out = run(asm);
        assert!(out.contains("movq (%rax), %rcx"), "must not fire:\n{out}");
    }

    // ── Label uniqueness, under inlining ─────────────────────────────────

    /// A generated label that collides with one already in the unit must be
    /// disambiguated.
    ///
    /// lccc inlines, so one assembly unit can hold several textual copies of the
    /// same callee body and therefore of the same `.LBB*` labels. Two blocks
    /// sharing a label are silently merged by the assembler, and the merged
    /// block keeps only the first predecessor set — which is a miscompile with
    /// no diagnostic anywhere.
    #[test]
    fn a_colliding_exit_label_is_disambiguated() {
        let asm = format!(
            concat!(
                "    .globl main\n",
                "    .type main, @function\n",
                "main:\n",
                "    .cfi_startproc\n",
                "    subq $64, %rsp\n",
                "    movq %rdi, 112(%rsp)\n",
                "    movq %rsi, 56(%rsp)\n",
                ".Lcciv_st0:\n", // <-- already taken
                "    nop\n",
                ".LBB14:\n",
                "    movq 112(%rsp), %rcx\n",
                "{guard}",
                "    jne .LBB16\n",
                ".LBB15:\n",
                "    addq $1, %rcx\n",
                "    movq  %rcx, 112(%rsp)\n",
                "    addq $1, %rbx\n",
                "    cmpq 56(%rsp), %rcx\n",
                "    jb   .LBB14\n",
                ".LBB16:\n",
                "    movq %rbx, %rsi\n",
                "    leave\n",
                "    ret\n",
            ),
            guard = GUARD
        );
        let out = run(&asm);
        // The pre-existing label is untouched: still present, still alone, and
        // still the one the generated code must NOT have reused.
        assert_eq!(
            out.matches(".Lcciv_st0:").count(),
            1,
            "the original label survives exactly once:\n{out}"
        );
        // The generated block took a free suffix, and the branch names THAT one.
        let generated = out
            .lines()
            .map(str::trim)
            .filter_map(|l| l.strip_prefix(".Lcciv_st").map(|_| l.trim_end_matches(':')))
            .find(|n| *n != ".Lcciv_st0")
            .unwrap_or_else(|| panic!("no disambiguated label was generated:\n{out}"));
        assert_ne!(generated, ".Lcciv_st0", "must not reuse the taken name");
        assert!(
            out.contains(&format!("jne {generated}")),
            "the early exit must branch to the NEW label {generated}:\n{out}"
        );
    }

    /// Two functions in one unit carrying the SAME `.LBB*` labels must not have
    /// their generated labels merged.
    ///
    /// The emitter does not actually produce this shape today — it renumbers
    /// per unit, and 0 of 935 corpus assemblies contain a duplicated local
    /// label — so this is a property test of the pass's independence from the
    /// emitter's numbering, not a regression against an observed bug. It earns
    /// its place because the failure it describes is silent: two blocks sharing
    /// a label are merged by the assembler, the merged block keeps the first
    /// predecessor set, and nothing is printed anywhere.
    #[test]
    fn an_inlined_duplicate_does_not_merge_generated_labels() {
        let one = concat!(
            "    .type inl, @function\n",
            "inl:\n",
            "    .cfi_startproc\n",
            "    subq $64, %rsp\n",
            "    movq %rdi, 112(%rsp)\n",
            "    movq %rsi, 56(%rsp)\n",
            "    movq %rdx, %rbx\n",
            ".LBB14:\n",
            "    movq 112(%rsp), %rcx\n",
            "    movzbl (%rcx), %r8d\n",
            "    cmpb (%rbx), %r8b\n",
            "    jne .LBB16\n",
            ".LBB15:\n",
            "    addq $1, %rcx\n",
            "    movq  %rcx, 112(%rsp)\n",
            "    addq $1, %rbx\n",
            "    cmpq 56(%rsp), %rcx\n",
            "    jb   .LBB14\n",
            ".LBB16:\n",
            "    movq %rbx, %rsi\n",
            "    leave\n",
            "    ret\n",
            "    .cfi_endproc\n",
        );
        let asm = format!("    .text\n{one}{one}    .cfi_endproc\n");
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        // Two rounds: the driver re-enters after a structural rewrite.
        promote_loop_iv_spill(&mut store, &mut infos);
        promote_loop_iv_spill(&mut store, &mut infos);
        let out = norm(
            &(0..store.len())
                .filter(|&i| !infos[i].is_nop())
                .map(|i| format!("{}\n", store.get(i)))
                .collect::<String>(),
        );
        // Every generated label is unique across the whole unit.
        let mut seen = std::collections::HashMap::new();
        for line in out.lines() {
            if let Some(name) = line.trim().strip_suffix(':') {
                if name.starts_with(".Lcciv_") {
                    *seen.entry(name.to_string()).or_insert(0) += 1;
                }
            }
        }
        assert!(
            seen.values().all(|&c| c == 1),
            "generated labels must be unique across the unit: {seen:?}\n{out}"
        );
        assert!(
            seen.len() >= 2,
            "both copies must have been rewritten: {out}"
        );
    }

    // ── Unit-level parsing ────────────────────────────────────────────────

    /// `112(%rsp)` must not match inside `1112(%rsp)`.
    ///
    /// A substring comparison here would make the "nothing else touches the
    /// slot" guard fire on unrelated code — conservative, so not a
    /// miscompile — but it would also make it fire on the pass's OWN hoisted
    /// load after a later offset shift, and the guard is the specification, so
    /// it is pinned exactly.
    #[test]
    fn slot_matching_respects_the_displacement() {
        assert!(mentions_slot("    movq 112(%rsp), %rcx", "112(%rsp)"));
        assert!(mentions_slot("    movq %rcx, 112(%rsp)", "112(%rsp)"));
        assert!(mentions_slot("    cmpq -8(%rbp), %rax", "-8(%rbp)"));
        assert!(!mentions_slot("    movq 1112(%rsp), %rcx", "112(%rsp)"));
        assert!(!mentions_slot(
            "    movq 112(%rsp,%rax,8), %rcx",
            "112(%rsp)"
        ));
        assert!(!mentions_slot("    movq 112(%rbp), %rcx", "112(%rsp)"));
    }

    /// Only literal frame slots parse. A hoisted access is only sound where the
    /// frame fixes the address.
    #[test]
    fn only_literal_frame_slots_parse() {
        assert!(parse_slot_mov("movq 112(%rsp), %rcx").is_some());
        assert!(parse_slot_mov("movq -8(%rbp), %rcx").is_some());
        assert!(parse_slot_mov("movq (%rsp), %rcx").is_some());
        assert!(parse_slot_mov("movq %rcx, 112(%rsp)").is_some());
        assert!(parse_slot_mov("movq 112(%rsp,%rax,8), %rcx").is_none());
        assert!(parse_slot_mov("movq (%rax), %rcx").is_none());
        assert!(parse_slot_mov("movq 112(%rip), %rcx").is_none());
        assert!(parse_slot_mov("movl 112(%rsp), %ecx").is_some());
        assert!(parse_slot_mov("movb 112(%rsp), %cl").is_none());
        assert!(parse_slot_mov("leaq 112(%rsp), %rax").is_none());
    }

    /// The IV chain step accepts exactly the shapes it claims to.
    #[test]
    fn iv_chain_steps_are_narrow() {
        let info = |t: &str| classify_line(t);
        for t in [
            "    addq $1, %rcx",
            "    addq $8, %rcx",
            "    addl $1, %ecx",
            "    subq $1, %rcx",
            "    leaq 8(%rcx), %rcx",
            "    andq $-1, %rcx",
        ] {
            assert!(
                is_iv_chain_step(&info(t), t.trim(), 1),
                "{t:?} must be a step"
            );
        }
        for t in [
            "    movq %rax, %rcx",      // a foreign source
            "    movq %rax, 112(%rsp)", // not even a register write
            "    imulq %rax, %rcx",     // not on the list at all
            "    popq %rcx",            // implicit write
            "    addq 112(%rsp), %rcx", // memory source: could alias the slot
            "    addq $1, %rax",        // writes another family
            "    addq $1, %cl",         // partial-width write
        ] {
            assert!(
                !is_iv_chain_step(&info(t), t.trim(), 1),
                "{t:?} must be rejected"
            );
        }
    }

    /// The exit label search scans the WHOLE unit, not the current function.
    #[test]
    fn exit_labels_are_unique_across_the_unit() {
        let asm = ".Lcciv_st0:\n    nop\n.Lcciv_st1:\n    nop\n".to_string();
        let store = LineStore::new(asm);
        let infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert_eq!(unique_exit_label(&store, &infos, "Lcciv_st"), ".Lcciv_st2");
    }
}
