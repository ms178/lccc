# Red-team audit — byte_load, MOVBE, and the oracle harness (2026-10-13)

Written by the author of the changes, against the author's own work, under the
standing rule that every claim must survive "would an Intel-grade compiler
engineer approve this, and can I show the data?"  Sections: (1) what was
wrong and how it was caught, (2) choices I stand behind with their data,
(3) choices I would revisit, (4) where I disagree with in-tree decisions,
(5) the checklist.

## 1. What was wrong (found by measurement, not review)

Seven defects in this session's own work, in the order they were caught.  All
were found because a *test that could fail* existed; none by reading the code.

1. **Miscompile: the load adopted the resolved root of the address
   expression.**  The big-endian/narrow rewrite folded the run's
   `GetElementPtr` away whenever the run's lowest byte was at the chain's
   lowest offset — a condition that is *true by construction* for every run —
   and then loaded through the root.  For a run inside an object
   (`framed.body[0]`) that read the object's first byte: `body_le64` returned
   the struct head instead of `body`.  Caught by the benchmark kernel's
   known-answer test (exit 6), **not** by the unit suite, which had no fixture
   at a nonzero offset even though the `field_offset`/`nested_gep` flags
   existed for exactly that shape.  Fixed: the load reads through the run's own
   byte pointer (a zero-displacement GEP folds into the addressing mode, so
   the emitted instruction is unchanged), plus two permanent guards —
   `folds_a_run_at_a_nonzero_offset` (three shapes) and the executable
   footprint invariant `assert_loads_stay_in_footprint`, which asserts the
   pass's actual proof obligation: every wide load reads only bytes the
   original chain read, same root, byte for byte.  The invariant now runs in
   *every* folding test, including the malformed-run ones, so a future
   relaxation cannot silently widen a load past the footprint.
   *Lesson recorded because it will recur:* the fixture flags existed and were
   never used.  A matcher's test matrix must cover its *parameters*, not its
   favourite shape.

2. **A liveness check that refused the pass's entire purpose.**  The first
   version of the MOVBE fusion required the un-swapped loaded value to be dead
   after the swap.  That is backwards: `be64(x)` returning the loaded value is
   a reader by definition, so the check vetoed every real shape (measured: 3
   tests failed, and the compiler emitted no `movbe`).  The correct argument is
   much stronger and simpler: the load and the swap are adjacent, write the
   same register, and nothing can run between them, so the intermediate value
   is unobservable in principle — no liveness judgement is needed at all.  The
   pass now proves exactly that (adjacency via `next_insn`, which refuses a
   label — a possible jump target — and exact width/family match), and the
   test that caught the error is the regression cover.
   *This is the failure mode to watch:* a proof obligation invented locally
   ("the value must be dead") instead of derived from the transformation's
   semantics.

3. **A process-global race in my own unit tests.**  `fuse_load_bswap` first
   read `X86Isa::current().movbe` (the process-global mirror).  Unit tests run
   in parallel threads, so a test that set the global to "no MOVBE" made three
   sibling tests fail intermittently.  Fixed by making the permission a
   *parameter* of the pass: the caller (which owns the TU's options) passes
   `current().movbe`, and tests exercise both arms without touching globals.
   The general principle, worth applying elsewhere: a transformation should
   take its permissions as arguments; globals exist only for code that cannot
   reach the options.

4. **The oracle harness was lying about other architectures** (a defect in
   session work, found while trying to *use* it): `--arch riscv64 --oracles
   gcc16.2` ran the x86-64 channel and stored the output as riscv64; the
   giveaway was that every per-function count matched x86-64 exactly.  Two
   further faults in the same area: the x86 default flags
   (`-march=x86-64-v3`) were handed to the AArch64/RISC-V channels, so every
   cross-architecture oracle failed to compile ("no oracles" in a survey), and
   the per-architecture oracle sets pointed at dead ids *outside* the
   `godbolt.py audit` net, so nothing noticed for as long as the sets existed.
   All three fixed (`_check_arch`, per-architecture `default_flags`,
   audited aliases), 11 tests in `tests/corpus/test_oracle_arch_guard.py`.
   Without this fix, the AArch64 decision below would have been made on x86
   assembly wearing an AArch64 hat — the exact opposite of "no guesswork".

5. **A stale number in a shipping document.**  The byte-load doc carried
   `sip24_final 165 -> 149`.  Re-measured today: 165 -> 165.  The chain in
   `sip24_final` is split across one basic block per `case` label (the asm is
   quoted in the doc), so a within-block matcher cannot fold it — the 149 was
   measured on an older tree where the CFG differed, was quoted onward, and
   would have been indefensible under scrutiny.  Removed and replaced with
   today's numbers plus the reason; the shape is now an explicit open item
   ("folds across a split chain") with its evidence on disk.
   *Rule applied from here on:* a number in a document names the command and
   the tree it came from, and is re-measured when either changes.

6. **The branch deleted twenty-five files it never meant to touch.**  Found
   by the *whole* gate set, not by review: `doc-link-integrity` refused
   `work/regression.json`, and the reason was not the document.  `git diff
   --diff-filter=D <base>..HEAD` listed 21 files under `work/` (saved asm
   dumps, `regression*.json`, the `read64_*` scratch pair) plus `.base_ref`,
   `.claude/settings.local.json` and the two `builtin_cpu_supports_raptor`
   fixtures — every one of them present upstream and absent from the branch,
   because the tree was rebuilt after the session wipe and a `git add -A`
   recorded their absence as a deletion.  Nothing in the change *needed* them
   gone, and a patch that silently deletes twenty-five upstream files is an
   incident, not a patch: restored, except the two `raptor` fixtures, which
   stay deleted because `cpu-supports-runtime-model.md` documents that as the
   deliberate replacement of the old GCC-suppressed comparison.
   *Rule applied from here on:* `git diff --diff-filter=D <base>..HEAD` is part
   of the pre-delivery checklist, and the branch is a superset of the base
   except where a document argues the deletion.

7. **A new pass broke a neighbour's unit test, and the neighbour was right.**
   `dead_writes::tests::dead_scaled_lea_is_deleted` asserts a `bswapl %eax`
   witness.  With the MOVBE fold in the pipeline (phase 7a runs before
   `dead_writes`), the same fixture now compiles to
   `movbel (%rsi,%rbx,4), %eax` — the pair correctly fused — and the assertion
   failed on the spelling, not on the subject.  The fix belongs in the witness,
   not in the pass: the assertion accepts either legal spelling of the swap and
   additionally pins the store that makes the value live.  Caught by
   `cargo-test` inside `ci_local.sh --fast` — i.e. by running every gate, not by
   reading the diff.

## 2. Choices I stand behind, with the data

* **AArch64 enabled for `byte_load`.**  Every AArch64 channel (GCC 16.1, GCC
  trunk, Clang 23.1) widens the same chains: 2/2/2/3/2 for
  `le16/le32/le64/be64/body_le64`, the big-endian form as `ldr` + `rev` — the
  same rewrite this pass performs.  RISC-V is deliberately *not* enabled, and
  the reason is measured, not assumed: 5/12/23/23/23 for both GCC 16.1 and
  Clang 23.1, because bare RISC-V gives no unaligned-access guarantee and a
  wide load could trap into the kernel's handler.  The gate is now a data
  table in the pass documentation rather than a taste judgement.
* **Big-endian runs require the run width to equal the carrier.**  `bswap16`
  followed by a zero-extend is a width-sensitive sequence whose correctness
  would depend on the backend's `bswap` lowering for a narrower operand than
  its destination (`emit_int_bswap_impl` on x86 writes `%eax` unconditionally
  for 32-bit and `rev w0` for AArch64 — correct today, but the pass should not
  be the thing that makes it load-bearing).  The refusal is tested.
* **MOVBE as a peephole, not in isel.**  The offsetting choice is real: isel
  knows the shape earlier and could emit the fused instruction directly.  But
  isel does not know the target permission *at that point* in this codebase,
  the pair is also created by other paths (`__builtin_bswap` on a loaded
  value) and by the byte_load rewrite itself, and the peephole already has a
  per-pass switch, a permission, and liveness machinery.  Measured outcome:
  exact oracle parity (2 instructions, `movbeq (%rdi), %rax`), with a
  one-flag A/B and no isel risk.
* **`X86Isa::movbe` outside the SSE implication chain.**  MOVBE is an integer
  load/store: `-mno-sse` must not remove it, and it must not imply AVX/SSE4.1.
  Encoded as a struct field outside `normalized()`'s chain, with a test that
  asserts exactly that, and driven by `Driver::resolved_movbe`, the same
  answer `__builtin_cpu_supports("movbe")` uses.  `X86Isa::SSE2` (the plain
  x86-64 level) does *not* grant it — measured: `-march=x86-64` keeps the
  `mov`+`bswap` pair.
* **Coalescing the allocator's copy inside the pass.**  I first tried to reuse
  the existing `retarget_producer_into_copy`, which declined: its final guard
  rejects a rewritten instruction that still mentions the original
  destination family, and `movbeq (%rdi), %rax` mentions `%rdi` as the
  addressing base.  That guard is conservative rather than necessary (the
  retarget no longer writes `%rdi`, so `%rdi`'s value as a base stays valid),
  but changing a general-purpose transform's safety margin late in a session
  to save one line in my own file is the wrong trade.  The fold therefore
  coalesces its own copy using the same proof primitive
  (`provably_dead_lv`: exact liveness first, syntactic fallbacks after), which
  is local, covered by three tests (fires; refuses when the intermediate is
  live; refuses a width mismatch), and cannot change behaviour anywhere else.

## 3. Choices I would revisit (stated, not hidden)

* **The RUN of `byte_load` might belong in the IR, not in three backends'
  peepholes.**  The pass itself is target-neutral IR; the *permission* is
  per-target and the *lowering* of `Load`+`Bswap` is per-backend.  That is
  the right split, but it means the AArch64 win depends on AArch64 lowering
  `Bswap` for `u64` (it does: `rev x0, x0`), and the RISC-V target has a
  `Bswap` lowering but no permission.  A follow-up should define the
  condition under which a target may widen an unaligned access *as a target
  property* (AArch64: architectural; RISC-V: subtarget/`Zicclsm`), rather
  than as a list in a pass.
* **The frame findings are bigger than anything this session fixed.**
  AArch64: `int deref(const int*)` is 7 instructions against GCC's 2 (an
  unconditional frame chain, an LR save, and a callee-saved `x19` saved and
  reloaded without ever being read); i686: `int twice(int)` is 7 against 3
  because every function with a parameter gets a callee-saved home plus an
  8-byte outgoing area.  These are 4–7 instructions per function on whole
  architectures.  They were deliberately *not* touched here: they are
  location-assignment/prologue policy, the allocator's own documentation
  records that small edits there move benchmarks in both directions, and a
  rushed change would have been worse than a documented, reproducible gap.
  But they are now the largest measured wins on the board, and a session that
  fixes only these would beat everything this one did.
* **`sip24_final`'s cross-block chain.**  Today it is not folded at all; the
  work item is a path proof (a block whose only predecessor is the chain's
  continuation is still part of the chain), not a matcher relaxation.

## 4. Where I disagree with in-tree decisions

* **`be16` stays byte-wise, and I would not change it in this patch.**
  `(uint16_t)((p[0] << 8) | p[1])` — the `ntohs`/`get_unaligned_be16` shape —
  is 6 instructions in LCCC (2 `movzbl`, `shll`, `orl`, `movzwl`, `ret`)
  against **2 for every oracle channel** — GCC 16.2, Clang 23.1 and ICX all
  answer with one `movbe` (`-O3 -march=x86-64-v3`,
  `deliverables/artifacts/oracle/byte-load-shapes.{json,log}`, refreshed against this tree);
  local GCC 14.2, whose baseline predates MOVBE, reaches 3 through `movzwl` +
  `rolw $8`.  Three times the best channel, on the `ntohs` shape, is the largest
  gap in the family — and the reason to close it with ISel work rather than a
  peephole: the run is 16 bits wide inside a 32-bit carrier, so the lowering
  needs a 16-bit `bswap` (`rolw $8`, whose high half the zero-extending load
  has already cleared) that the `movbe` fold can then consume into `movbew`.  `byte_load` declines that chain on
  purpose (`refuses_a_big_endian_run_narrower_than_its_carrier`): the carrier is
  32-bit, so the reversal needs a 16-bit `bswap` plus a zero-extension — the
  width-sensitive sequence whose little-endian twin produced this session's
  miscompile (§1.1).  Agreed with the refusal; the measured cost is now in the
  byte-load doc and the BE16 item in the follow-up, so the next session can
  close it with an IR-level 16-bit `bswap` lowering and its own differential
  gate instead of a rushed peephole in the last commit of a session.

* **The i686 allocator notes read as if the callee-saved band were the whole
  story.**  `location_alloc/policy.rs` documents band 2 as measured-correct
  for i686 (21 reps, paired A/B).  That may well be right for the *band*, but
  the code above it shows the larger cost is elsewhere: `int twice(int x)`
  compiles to seven instructions because `%ebx` is bought for a parameter
  home plus an 8-byte outgoing area on a function with no calls and nothing
  live across anything.  Band tuning cannot fix that; the parameter-homing
  policy can.  I did not change it, but the notes should not be read as "i686
  codegen is understood".
* **The census ratchet was layout-coupled to the source it audits.**  A
  drift ratchet that fails on `cargo fmt`'s line breaking is measuring
  formatting, and it will be *trusted* precisely because it usually means
  something.  Fixed at the root (table-body extraction with a
  layout-independent pattern) and covered by tests; the same critique applies
  to any other extractor in `scripts/` that regexes Rust source — worth an
  audit pass.
* **`-mno-movbe` had no codegen effect until now.**  The driver modelled the
  flag, `__builtin_cpu_supports("movbe")` reported it, and no instruction ever
  consulted it — a knob that did nothing.  That is the exact class of defect
  the audit brief calls silent: the flag now changes the emitted code, and the
  behaviour matrix (2/4/4/4) is asserted.

## 5. Checklist, per the standing brief

* **Correctness:** 58/58 benchmark programs byte-identical to GCC (normalised
  timings); kernel known-answer digest `7ee1f2c0 3dcf60f284155e00` identical
  under GCC and LCCC on x86-64 *and* i686, in all four pass/movbe cells
  (re-measured this session with `lccc-i686` after `gcc-multilib` was
  installed);
  `folds_a_run_at_a_nonzero_offset` + `assert_loads_stay_in_footprint` guard
  the pass's proof obligation; 17 MOVBE tests + 11 oracle-guard tests + 3
  census-extraction tests + 21 byte_load tests.
  *Scope of the four cells:* for this file the two `movbe_fuse` cells are
  **vacuous** — at `-O2` its KAT byte loads are shared with a later chain, so
  the big-endian chains are never rewritten and neither LCCC nor GCC emits a
  single `movbe` (`grep -c movbe` = 0 in the `-O2 -S` output of both; the
  Callgrind pair agreeing to the instruction is consistent with that).  The
  digest proves the two *`byte_load`* cells differ in work (−52.9 %) while
  agreeing on output.  MOVBE evidence is the shape probes (below and in the
  byte-load doc), the 17 unit tests, and the corpus census: across the 58
  benchmark programs LCCC emits 4 `movbe` instructions (all in
  `ascii_case_fold`) and GCC 1 (in `linux_find_bit`), so neither compiler
  sprays the instruction — and the fold is decided by the resolved target ISA,
  not by the host (same TU, `-O2 -S`: `i686` → 0 `movbe`, `x86-64` → 1).
* **No UB / no leaks / valid register usage:** the pass rewrites text and
  value graphs, not memory; the MOVBE fold is a peephole on text with an
  explicit adjacency + width + family + permission proof; nothing in the
  change introduces `unsafe`, threads, or global mutable state (the one
  global read was removed — see §1.3).
* **Performance:** kernel Callgrind 82,382,794 vs 175,062,832 (-52.9 %);
  five kernel functions at 2/2/2/2/2 = best-oracle parity including `be64`
  (Clang 23.1 emits 12 there).  Shape-by-shape against GCC 16.2 / Clang 23.1 /
  ICX (`-O3 -march=x86-64-v3`, oracle log refreshed against this tree): six of
  the seven shapes at parity with the best channel, `be64` 2 against Clang's 12,
  and one measured gap — `be16`, 6 against 2 for all three channels — itemised
  in the follow-up with the implementation plan rather than left implicit.
* **Gates:** rustfmt clean, clippy `-D warnings` clean, `ci_local.sh --fast`
  green (stamp in `target/ci_local.pass`), no network in the gates.
* **Honesty:** every stale number found was removed rather than explained
  away; every architectural reach (AArch64 on, RISC-V off) cites per-channel
  oracle counts; the two largest measured gaps (AArch64/i686 frames) are
  documented as open with reproductions, not quietly left out.
