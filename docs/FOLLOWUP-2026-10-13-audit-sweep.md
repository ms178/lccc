# Follow-up — 2026-10-13 · AUDIT-2026-10-13 sweep (session 2)

Baseline: `7e94441` (upstream `ms178/lccc` main at session start).
Deliverable: `ms178-1.patch` (snapshots `S01`, `S02`, … in the session ledger).
Scope of this document: what the audit's item list asked for, what is now
**fixed and gated**, what is **verified-open** (with the entry point a future
session should start from), and the standing environment/harness rules this
session had to relearn.

---

## 1. Accomplished in this session

### AUD-VABI-1 — a 16-byte vector crosses a compiler boundary in ONE `%xmm` (FIXED, cross-compiler verified)

*Symptom.* `<4 x i32>` (a `vector_size(16)` C vector) returned from a
GCC-compiled callee to an lccc caller gave `16 0 65536 0`, and the mirrored
combination (lccc callee, GCC caller) gave `1 2 3 4` — both rc=0, both silently
wrong. `{double,double}` (the two-`xmm` struct control) was already correct in
all four combinations, which is why same-compiler testing could never see it.

*Cause.* Vectors travelled the by-value-aggregate path with **empty**
`struct_arg_classes`, i.e. they were classified INTEGER/GP. The psABI
classifies `vector_size(16)` as SSE + SSEUP: the whole 16 bytes live in ONE
`%xmm` (GCC oracle asm: `movdqa`), while the struct control legitimately uses
two registers.

*Fix.* `CType::sysv_vector_carrier()` returns `Some(([Sse], false))` for
≤8-byte vectors, `Some(([Sse, Sse], true))` for 16-byte vectors — reusing the
existing "one 16-byte %xmm" carrier that the `_Float128` path already had
(`is_f128_sse`, `ParamClass::F128SseReg`, `movdqu` in/out), so no new backend
path was needed — and `None` for anything else (including 32-byte, which needs
an `-mavx` decision). Wired into: return classification + `ret_is_f128_sse`,
both parameter paths (declaration and definition), the argument side, and the
three **indirect-call** arms, which have no `FuncSig` to read and hardcoded
`ret_is_f128_sse: false` — those now derive the carrier from the callee
expression's own function type, so `pick_impl(0)(a,b,c)` reads `%xmm0` where
the callee wrote it.

*Guard.* `target_elf_machine() == 62`. The flag is target-relative (AArch64 Q
register, RISC-V GP pair) — **do not generalise it** without that work.

*Gate.* `tests/regression/check_vector_abi_boundary.sh` (+ fixture
`tests/regression/vector_abi_boundary/vector_abi.c`, split into caller/callee
TUs with `CALLEE_ONLY`) links each caller/callee combination against GCC at
`-O0` and `-O2`, asserts the four combinations' output, and asserts the
**arrival registers** (`%xmm1` for a `v2df` pair, `%xmm3` for the
`{double,double}` control) so the two conventions cannot silently merge.
Mutation-verified: neutering the carrier helper fails the gate.

### AUD-RATCHET-1 — the audit census is measurable and enforced (DONE)

`scripts/audit_census.py` + `scripts/audit_census_baseline.json`:
panic-class sites (and the hot subset in `frontend/` + `ir/lowering/` +
`driver/`), `unsafe` sites, unannotated `#[ignore]`, functions over
300/500/1000 lines, `CCC_*`/`LCCC_*` literals and how many **no** markdown file
names. `--check` fails the build when any count grows; `--update` re-baselines
(it warns when a metric rises); `--selftest` pins the tokenizer (line comments,
nested block comments, raw strings, `#[cfg(test)]` truncation, line-number
preservation); `--top-fns N` and `--list-undocumented-knobs` are the worklist
views.

*Calibration (why the numbers are trustworthy).* Running `--top-fns` on this
tree reproduces the audit's published worst-offender table **exactly**:
`emit_intrinsic_impl` 5,839 @ `x86/codegen/intrinsics.rs:1713`;
`emit_shared.rs:179` 4,135; `emit_exec.rs:115` 4,120; `emit_script.rs:1009`
3,094; `encode_mnemonic` 3,056 @ `x86/assembler/encoder/mod.rs:3371`;
`allocate_registers` 2,669 @ `regalloc.rs:3111`; `prologue.rs:289` 2,446;
`slp_vectorizer.rs:2567` 2,235. The metric definitions in the docstring are
therefore the audit's own, not an approximation of them.

Baseline at `1f0261a`: `panic_sites` 889 (109 hot), `unsafe_sites` 27,
`ignored_tests` 1, `fns_over_300/500/1000` 192/76/23, `knob_literals` 498,
`knobs_undocumented` 292.

Wired into `scripts/ci_local.sh` (two fast gates) **and**
`.github/workflows/ci.yml`; `scripts/check_ci_gate_parity.py` is green (188
commands), which is the mechanism that keeps the two mirrors from drifting.

### AUD-HYGIENE-1 — tracked files that should not be in the repository (DONE)

* The tracked Claude Code permissions file (it granted every clone blanket
  shell auto-approval), the leaked snapshot anchor at the repository root, and
  the tracked scratch directory that `.gitignore` already listed
  (assembly + two ELF executables referenced by no script) are gone, with
  `.gitignore` entries so they cannot come back.
* The loose `tests/*.{c,s,sh}` became real gates — and this is where the audit's
  "move with a gate or delete" turned up value:
  * `tests/regression/check_compat_suite.sh` (was `tests/compat_test.sh`, 20
    programs, previously unreferenced): now honours `$CCC`, uses a private
    `mktemp -d` scratch (it used fixed `/tmp/lccc_test*` paths, so two runs
    clobbered each other), prints the compiler's own stderr on failure, and
    **exits non-zero** (it always exited 0 before, so it could not fail
    anything). 20/20 vs GCC in ~2 s.
  * `tests/regression/check_ivsr_reproducers.sh` + the probe sources under
    `tests/regression/ivsr-probes/`: metamorphic rather than shape-pinned —
    the same programs must compute identical numbers at `-O0..-O3` with
    `CCC_NO_IVSR` on and off, differentially against GCC. Pinning today's
    instruction selection would fight every future improvement; pinning the
    *equivalence* protects the property that matters.
  * `fib_rec2iter.c` / `verify_fib_perf.sh` moved next to the benchmark
    workflow's new path (`.github/workflows/bench.yml` updated in lockstep).
* Stale pre-fix `.s` snapshots deleted (they were records of a state, not
  tests).
* `artifacts/repros/` (97 programs, the directory's own scar tissue) replayed
  by `scripts/check_repro_replay.py`: **none of them still reproduce**. Getting
  there needed the ISA to be pinned on both compilers, which is the finding
  worth keeping — see the box below. The three `csmith` programs that
  `#include "csmith.h"` (a header the generator emits and this repository does
  not vendor) are *declared* rather than skipped, and the declaration is
  checked in both directions so it cannot rot into a skip list. The bracketed
  file names (`[CCC_RA_GLOBAL_LOCATION=1]`) now use the shell-safe
  `__env_CCC_RA_GLOBAL_LOCATION-1` form, and `artifacts/repros/README.md`
  documents the naming contract the gate parses.

> **Reading a differential result: pin the ISA on both sides.** An unpinned
> replay of `artifacts/repros/` reports 29 failures; with `-march=x86-64` handed
> to both compilers, 94 of 97 agree (the other three are the header-dependent
> csmith files). The gap is not codegen: lccc's documented project baseline is
> `x86-64-v3`, GCC's default is plain `x86-64`, and lccc's backend **contracts
> `a*b+c` into FMA at every level including `-O0`** (measured: 10 `vfmadd`/
> `vfmsub` sites in an `-O0 -march=x86-64-v3` build) while GCC starts
> contracting at `-O2`. Contraction is implementation-defined in C, so this is
> legal — but it means `-O0` numerics differ from GCC unless the comparison is
> pinned. The gate therefore runs two pinned legs (`-march=x86-64` and
> `-march=x86-64-v3 -mno-fma`); a `-ffp-contract=` flag is the obvious knob if
> GCC-identical `-O0` numerics are ever required.

### AUD-SILENT-ZERO-1 — the remainder (FIXED, two layers + the ICEs)

Previously the non-x86 emitters' "x86-only" arm stored 16 zero bytes when the
destination was a known slot and **nothing at all** otherwise, so an x86
intrinsic compiled for AArch64/RISC-V produced a wrong value with rc=0. The
first half (previous session) replaced that with the target's trap at runtime.
This session added the compile-time contract the audit asked for, plus removal
of the remaining ICEs:

* Lowering (`check_x86_intrinsic_feature`, which is reached **only** from the
  arm that dispatches the x86 builtin families) now rejects any such call on a
  target that is not x86-64/i686:

  ```
  x86_only.c:4:69: error: `__builtin_ia32_aesenc128` is an x86-only intrinsic
  and is not available on aarch64 (aes instruction-set extension, -maes)
  ```

  No per-op table is needed to decide *that* it is unavailable — only to name
  the extension in the hint. `#ifdef __x86_64__`-guarded code still compiles:
  the diagnostic fires only when the call is actually lowered.
* Eight compiler ICEs replaced by the target's own trap (`emit_unreachable` →
  `brk #0` / `ebreak` / `ud2`), each with its invariant written down: five
  AArch64 arms (4-wide/AVX group, 256-bit group, three "dest is not
  slot-homed" f128 sign-bit paths), one RISC-V vector arm, one RISC-V f128
  path, one i686 vector arm. The f128 invariant is structural —
  `track_f128_self` registers a value whose readers resolve it through
  `f128_resolve_slot_addr`, so a register home would be invisible to its own
  consumers — and was checked to hold across `-O0..-O3` on the f128 intrinsic
  corpus (`__builtin_fabsf128`, `__builtin_copysignf128`, unary minus, 24 live
  f128 locals in one frame, nested intrinsic calls) before relying on it.
* Gate `tests/regression/check_cross_target_intrinsic_trap.sh` rewritten:
  (1) nine intrinsic **families** (`aesenc` 128, `pclmulqdq` 128, `crc32qi`,
  VAES-256, VPCLMULQDQ-256, `_mm256_add_ps`, `_mm256_mul_pd`,
  `_mm256_broadcastsi128_si256`, and the no-operand `_mm256_zeroupper` the
  AArch64 catch-all used to swallow) must each produce their own
  "not available on <arch>" diagnostic at `-O0` and `-O2` on AArch64 and
  RISC-V — nine families, nine independent rejections, counted;
  (2) portable builtins (`fixture .../portable_only.c`) still lower for all
  four targets; (3) x86 targets that *have* the extension still emit `aesenc`;
  (4) the safety net's trap still assembles and encodes
  (`0xd4200000` / `0x00100073`, read out of the ELF section table).

### Latent defect found by the full gate run (FIXED)

`backend::x86::codegen::peephole::passes::whitespace_invariance` collected its
corpus from `tests/` **and `work/`** — a directory that `.gitignore` lists.
Removing `work/` (AUD-HYGIENE-1) dropped the corpus below the test's vacuity
threshold, which is how the dependency surfaced: an invariant test whose inputs
can be deleted by a cleanup is not evidence. The second root is now
`engineering/evidence/godbolt/corpus` (committed GCC/Clang/ICX output — also the
more honest corpus, since asm-diff feeds the peephole foreign assembly).

---

## 2. Verified-open items (start here)

Ordered by value ÷ cost, with the entry point each one needs.

1. **AUD-CPU-3 remainder — the feature table is 11 rows for 268 x86 builtin
   variants.** `x86_intrinsic_required_feature` (`src/ir/lowering/expr_builtins.rs`)
   covers CRC32, the six 128-bit AES ops and `pclmulqdq128`; the audit names
   256-bit VAES/VPCLMULQDQ, `X86Crc32Load*`, SHA/GFNI/AVX-512 as missing, and
   the family sweep above now proves that at least VAES-256/VPCLMULQDQ-256
   reach lowering *without* a permission check on x86. Method that keeps it
   honest: `x86_features::X86Feature` has no `Sha`/`Gfni`/`Avx512*` variants
   yet (add them), the source-name → `BuiltinIntrinsic` mapping is a literal
   `m.insert("name", …)` table in `src/frontend/sema/builtins.rs` (624 rows,
   mechanically extractable), and the verdict for each row can be
   oracle-checked against `gcc -march=x86-64 -S` (error ⇒ the row needs a
   feature), exactly the way `check_intrinsic_feature_gate.sh` does it today.
2. **AUD-ICE-1 remainder — 889 panic-class sites (109 in the hot set).**
   Worklist: `python3 scripts/audit_census.py --top-fns` for size and
   `grep -nE 'panic!|unreachable!|\.expect\(|\.unwrap\('` per file for sites.
   Hottest files: `x86/codegen/intrinsics.rs` (47), `slp_vectorizer.rs` (32),
   `x86/assembler/parser.rs` (29), `backend/traits.rs` (27), `pgo/mod.rs` (22),
   `ir/lowering/init_canon.rs` (13), `frontend/parser/declarators.rs` (13).
   The audit's acceptance test still to write: an ill-typed-builtin gate (a
   new script next to the audit census, driven by the same name table) — a
   table of ill-typed calls of *every*
   `BuiltinIntrinsic` (wrong arity, wrong type class, non-constant where a
   constant is required) at `-O0` and `-O2`, asserting exit 1, a
   `file:line:col: error:` prefix and the absence of `internal error`.
   AUD-DIAG-1 (`__builtin_shuffle` mask element class) folds into it, and
   `__builtin_shuffle` is already fixed for the arity/type cases.
3. **AUD-DEBUG-1 — `-g` still yields line tables only** (no
   `.debug_info`/type info, `-gsplit-dwarf` rejected). Minimal DWARF 5 for
   `-O0`: `DW_TAG_compile_unit/subprogram/formal_parameter/variable/base_type/
   pointer_type/structure_type` with `DW_OP_fbreg`; acceptance = gdb
   `bt`/`info locals`/`print` on a three-function sample, `readelf
   --debug-dump` clean, `llvm-dwarfdump --verify`. Below the performance work
   in priority, but it is the difference between "can be debugged" and not.
4. **AUD-EXPRID-1 — expression identity is a heap address.** `ExprId` wraps
   `&Expr as *const _ as usize`; `expr_ctype_cache` (`ir/lowering/lower.rs`,
   `expr_types.rs`) detects address reuse by comparing the `Expr` *variant*
   only, so a dropped `Expr::Binary` whose address is reused by another
   `Expr::Binary` of a different type can return a stale `CType` (silent
   wrong-type miscompile). No reproducer yet: step 1 is one — many
   `typeof(a+b)`/`_Generic` nodes of differing types in a function whose AST
   nodes are dropped between lowering steps, with a poisoning allocator behind
   a `CCC_` knob. Step 2 is the real fix: a parser-assigned `u32` counter in
   each `Expr` (call sites already use `expr.id()`), deleting the variant check
   and the two TODOs.
5. **AUD-ARM-GSF-1 — a parked ARM peephole miscompile.** Global store
   forwarding "produces wrong output" in complex float-array code and is
   skipped; four tests are `#[ignore]`d (`arm/codegen/peephole.rs`), and two
   x86 tests are ignored as "trampoline coalescing not triggering". The census
   now ratchets the *count* (baseline 1, because the ARM ones carry an
   annotation); either fix and re-enable, or delete the pass and its tests.
6. **AUD-MONO-1 — 192 functions over 300 lines, 23 over 1,000.** Split
   behaviour-preservingly, one function per change, proved by the repo's own
   identity gates (`check_codegen_refactor_identity.py`, encdiff/asmdiff,
   linker suite). Start with `emit_intrinsic_impl` (5,839) and
   `encode_mnemonic` (3,056), which are also where the ICE/SILENT-ZERO fixes
   land — splitting them first makes both easier.
7. **AUD-KNOB-1 — 498 `CCC_*`/`LCCC_*` literals, 292 with no documentation.**
   `python3 scripts/audit_census.py --list-undocumented-knobs` is the list. The
   audit's cost claim (≈550 `env::var` reads outside `LazyLock`/`OnceLock` in a
   compiler that lowers on a worker thread) is **unmeasured**: measure first
   (`ltrace -c -e getenv` on `sqlite3.c`) before building the registry.
8. **AUD-CI-1 remainder** — `ci.yml` is still one ~1,430-line `test` job
   (first failure hides every later gate, nothing runs in parallel), no
   `permissions:`/`concurrency:`/`timeout-minutes`, actions pinned to mutable
   tags, `runs-on: ubuntu-latest`. `scripts/check_ci_gate_parity.py` encodes
   the contracts that a split must preserve — run it after every step of the
   split, do not reorder it into the same commit.
9. **AUD-DOC-1 remainder** — `engineering/STATE.md` mixes GCC 14.2 numbers with
   GCC 16.2 oracle pins and references absent paths; 69 `/home/user`
   occurrences across 30 scripts need one shared env-with-auto-detection
   helper; four snapshot flavours still exist (this session used
   `scripts/lccc-snapshot.sh` as canonical, per the audit's own declaration);
   `scripts/README.md` should note that `-o -` and the temp-file behaviour are
   fixed as of AUD-DRV-1/AUD-TMP-1.
10. **AUD-BL-1** — the backlog file itself (upstream `PBT_TRANSPLANT.txt` /
    its backlog section) still mixes closed and open items and has a duplicate
    `IVOPTS-1`; each open entry should carry front matter naming its
    reproducer, with a gate that the path exists.
11. **`sv_assign` miscompile (segfault at `-O2`)** — carried over from the
    audit digest; not in the audit's own item list, but it is a real
    correctness bug with a reproducer in the backlog.

---

## 3. Environment and harness rules this session had to relearn

* **The harness wipes `/home/user/artifacts`, `/opt/rust` and
  `lccc/target` and re-clones the repository.** Working-tree files survive,
  including uncommitted edits. Recovery: `git fetch origin <branch>:refs/remotes/origin/<branch>`
  then `git reset --mixed -q <ref>` — **never** `git checkout`/`--hard`, which
  would discard the surviving edits.
* **The Rust toolchain is not part of the image.** `pip download
  arena-rust-toolchain{,data1,data2,data3}==1.97.0`, concatenate the three
  parts (296,984,961 B), `zstandard`-decompress to a 932,915,200 B tar, extract
  to `/opt/rust` (`sudo mkdir -p /opt/rust && sudo chown user:user /opt/rust`).
  `arena_rust_toolchain.install()` cannot work: it hardcodes a
  `arena_rust_toolchain_data4` that is not on PyPI, and it needs a `zstd`
  binary the host does not have. Recipe: `/home/user/tools/restore_toolchain.sh`
  (+ `zstd-shim.py`). Build with `PATH=/opt/rust/prefix/bin:$PATH`.
* **Swap is mandatory**: `sudo fallocate -l 8G /swapfile`, `mkswap`, then
  `sudo /sbin/swapon /swapfile` (`swapon`/`mkswap` are not on `PATH`).
* **After a wipe the snapshot script anchors `BASE` at `HEAD`** (its
  `artifacts/.base_ref` is gone) and silently publishes a zero-byte patch. Pass
  `LCCC_BASE_REF=7e9444174f9b76181504329cd4ff8010b1a1483b`, and never accept a
  shrinking patch.
* **`scripts/lccc-snapshot.sh` is gate-aware**: it refuses to publish unless
  `ci_local.sh --fast` passed on *that exact tree* (`target/ci_local.pass`,
  keyed by tree hash). Budget a full run (~40 min) before every snapshot;
  `LCCC_SNAPSHOT_ALLOW_PARTIAL=1` exists for an interim autosave.
* `PyYAML` is needed by `scripts/check_ci_gate_parity.py`
  (`pip install --break-system-packages pyyaml`); without it the gate fails
  cleanly instead of silently passing.
* Gate files must avoid `producer | grep -q` and `producer | head -N` under
  `set -o pipefail` (the repo has `scripts/check_pipefail_sigpipe.py`, which
  caught exactly that in the new IVSR gate): capture first, or use
  `grep -c … >/dev/null` / `sed -n '1,Np'`.
* A commit message is checked against the tree at that revision
  (`scripts/check_pr_claim_fidelity.py`): naming a path that the same commit
  *moved* fails the gate, so describe moves without the old path.
* `tests/regression/*.c` is globbed **non-recursively** and each file must be a
  self-contained program; multi-file fixtures belong in a subdirectory (this
  is also what `run_regression.py`'s own failure text says).
