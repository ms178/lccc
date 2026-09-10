# Cross-backend oracle, miscompile fixes and cross-pollination

State of the work as of the ARM/RISC-V copy-coalescing ports. This document
describes **what is true now**; superseded analyses are not kept as history.

---

## 0. Environment: the sandbox resets at every turn boundary

Everything outside the persisted `/home/user` workspace disappears between
turns, and `target/` is excluded from snapshots by design, so the compiled
compiler is gone too. Restore in this order (one command):

```
scripts/arena_session_restore.sh      # swap, toolchain, apt deps, exec bits
cargo build --profile fastbuild -j 2  # ~2m30s from clean
```

`arena_session_restore.sh` is idempotent and covers everything: 8 GB swapfile
(`/sbin/swapon`, not on PATH), the rustup toolchain under `/home/user/.cargo`
+ `/home/user/.rustup`, `gcc-multilib` + `gcc-aarch64-linux-gnu` +
`gcc-riscv64-linux-gnu` + `qemu-user`, the `origin` remote, and the `+x` bits
(86 scripts lose them on every restore).

Two traps that cost real time:

- **Without the cross toolchains, `lccc-arm`/`lccc-riscv` fail with
  `stddef.h: No such file or directory`** — they read GCC's builtin include
  directory for the target (`/usr/lib/gcc/<triplet>/14/include`). A one-line
  `hello.c` fails; it looks like a compiler bug and is not.
- **`git diff` shows ~86 spurious mode changes** after a restore. Run
  `git ls-tree -r HEAD --format='%(objectmode) %(path)' | awk '$1=="100755"{print $2}' | xargs chmod +x`
  and verify `git diff --summary | grep -c 'mode change'` is **0** before
  snapshotting, or the patch carries all of them.

---

## 1. The instrument: `scripts/cross_backend_check.py`

Compiles each corpus program with `lccc`, `lccc-i686`, `lccc-arm`,
`lccc-riscv` and one or more reference compilers, runs every binary under
qemu, and diffs exit status + stdout.

```
python3 scripts/cross_backend_check.py [--programs a,b] [--opts=-O0,-O2,-Os]
        [--reference gcc [--reference clang]] [--repeat N] [--jobs N]
        [--json P --compare OLD] [--quick] [--gate] [--strict-timeout]
```

Design points that matter (each one was a false bug report before it was
fixed):

- **All references must agree**, or the program is skipped as unusable.
- **`preflight()`** checks every compiler, qemu and the 32-bit libs *before*
  the run and exits 2 — never 45 minutes into it.
- **Per-cell verdicts** distinguish `ok` / `MISMATCH` / `COMPILEFAIL` /
  `TIMEOUT` / `EXECFAIL` / `NONDET`. A qemu timeout under `--jobs` load is a
  speed artefact, not a miscompile; `--repeat N` separates nondeterminism from
  a real mismatch.
- **A program's output is a function of its data model *and* its FP model.**
  The i686 reference must be `-m32 -msse2 -mfpmath=sse`: plain `-m32` uses x87
  excess precision and fakes a `libm_round_family` mismatch at every opt level.
- **`LINK_LIBS = ["-lm"]`** is appended to every link (nbody needs `sqrt`).
- **Never put a data-defining macro in `DEFAULT_SCALE`.** `-DNBODIES=32` on
  `nbody.c` zero-fills 27 of the 32 static bodies, so every backend prints NaN
  and "mismatches" identically. Only pure size/step macros belong there.
- Scratch is disk-backed (default `target/crossbe-scratch`): `/tmp` is a
  993 MB tmpfs and ~1350 static binaries exhaust it, which surfaces as
  `final link failed: No space left on device` reported as a reference failure.

Run it with `PYTHONUNBUFFERED=1` under `tee`, and never rebuild lccc while a
run is in flight.

---

## 2. Miscompiles found and fixed (all verified against reference compilers)

| # | Bug | Status |
|---|---|---|
| 1 | AArch64 silent miscompile in `sha256_transform` | fixed |
| 2 | `-O0` emitted x86 text into AArch64/RISC-V assembly | fixed |
| 3 | i686 expat name-scanner miscompile | fixed |
| 4 | AArch64 `-Os`: value orphaned by `invalidate_acc` | fixed |
| 5 | AArch64 `-Os`: peephole deleted a live alias (`glibc_memcmp` SIGSEGV) | fixed |

Bug 5's **root cause is not what the first fix assumed**. The scan treated
`ret` as having no runtime effect, so a redefinition of `x0` in a *later block*
was accepted as proof that a copy was dead — but x0-x7 are return-value
registers, and the path through that `ret` had already handed the value to the
caller. The first fix ("never cross a label") suppressed that one layout and
left the hole open. The real fix is a `ret` barrier for `dst < 8`
(`LineKind::Ret` invalidates the fold); x8-x17 are caller-saved and x19-x28 are
restored by the epilogue, which is itself a redefinition, so the barrier is
precise rather than another guess.

The `nbody` `-O2`/`-O3` AArch64 SIGSEGV that used to be listed here as an open
defect is **fixed** — it was the same class of bug (a copy deleted while still
live on the loop path) and now matches gcc at every opt level.

---

## 3. ARM alias folding: from heuristic to exact CFG query

`propagate_address_aliases` deletes `mov xD, xS` when dst is only ever used as
an address base, rewriting those uses onto `xS`. Proving the copy dead needs to
know whether the redefinition of dst *dominates* every read of it, which text
cannot answer.

The pass now builds a real CFG (`struct Cfg`: blocks, successors, iterative
dominator bitsets) and gates the fold on one exact query,
`redef_covers_all_reads`:

> every read of `dst` that can execute after the `mov` **without passing the
> redefinition** must be an address use that gets renamed onto the source.

Details that make it exact rather than conservative:

- Blocks split after **every** control transfer, not only at labels. Earlier
  passes can leave a branch mid-block; an edge missing from `succs` is an edge
  the query cannot see.
- The redefinition's own block **is** walked (execution can run the lines
  before the redefinition) but is not propagated out of (every path leaving it
  has executed the redefinition).
- When the mov's block is in a cycle, the back edge re-enters at the block's
  **top**, so lines *before* the mov are fed by it and are checked too.
- `LineKind::Ret` invalidates the fold for `dst < 8` (§2).

With that in place the dead-copy deletion — removing a copy with no rewritable
address uses — is sound. It is worth 11 (-Os) / 16 (-O2) instructions; the
other ~40 of the raw 53/51 that the unsound version removed were illegal
deletions of live copies, i.e. latent miscompiles of the glibc_memcmp kind.

---

## 4. Cross-pollination: register-copy coalescing, x86 -> ARM -> RISC-V

x86 had whole-function register-copy coalescing
(`x86/codegen/peephole/passes/copy_coalesce.rs`). **ARM and RISC-V had
nothing**, so their functions paid an entry shuffle x86 deletes: the register
allocator hands parameters homes that do not match the ABI registers they
arrive in, and the resulting `mov`/`mv` has a destination that is live for the
whole function — no local pass can remove it. Coalescing renames the
destination family onto the source everywhere after the copy and drops the
copy.

Ported as `coalesce_entry_copies` in both backends, with per-architecture
legality rules:

1. The copy sits in the **straight-line entry run** (no label/branch/call/ret
   before it), so every earlier line runs once and cannot be re-entered by a
   back edge.
2. **`src` is mentioned nowhere else in the function** — no later write
   clobbers the coalesced value, no other live range is disturbed. A
   parameter's arrival in `src` is implicit, not a mention.
3. After the copy, `dst` has **no unrenamable reader or writer**: no implicit
   operand (`classify_implicit_operands_a64` / `_rv`), no `ret` while `dst` is
   a return-value register (ARM x0-x7, RISC-V a0-a7), and no call while the
   value lives in a caller-saved register **and is still used afterwards**.
4. Frame pointer, return address and stack pointer are never renamed
   (ARM x29/x30/sp; RISC-V s0/ra/sp). A far RISC-V `jump` uses t6 as scratch,
   so t6 is not renamed across one.

**Measured (43-program corpus, instruction count):**

| backend | -Os | -O2 | win vs. no coalescer |
|---|---|---|---|
| ARM | 10675 | 11253 | -15 / -12 |
| RISC-V | 20942 | 21998 | -9 / -2 |

The ARM win is real because 75 of its 567 entry-run copies have a source that
appears nowhere else. **RISC-V has 775 entry-run copies but only 1 with a
unique source** — its allocator reuses argument registers after moving them to
their homes, so rule 2 rejects nearly everything. That is the single biggest
remaining optimisation in this area: x86's pass uses **liveness**
(`live_after(i, src) == false`) instead of the syntactic rule 2, and porting
that liveness to ARM and RISC-V is what would unlock the other ~490 (ARM) and
~774 (RISC-V) copies. It is the first item in §10.

---

## 5. Measured results

ARM, 43 programs, instruction count:

| build | -Os | -O2 |
|---|---|---|
| baseline before this work (miscompiles: glibc_memcmp SIGSEGV) | 10677 | 11248 |
| conservative label bail | 10703 | 11279 |
| + exact CFG query | 10701 | 11281 |
| + sound dead-copy deletion | 10690 | 11265 |
| + copy coalescing | 10675 | 11253 |
| **+ identical-block merging (current)** | **10630** | **11223** |

RISC-V, same 43 programs:

| build | -Os | -O2 |
|---|---|---|
| baseline | 20951 | 22000 |
| + copy coalescing | 20942 | 21998 |
| **+ identical-block merging (current)** | **20891** | **21953** |

The correctness fixes are paid for by two new optimisations rather than by
surrendering one: the current ARM build is **47 (-Os) / 25 (-O2) instructions
smaller than the miscompiling baseline it replaces**, and the RISC-V build is
**60 / 47** smaller than the pre-work baseline.


---

## 5b. Cross-pollination from x86: identical basic-block merging

The x86 backend is the most polished of the four: 22 peephole pass modules
behind ~70 `sk("...")` gates, where ARM and RISC-V carry their passes inline.
A census of what x86 has that the others do not singled out three passes:
`identical_blocks`, `epilogue_merge` and `frame_compact` — none of which
existed in ARM, RISC-V or i686.

`identical_blocks` was ported first. Its soundness contract, carried over
verbatim in spirit, is:

1. the two blocks must have the same instruction text;
2. the deleted block must be entered only by **explicit branches** — a
   fall-through edge cannot be retargeted by renaming a label, and after
   deletion the preceding block would fall into whatever now follows it;
3. a **function's entry block is never deleted** (its label is the symbol);
4. blocks reached from an unresolvable edge (an indirect `br xN` jump table,
   a `jalr`) are excluded — and, unlike `Cfg::build`, they do **not** poison
   the whole file. The merge pass therefore builds its own tolerant block
   model: a function containing a switch's jump table still gets merged
   everywhere else.

On entry state, x86 requires *equal predecessor sets* plus *not flag-dependent
at entry*. AArch64 and RISC-V have no EFLAGS, so the second condition
disappears — and the first turned out to be far too weak on its own:

> **Measured.** A census of the ARM corpus found 19 duplicate-text groups (33
> candidate pairs). **All 33 were rejected by the predecessor-set rule** —
> the x86 pattern it was written for (phi trampolines sharing their
> predecessors) simply does not occur in this corpus. Porting the rule as
> written would have shipped a pass that never fires.

The fix was to add a second, *stronger* admission test that does not need the
predecessor sets at all: a block is **live-in independent** when every register
it reads is defined inside the block before the read (implicit operands
included; blocks containing a call or `ret` are excluded, since a call reads
the argument registers implicitly and `ret` reads the return-value registers).
Two identical live-in-independent blocks are interchangeable whatever their
predecessors are, because the live-in state is irrelevant to both. With that
test added the pass fires for real: **-45 / -30 instructions on ARM, -51 / -45
on RISC-V.**

Both backends were verified at 48/48 against `aarch64-linux-gnu-gcc` and
`riscv64-linux-gnu-gcc` under qemu at `-O1/-O2/-Os/-O3` (12 programs x 4
levels), on top of the corpus-wide instruction-count measurements.

Still unported: `epilogue_merge` and `frame_compact` (x86-only), and the
register-copy coalescers still use the *syntactic* "source appears nowhere
else" rule on ARM/RISC-V rather than x86's *liveness* rule
(`live_after(i, src) == false`). The census for that is in section 4: ~490
further copies on ARM and ~774 on RISC-V are in reach once the liveness rule
is ported — the largest remaining single item.

---

## 6. Golden workloads: zstd added

`.github/scripts/ci-codegen-gate.py` now gates `zstd_count.c` alongside
gzip_crc32, zlib_ng_adler32, expat_xml_scan, sqlite_varint, glibc_memcmp and
hash_table, with a baseline entry (`insns 131, moves 34, pushes 7,
stackmem 0`). It earned the place twice: it caught the loop-carried copy bug
(§3), and zstd is what the x86 boot path decompresses with, so the golden set
now covers the decoder the boot path actually runs.

---

## 7. Known open defects

- **`struct_copy` -O1/-Os (AArch64) mismatches the reference.** It also fails
  with `LCCC_NO_PEEPHOLE=1`, so it is **not** a peephole bug — it is a
  pre-existing codegen defect elsewhere (struct copy / aggregate ABI) and is
  still open.
- **ARM global store forwarding remains disabled** (test `0036_0041`): the
  same-register NOP elimination has an unidentified correctness bug in complex
  float-array code. RISC-V *has* a working `global_store_forwarding`, so that
  implementation is the reference for fixing ARM's.
- **The i686 `staging_reads_safe` census** (whole-function read census in the
  i686 peephole) is still unaudited.
- **IR loop rotation is still opt-in** (`CCC_LOOP_ROTATE=1`,
  `engineering/tasks/TASK-PF-17-LOOP-ROTATE-DEFAULT.md`); the ARM peephole's
  `rotate_simple_loops` is default-on independently.

---

## 8. Kernel boot: blocked, and how to unblock

`scripts/build_kernel_boot.sh` cannot run: the workspace wipe removed
`/home/user/kernel-work/linux-6.18.47` **and**
`/home/user/archpkgbuilds/packages/linux-cachymod-6.18`, which
`prepare_kernel_tree.sh` requires for the 26 CachyMod patches and the package
config (`PKGDIR` check line 44, patches line 158, config line 172).

The AUR `linux-cachyos` clone is **not** a substitute: it is version **7.2**
and carries **0 patch files**.

```
# restore the PKGBUILD tree (26 patches + config, CachyOS source order), then:
KERNEL_DIR=/home/user/kernel-work/linux-6.18.47 bash scripts/prepare_kernel_tree.sh
KERNEL_DIR=... LCCC=.../lccc bash scripts/build_kernel_boot.sh
```

Fallback if the patch set is unrecoverable: a **vanilla 6.18.47** tree. The
tarball download is already in the script (kernel.org), and `arch/x86/boot` is
largely patch-independent, so a defconfig build is a valid boot-code test.
`scripts/qemu_qmp_probe.py` probes `decompress_kernel -> zstd_decompress_dctx`.

---

## 9. CI status

- `cargo fmt --all -- --check` and
  `cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings`
  — the two commands of the `Clippy` job (`.github/workflows/ci.yml:234-254`,
  reproduced verbatim by `scripts/ci_local.sh:144-146`) — both **exit 0**.
- The previous red was `cargo fmt`: a temporary `peep!` debug macro was still in
  the tree when the patch was snapshotted. Never snapshot with debug code in
  the tree; `cargo fmt --all -- --check` is part of the snapshot gate.
- **The full `ci_local.sh` has not been run end to end.** Run
  `scripts/ci_local.sh --fast` per the standing instruction; the full run
  (regression corpus + benchmark oracle, ~26 min) is still owed.

### 9a. PR #471 was red: a flaky test, not a codegen regression

The `Test Suite` job failed at the `Run tests` step with exit 101. It is a
**flaky test**, and reproducing it took more than running the suite once:

> `passes::loop_invert::tests::the_duplicated_test_reads_the_updated_induction_variable`
> panicked with `index out of bounds: the len is 1 but the index is 1` — the
> latch never received the duplicated header block. It failed in **1 run out of
> 12**; run in isolation it passed 30/30, which is the signature of
> cross-thread interference rather than a logic bug in the pass.

**Root cause.** `invert_loops` read its kill switch from the process
environment (`std::env::var("CCC_NO_LOOP_INVERT")`), and a neighbouring test,
`the_pass_can_be_disabled`, exercised that switch with
`std::env::set_var`. The environment is process-global and `cargo test` runs
its tests on parallel threads, so the ten other tests in the module could call
`invert_loops` while the variable was set, observe `enabled == false`, skip the
rotation and then index an instruction that was never created. `set_var` is
`unsafe` in a multithreaded process for exactly this reason.

**Fix.** The kill switch is now an explicit parameter. `LoopInvertConfig`
carries `enabled`/`debug`; `invert_loops` (the driver's entry point, called
from `src/driver/pipeline.rs:1846`) resolves it from the environment **once
per compilation** and delegates to `invert_loops_with(func, cfg)`. The tests
call `invert_loops_with(&mut f, LoopInvertConfig::testing()|disabled())` and
touch no global state, so `set_var` is gone from the module entirely. The
documented `CCC_NO_LOOP_INVERT` / `CCC_DEBUG_LOOP_INVERT` escape hatches behave
exactly as before.

Verified: **30 consecutive full-suite runs, 0 failures** (previously ~1 in 10).

Two sibling passes — `vec_interleave` and `vec_load_sink` — still use the same
`set_var` idiom, defended by a module-local `ENV_LOCK`. That lock only
serialises callers that remember to take it, so the pattern is fragile even
though it is not currently racing; both should get the same explicit-config
treatment.

### 9b. `scripts/ci_local.sh` now runs the tests GitHub runs

The `--fast` gate set was green while the PR was red because the `cargo-test`
gate was tagged `slow` and therefore skipped. It is now:

- **in the fast set** — it is the step the required check runs, so `--fast`
  has to cover it;
- **run with the same `RUSTFLAGS` the build resolved** — `build_lccc_fast.sh`
  publishes them to `target/lccc-rustflags`. `RUSTFLAGS` is part of cargo's
  fingerprint, so passing a different value would rebuild the whole tree;
- **repeatable** via `LCCC_TEST_REPEATS=N`, which is how the flake above was
  found and how its fix was proven. A single run cannot see a
  process-global-state race; three repeats cost seconds once the test binaries
  are built.

`ci_local.sh --fast` with `LCCC_TEST_REPEATS=3`: **15 passed, 0 failed,
3 skipped — ALL GATES GREEN**.

---

## 10. TODO for the next session (ordered)

1. **Liveness-based copy coalescing for ARM and RISC-V.** Replace rule 2
   ("`src` mentioned nowhere else") with x86's dataflow rule
   (`live_after(i, src) == false`, plus "no write to `src` while `dst` is still
   live"). Measured upside: ~490 more copies on ARM, ~774 on RISC-V. This is
   by far the largest single win available in the backend.
2. **Close the RISC-V pass gap.** Its peephole has 27 passes against ARM's 78
   and i686's 117; candidates for porting, cheapest first:
   `eliminate_move_chains`, `eliminate_overwritten_moves`,
   `eliminate_overwritten_stores`, `reuse_stack_loads_within_blocks`,
   `thread_spill_slots`, `hoist_loop_invariant_remats`,
   `sink_loop_carried_stores`, `fold_zero_stores`.
3. **Fix ARM global store forwarding** using RISC-V's working
   `global_store_forwarding` as the reference (test `0036_0041`).
4. **`struct_copy` AArch64 defect** — bisect outside the peephole (it fails
   with the peephole disabled).
5. **Audit i686 `staging_reads_safe`.**
6. **Kernel boot** once the tree is restored (§8): build the 23 setup objects
   with lccc + lccc-ld, pass the `_end <= 0x8000` ASSERT, then fix whatever
   miscompiles the boot exposes.
7. **Full `ci_local.sh`** end to end.
