# ms178-1 — LCCC codegen: what landed, and everything still needed upstream

**Base / rebase status.** Upstream `main` moved during the session: it is now
`2d1db599` (`Merge pull request #437`, carrying `43cb636b` "Harden optimizer memory barriers and
x86 allocation safety" and `42537a93` "Gate x86 vectorization on ISA flags; veto aggregate-frame
inlining at -Os"). The whole series was rebased onto it (clean replay, no conflicts) and the tree
re-measured against it; the P0-1 defect survived that move untouched, which is what led to the
root cause in §2. `git diff 2d1db599..HEAD` is `/home/user/ms178-1.patch`.
**Environment note:** this sandbox destroys `target/`, `/swapfile`, `/tmp` and **`.git`** between
some turns. `scripts/arena_session_restore.sh` repairs swap + toolchain in one command (it does
*not* recreate `.git`; `/home/user/artifacts/lccc.bundle` holds the pre-rebase series, kept here as
`refs/heads/legacy-pre-rebase`), and the harness strips `+x` from worktree files, which shows up as
~200 spurious `M scripts/*` — restore modes with
`git ls-tree -r HEAD | awk '$1=="100755"{print $4}' | xargs chmod +x` before committing.
All measurements: local 2-core Xeon @2.6 GHz, 1.9 GB RAM (no PMU), `-O2`, lccc == `main`.

## 1. Landed, built, and verified in this session

| # | change | verification |
|---|---|---|
| 1 | **`aggregate_sroa` form 4 — constant-offset aggregate splitting** (`src/passes/aggregate_sroa/split.rs`, parent converted to `aggregate_sroa/mod.rs`, `mod split;` at :36, called at :885 immediately before the plan is applied). Child module, not a sibling pass: it reuses the parent's `Scan`, `Plan`, `resolve`, `ty_size`, `const_i64`, the escape *allowlist*, and the index-stable rebuild instead of duplicating the machinery that produced every past soundness bug here. Rules 1-6 in the module header. Caps `SPLIT_MAX_BYTES=512`, `CCC_AGG_SPLIT_MAX_FIELDS` (default 64, `0` disables). | builds clean; `scripts/run_regression_suite.sh` **PASS=647 FAIL=0 SKIP=15, A/B-diff 0** with `CCC_AGGREGATE_SPLIT=1` and identically with the form inert; 13 golden programs bit-identical to gcc |
| 2 | **Copy-in expansion** (rule 5): a `Memcpy` that writes the whole object at offset 0 no longer disqualifies it — it is rewritten at the copy's own program point into one `Load`/`Store` pair per field. This is what makes `memcpy(&s,&t,sizeof s)`-style and `t = seed;` initialization promotable. Every other copy shape (partial, copy-*out*, self-copy, already-planned) still refuses, and the *source* of any copy always refuses. | `tests/regression/aggregate_split_constant_fields.c` case (a) |
| 3 | **Anchor fix for that expansion** — see §3, the one real bug found in the new code. | same test, before/after |
| 4 | **Dead interior-pointer pruning** (`prune_dead_interior_pointers`): the `GetElementPtr x, const` values that fed the rewired accesses are removed (fixpoint over users; any user that is not a rewritten access or an already-dead derivation, and any terminator use, keeps the derivation). Required because the parent's drop-check counts `GetElementPtr { base }` as a reference: without pruning the aggregate alloca stays in the frame and the split only churns instructions. | measured pre-wipe: 633 → 603 with pruning absent vs. the frame shrinking when present |
| 5 | **`tests/regression/aggregate_split_constant_fields.c`** — behaviour lock: aggregate vs. scalar oracle for struct fields + copy-in, loop-carried array (back edge), volatile access inside the object, overlapping views. All four pass at `-O0/-O1/-O2/-O3` with the form on and off. A `CCC_AGG_SPLIT_DEBUG=1` `.env` sidecar now also arms the `may_write_memory` cross-check (row 10) on this file. | `ALL PASS` |
| 6 | **New P0 upstream defect found, minimized, documented — and now root-caused and fixed**: `artifacts/repros/rot16-arx-O2-miscompile.md` + 3 self-checking repros. Reproduced at pristine `main` with zero local changes. | see row 9 |
| 7 | **Closed-ended refusal for unmodelled memory writers.** `Instruction::VaArgStruct { dest_ptr, va_list_ptr }` (and `VaArg`, `AtomicStore`, `Intrinsic { dest_ptr }`) carry their memory destinations as **raw `Value` fields, not `Operand`s** — so the parent's escape sweep, which walks operands, never files them as escapes. My first version inherited that blind spot and split an object out from under a `va_arg` wide-struct write; codegen then aborted with `value 15 has no register, stack slot, Copy, or GlobalAddr definition`. Form 4 now refuses every root named by *any* instruction it does not model, via `used_values()` (which does see those fields), plus terminator operands. | `tests/regression/va_arg_wide_struct.c` fails to build with the form on before the fix, passes after; full suite green both ways |
| 8 | **Miscompile triage tooling** kept in-tree: `artifacts/repros/delta_shrink.py` (line-level delta debugger with a "differs from gcc" oracle), `artifacts/repros/killswitch_bisect.sh` (sweeps every `CCC_NO_*` switch, harvested from `src/` at run time, and reports which restore agreement). | both used to produce item 6 |
| 9 | **P0-1 fixed** (`f43037f9`): the x86-64 asm peephole `fold_accumulator_alu_store` folded a staged ALU op onto its copy's *source* register on the strength of a kill test that accepted an x86 two-address ALU op — which also **reads** its destination — as a redefinition. New predicate `pure_family_write()` demands a full, value-independent write; `fam_read_after`'s matching `mov_store` hole (`movzbl %al, %eax` counted as a pure write) is closed by the same all-width source test. | 3 repros + `tests/regression/peephole_acc_fold_arx_src_kill.c` correct at `-O2/-O3/-Os`; 5 new `acc_fold_src_kill_tests` (2 of them fail when the predicate is short-circuited to the old semantics, 3 guard against over-refusing); **0** instruction-count change over 20 corpus programs (4694 vs 4694) |
| 10 | **Cross-check against the canonical predicate.** `split.rs`'s refusal set is now verified against upstream's exhaustive `Instruction::may_write_memory()` (added in `43cb636b`): every instruction that may write memory and names a candidate root must have marked it refused, with `Store`/`Memcpy` exempt because those are the writes the split *models*. It is deliberately not a bare `debug_assert!` — `fastbuild` inherits `release`, so `debug_assertions` are off in exactly the profile the gates run, and a guard that only exists there reads as checked while never firing. It runs whenever `CCC_AGG_SPLIT_DEBUG=1` is set (hence on the suite's split test), and panics in a debug build. | 760 programs (42 corpus + 704 regression + repros) compiled with it armed: **0 violations**. Note the corpus fires the split itself only twice, so the positive path is thin here — that is a property of the corpus, not of the guard |
| 11 | **`aggregate_sroa` form 4 flipped to default-on** in the same series as the P0-1 fix, as planned; `CCC_NO_AGGREGATE_SPLIT=1` stays the escape hatch and `CCC_AGGREGATE_SPLIT=1` remains accepted as an explicit force by pre-flip scripts. | `CCC_AGG_SPLIT_DEBUG=1 ./target/fastbuild/lccc -O2 artifacts/repros/split_fire_probe.c` prints `[SROA-split] fn f alloca v3 size=16 fields=4` by default and 0 events under `CCC_NO_AGGREGATE_SPLIT=1`, with identical program output both ways; suite PASS=652 FAIL=0 SKIP=7, A/B-diff 0, `ci-codegen-gate.py` all golden workloads PASS |

**Not in this patch (lost to the workspace wipe, needs re-adding):**
`tests/benchmark/programs/zlib_longest_match.c`. It was validated earlier — self-checking, prints
`zlib-longest-match: 39157800 3960641400 39319200 0` — but it was untracked when the harness
dropped 1,881 files, and re-deriving the zlib-ng `longest_match` port to hit those exact
accumulators again was out of the time left. Everything needed to redo it is recorded in §5.

## 2. Still needed upstream, in priority order

### P0-1 `-O2` miscompile: register reuse kills a live value in rotate/ARX chains
Sixteen (or nine) live `u32` locals, straight-line add/xor/rotate, no aggregates. `-O0`/`-O1`/gcc
agree; `-O2`, `-O3`, `-O4`, `-Os` produce a wrong word, always **low by 1024 (bit 10)**. Proven to
be *after* the IR: the final IR for the same function evaluates to gcc's answer. The emitted code
puts the def of `x1` into the register that still holds live `x5` and then consumes `x5` from it:

```
addl 28(%rsp), %r8d      ; x5 + x1 -> x1, result left in %r8d (which held x5)
movl %r8d, 16(%rsp)      ; store x1
xorl 16(%rsp), %r14d     ; x13 ^= x1     (reads the slot: correct)
xorl %r11d, %r8d         ; x5 ^= x9       (reads %r8d: now holds x1)  <-- wrong
```

`rotl7(24584^10)=3145984` computed instead of `rotl7(24576^10)=3147008`.

**STATUS: FIXED, `f43037f9`.** It is not a live-range violation in the allocator at all: the
allocator's assignment is legal. `LCCC_NO_PEEPHOLE=1` vs. the final asm shows the asm peephole
`fold_accumulator_alu_store` rewriting the correct three-instruction staging sequence
(`movl %r8d, %eax; addl 28(%rsp), %eax; movl %eax, 16(%rsp)`) into an in-place
`addl 28(%rsp), %r8d; movl %r8d, 16(%rsp)`, destroying the value the next-but-one line still reads.
See `artifacts/repros/rot16-arx-O2-miscompile.md` for the pass-level diagnosis, the predicate that
fixes it, and the falsifiability measurements. Invisible to CI: all 39 golden programs are
bit-identical to gcc, because none of them keeps a second live value in the register being folded
onto.

**Two claims from the earlier version of this document were wrong, and both are corrected above:**
(i) "None of the 143 `CCC_NO_*` kill switches restores agreement" — `CCC_NO_PEEPHOLE_PHASE1=1`
does, in one step. `killswitch_bisect.sh` prefixed every harvested name with `CCC_` while the names
already began with `CCC_NO_`, so it exported `CCC_CCC_NO_X`: the sweep could never have found
anything. Re-run against a compiler with the old predicate it reports
`FIXED-BY CCC_NO_PEEPHOLE / CCC_NO_PEEPHOLE_PHASE1 / CCC_NO_REGALLOC / CCC_NO_SMALL_SLOTS` (4/140).
(ii) "First suspect: the fixed-GPR scratch model / early ABI param reads in `bf0c0f76`; bisect
around it" — unnecessary; the phase bisection named the pass, and the mechanism is text-level,
i.e. invisible to any IR- or RA-level accounting. The trap worth remembering: a `[RA-W4] homes`
dump showing a legal assignment was read as "then it must be a use-count miscount", which cost a
session; **for any asm-level live-range symptom, diff `LCCC_NO_PEEPHOLE=1` output against the final
asm first**.

### P0-2 small constant-trip copy loops defeat SROA *and* vectorization
`chacha20_block`'s state array is refused by form 4 for the right reason, printed by
`CCC_AGG_SPLIT_DEBUG=1`: `root v4 size=64 accesses=168 refuse=true variable=true` — the
`for (i = 0; i < 16; i++) x[i] = in[i];` / `out[i] = x[i] + in[i]` loops keep variable-index
accesses alive, and rule 2 refuses the object. gcc emits `movdqu`/`paddd` for the same loops; lccc
keeps `x` at `160(%rsp)`. The pipeline already runs `aggregate_sroa` twice (mod.rs:1189 pre-unroll,
:1342 post-unroll) — the loop simply never unrolls.
**Needed:** unroll small constant-trip-count loops (body ~6 insns, trip 16) before the post-unroll
call, or memcpy-idiom recognition for the pure-copy one. Verified dead end: the `lz4` zero-init loop
is already minimal, so idiom recognition is not the fix there.

### P1-1 `I32 → I32` copies lowered as sign-extending wide copy + partial stores
`movq %r15, %rbp` before 32-bit shifts; rotate halves materialized as
`movl %eax, 24(%rsp) … orl 20(%rsp), %eax`. Costs instructions and stack in every spill-heavy
function. **Needed:** narrow-copy lowering, no partial-store round trip.

### P1-2 register allocator demotes *all* simultaneously live scalars, not the excess
The fully constant-indexed ChaCha probe needs 16 registers and gets a **4,984-byte frame** for one
64-byte state array, so promotion cannot pay for itself. **Needed:** evict by spill cost, keep the
values feeding the hot chain. Form 4's field cap (64, `CCC_AGG_SPLIT_MAX_FIELDS`) exists because of
this; raising it before this is fixed makes code worse, not better.

### P1-3 variably indexed local arrays (`sha256` `m[64]`, `lz4` `hash_table[]`)
Refused by rule 2 by design — partial splitting with a residual array is unsound by omission.
Upstream needs either window promotion (only the live 16-word message-schedule window) or a real
array-to-registers rotation allocator. Until then `sha256_transform` stays at 234 insns vs clang's
126 and `lz4_compress` at 3.38x gcc.

### P2 codegen gaps vs the oracles (best-of gcc/clang/icx, hot-function instructions)
`chacha20_core` 633 vs 74-180 · `chacha` main 97 vs 69 · `nbody` 389 vs 164 · `sha256_transform` 234
vs 126 · `fannkuch` 202 vs 111 · `expat_xml_scan` 171 vs 97 · `sqlite_varint` 199 vs 138 ·
`linux_find_bit` 142 vs 82 · `lz4` 307 vs 284 · `zstd_count` 103 vs 91 · `mandelbrot` 62 vs 55 ·
`spectral_norm` 58 vs 53. Baseline geomean LCCC/GCC **0.8141** (arith **1.1644**), 39/39 identical
outputs; worst ratios `chacha20_block` 4.797, `lz4_compress` 3.376, `sha256_transform` 1.958,
`expat_xml_scan` 1.779, `linux_find_bit` 1.445. Full rows: `/home/user/results/sb_chacha.json`,
`sb_worst.json`.

### P2 pin the wins so they cannot silently revert
`.github/scripts/ci-codegen-gate.py` gates 7 golden workloads on insns/stackmem/pushes/ymm/fma with
a 2 % band and `--update-baseline`. **Needed:** add `chacha20_block` (and the re-created
`zlib_longest_match`) to `WORKLOADS` and refresh the baseline **in the same commit as P0-2**, since
that is the change that moves `chacha20_core`; and land Rust unit tests for the form —
a `#[cfg(test)] mod tests` in `split.rs` following `src/passes/aggregate_copy_forward.rs:1196`
(hand-built `IrFunction`) covering: split fires on ≥2 constant fields; refuses on volatile,
variable index, escaping address, already-planned position, overlapping views; copy-in expansion
emits GEP/Load/Store at the copy's index; pruning lets the parent drop the alloca; single-field and
over-cap objects are left alone.

## 3. The one real bug found in the new code, and how it was caught

The first version of the copy-in expansion anchored its per-field `GEP`/`Load`/`Store` at
`ii + k` (mirroring form 3's style). The rebuild drains inserts as soon as `anchor <= i` while
walking the **original** indices, and the memcpy itself is removed — so insert *k* landed after the
*k*-th following original instruction instead of all of them landing in the copy's slot. The
observable failure: a 4-field struct `t = seed;`-then-mutate case computed 86 instead of 152.
Fix: anchor every inserted instruction at `ii` and rely on the rebuild's **stable** sort to keep
GEP < Load < Store (the property the comment at that sort already documents). Caught by the new
regression test on the day it was written, which is the argument for landing tests with passes.

## 4. Ship status of form 4: **opt-in today, deliberately**

Suite-level evidence for that claim: with `CCC_AGGREGATE_SPLIT=1` the regression suite is
**647 pass / 0 fail** (same as with the form off), so the only known obstacle to default-on is the
back-end defect below — not anything in the transform itself.

> **Latent hazard worth fixing in the parent, not just here:** `aggregate_sroa::scan()`'s escape
> allowlist and the post-rebuild drop re-scan both inspect `Operand`s (plus a hand-written list of
> `Load/Store/GetElementPtr/Memcpy/Intrinsic` pointer fields). `VaArgStruct.dest_ptr` is in neither
> list. Forms 1-3 do not reach it today (they require memcpy-shaped writes, which a `va_arg` store
> is not), so this is latent rather than live — but any future form of that pass that widens
> eligibility will hit it. Either add those raw-Value fields to `scan()`/the re-scan, or have every
> form call a shared `names_memory(inst, target)` helper like the one `split.rs` now needs.

**Form 4 is now on by default**; `CCC_NO_AGGREGATE_SPLIT=1` disables it and
`CCC_AGGREGATE_SPLIT=1` remains accepted as an explicit force (harmless, kept so pre-flip A/B
scripts still run). It was held opt-in for one measured reason: form 4 is *correct* at the IR level,
but on high-pressure functions it removes the frame slots that were hiding P0-1 —
`artifacts/repros/rot16_arx_array_-O2.c` was correct with the form off (`rc=0`) and wrong with it on
(`rc=1`) while the two pure-scalar repros were wrong either way. Landing it default-on then would
have converted a latent back-end bug into a front-line miscompile; with P0-1 fixed in the same
series the gate is gone, and the array repro is now `rc=0` at `-O2/-O3/-Os` both with and without
the form. P0-2 is what makes it *pay*.

The corpus cost of the flip is still zero instructions (`validate_form4.sh`-style census: 13 golden
programs identical on/off, `chacha20_block 709/709`, `nbody 376/376`, `sha256_transform 432/432`);
the win is gated on P0-2 because chacha-class kernels are refused by rule 2 (copy loops) either way.

## 5. Reproducing, and the notes a future session needs

```
cargo build -j1 --profile fastbuild --locked          # cold build OOMs at -j2 on a 1.9 GB box:
                                                     # use -j1 (and `sudo swapon` a 6 GB file)
./target/fastbuild/lccc -O2 artifacts/repros/rot16_arx_scalar_-O2.c -o /tmp/a && /tmp/a; echo $?  # 1 = P0-1
./target/fastbuild/lccc -O2 tests/regression/aggregate_split_constant_fields.c -o /tmp/r && /tmp/r
CCC_AGGREGATE_SPLIT=1 ./target/fastbuild/lccc -O2 -S tests/benchmark/programs/chacha20_block.c -o - | grep -c '^    [a-z]'
CCC_AGGREGATE_SPLIT=1 CCC_AGG_SPLIT_DEBUG=1 ./target/fastbuild/lccc -O2 -S <file.c> -o /dev/null  # per-alloca verdicts
LCCC_BIN=/path/to/unfixed/lccc bash artifacts/repros/killswitch_bisect.sh <file.c>
                                                     # which pass (if any) restores agreement;
                                                     # empty switch list is now FATAL, and it
                                                     # scrubs ambient CCC_* instead of double-prefixing
python3 .github/scripts/ci-codegen-gate.py --lccc /home/user/lccc/target/fastbuild/lccc --summary
cargo fmt --check && cargo clippy --profile fastbuild --all-targets   # the repo is fmt- and
                                                     # clippy-clean; this was skipped once and
                                                     # `split.rs` landed unformatted
LCCC_BIN=$PWD/target/fastbuild/lccc bash scripts/run_regression_suite.sh   # ~4 min, IR verify + gcc A/B
bash /home/user/lccc/scripts/arena_session_restore.sh  # fresh sandbox: swap + toolchain in one go
```

* **Useful flags/env found the hard way**: there is no `-emit-mir`/`--mir-dump-dir` in this build. IR
  dumps are `CCC_DUMP_IR_AFTER=1` (after *all* IR passes, Debug format) and `CCC_DUMP_EACH_PASS=1`
  (+ `CCC_DUMP_FUNC=<name>`); `CCC_VALIDATE_SSA=1` / `CCC_VERIFY_IR=1` run the verifiers.
  `pass_disabled()`-style `CCC_NO_*` switches number 143.
* **Recreating `zlib_longest_match.c`**: port zlib-ng's `longest_match` chain walk; the traps that
  cost the most time the first round — `UNALIGNED_OK` pair loop needs the single
  `scan++, match++` *before* it and no `scan += 2`; `match_start` is only valid once
  `prev_length` was beaten; insert each position once and read the chain head before insertion;
  `longest_match` reads `MAX_MATCH` past `strend` so the corpus needs GUARD bytes; the window is
  `W_SIZE`, not the corpus size. Reference output to match: `zlib-longest-match: 39157800
  3960641400 39319200 0`.
* **Harness hazard**: the sandbox wipes `/tmp` and truncates the persisted snapshot (this session
  lost 1,881 worktree files, `~/.cargo`/`~/.rustup`, and `target/` — `git status` then looked like a
  mass deletion). Recovery that worked: `git clone /home/user/artifacts/lccc.bundle`, move the
  `.git` in, `git reset` (mixed, leaves the worktree alone), `git checkout -- .`. Snapshot via
  `/home/user/lccc-snapshot.sh` after *every* validated change; it is what made this recoverable.

---

# S16 (2026-09-12, part 2): census follow-ups, #503 carry/implicit-operand audit, latent jump-table gap

Full record: `engineering/AUDIT-2026-09-12-S16-census-peephole-followups.md`.

Shipped in this part:
- `label_is_fallthrough_only` now treats jump-table data edges (`.long .LBB - .Ljt`, `.quad .LBB`)
  as predecessors — the old predicate only scanned direct jumps, so switch case labels were
  misclassified as fallthrough-only (latent store-fold miscompile path; no live corpus hit).
  Symbol-token matching with boundary guards; positive + near-miss + adversarial tests.
- `is_dst_establishing_move` allow-list fixed: `movlzw` was never a GAS mnemonic (it is `movzwl`);
  now includes every reg-reg move/extend spelling. Missed -O0 folds only.
- #503 carry audit completed: combined CF+ZF predicates (seta/setbe/cmova/jbe…) added to the
  CF-reader set; silicon-verified PDEP/PEXT **preserve** CF (moved back to the skip table), BLSI
  writes CF=1, ANDN/BEXTR/BZHI/BLSMSK/BLSR clear, PTEST/PCMPSTR/COMISS/FCOMI/RDRAND family
  clobber; implicit-operand classifier extended and unified via `implicit_write_refs()`
  (string/BCD/pusha*/syscall-family/VMM/TDX/RSM/PCONFIG/…).
- Store/load cascade fixes (S15 leftovers): fold across untargeted guard labels; ALU fold target
  resolution across destination-establishing copies; reload-preservation predicate so the phase-1
  reload pass cannot starve the phase-2 memory fold.
- Upstream miscompile documented: `ra09_selfop_xor.c -O1` forwards a register across a
  rotl diamond that upstream gets wrong (candidate's +1 stack ref is the correct output).
- Scripts added: `ab_pair_sweep.sh`, `ab_focused_remeasure.sh` (median-of-seeds),
  `align_policy_ab.py`; A/B noise characterised (byte-identical controls; byte-exact NOP-padding
  isolation) — blanket 32-byte alignment rejected after a 31-rep confirmation failed to reproduce
  the 15-rep signal; default stays GCC's 16, hotness-driven alignment recorded as P1 follow-up.

Final gates (hardened tree): ci_local --fast 27/27; clippy -D warnings clean; rustfmt clean;
lib 2541/0/6; regression 746/0 (+SSA-validated 746/0); benchmark output oracle 204/0;
x64 differential 0 rc-diff / 0 one-sided / censused 66 asm diffs; i686 fully byte-neutral;
fuzz 2160/0; census -1885 insns / -1745 stack refs with only the two accepted residuals.
