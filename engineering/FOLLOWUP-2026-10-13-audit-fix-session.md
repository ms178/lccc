# FOLLOWUP — 2026-10-13 audit-fix session (AUD-* series, part 1)

Upstream base: `ms178/lccc` main @ `7e94441` (the commit the Claude audit
`AUDIT-2026-10-13` was measured on).  All work of this session is one patch
series, delivered as `ms178-1.patch` (see the snapshot ledger in the workspace).

## 1. What was delivered, with evidence

| # | Audit item | Status | Evidence |
|---|---|---|---|
| 1 | **AUD-TMP-1** temp files predictable / written through symlinks | **FIXED** | `src/common/temp_files.rs` reserves an atomically created 0700 directory per driver invocation (`TempFile::new` is fallible, `O_EXCL`-equivalent); `CCC_ASM_DEBUG` dumps use the same mechanism. Gate the temp-symlink isolation gate: the audit's symlink reproducer leaves `victim.txt` byte-identical, the compile still works, a poisoned `TMPDIR` gives a normal driver error, and no temp directory leaks. Mutation-checked: the pre-fix compiler overwrites the victim. |
| 2 | **AUD-CPU-1** `__builtin_cpu_supports` ignores `-march`/`-m*`/target | **FIXED (final revision: runtime model)** | The first revision made the *fold* follow the effective feature set; the oracle then showed the fold itself is the defect — GCC 16.2, Clang 23.1 and ICX all lower the builtin to a runtime CPU-model lookup in every flag set (including target-guaranteed features and explicit `-mno-avx2`) and all reject the four constant contexts a fold makes legal. LCCC now always emits the runtime probe (`src/common/cpu_probe.rs`, x86-64 + i686 PIC/absolute, xstate gates, max-leaf guards, CFI, fail-closed), diagnoses all four constant contexts with GCC's wording, hard-errors unknown names with GCC's wording, and accepts a strict superset of GCC's 93-name set (6 added: `avx10.1`, `avx10.2`, `clzero`, `movrs`, `ptwrite`, `wbnoinvd`). Gate `tests/regression/check_cpu_supports_runtime.sh` (census⇔source, 93-name acceptance, 4 constant-context diagnostics, runtime differential vs GCC in 3 flag sets, assembly contract, i686 + AArch64/RISC-V rows); census recorded in `tests/data/cpu_supports_name_census.json`, evidence in `engineering/cpu-supports-runtime-model.md`. |
| 3 | **AUD-ICE-1** invalid user code reaches `panic!` | **FIXED (builtin family)** | `__builtin_shuffle` / `__builtin_shufflevector` panics became located diagnostics with GCC's wording; the rejected call lowers to a shape-correct placeholder and the driver aborts on the error before any pass. Gate `scripts/check_ill_typed_builtins.py` (13 rows at `-O0`/`-O2`, GCC agreement, `--self-test`), mutation-checked (every row panics on the pre-fix compiler). Remaining panic-class sites in `frontend/`/`ir/lowering/`/`driver/`: see §2.1. |
| 4 | **AUD-DIAG-1** `__builtin_shuffle` accepts a non-integer mask | **FIXED** | Mask element class (integer) and element-count agreement are validated in the lowering; a float-vector mask is rejected with GCC's message. Same gate as #3. |
| 5 | **AUD-SILENT-ZERO-1** x86 intrinsics on ARM/RISC-V compile to zeros | **FIXED (rejection half)** | Every x86 intrinsic passes through `lower_x86_intrinsic`; on a non-x86 target it now reports `'__builtin_ia32_aesenc128' is not available on aarch64` instead of lowering to a silently zeroed result. Same gate as #3 (aarch64 row). Removing the now-unreachable zero-emitting code in `backend/{arm,riscv}/codegen/intrinsics.rs` is §2.2. |
| 6 | **AUD-DRV-1** `-S -o -` writes a file named `-` | **FIXED** | `Driver::write_text_output` treats `-` as stdout for `-S`, `-E`, `-M`/`-MM`, `-dM`; `-MF -` sends the make rule to stdout; object/executable modes reject `-o -` with an explanatory diagnostic. Gate the `-o -` stdout gate (7 checks incl. a real pipe and a negative control). |
| 7 | **AUD-RATCHET-1** census not wired into CI | **FIXED** | New `scripts/audit_census.py` (+`scripts/audit_census_baseline.json`, `--check`, `--update`, `--top-fns`, `--selftest`) counting panic-class sites, `unsafe`, 300/500/1000-line functions, distinct and undocumented `CCC_*`/`LCCC_*` knobs, and `#[ignore]`s; wired as two fast gates in `ci_local.sh` and a CI step. |
| 8 | **AUD-HYGIENE-1** tracked files that should not be in the repository | **FIXED** | `.claude/settings.local.json` (blanket shell auto-approval in every clone), the leaked `.base_ref` and the scratch `work/` removed and ignored; loose `tests/*` fixtures moved to `tests/regression/fixtures/ivsr/` (documented) and `verify_fib_perf.sh` to `scripts/` (path fixed); the two reproducer file names containing `[`/`]`/`=` sanitized; `artifacts/repros/README.md` documents provenance and the replay contract. A replay gate now exists (§3). |
| 9 | **AUD-CI-1** CI workflow hardening | **PARTIAL** | `permissions: contents: read`, `concurrency` (cancel-in-progress), `runs-on: ubuntu-24.04`, per-job `timeout-minutes`. Action SHA pinning and the `test`-job split are §2.3. |
| 10 | **AUD-DOC-1** stale facts and harness assumptions | **PARTIAL** | `tools/workspace_dirs.sh` resolves the persisted workspace (`LCCC_WORKSPACE` → `/home/user` → `/mnt/user-data/outputs` → `$PWD`, loud failure) and `scripts/lccc-snapshot.sh` uses it; `snapshot_ms178.sh` and `lccc-harness-snapshot.sh` carry deprecation headers naming the canonical tool; `scripts/README.md` documents the new `-o -`, private-temp and cpu-supports contracts. The repo-wide `/home/user` burn-down and `STATE.md` regeneration are §2.4. |
| 11 | **FP-contraction parity** (not in the audit; found while replaying the corpus) | **FIXED** | `a*b+c` was fused into an FMA at `-O0`/`-O1`, where GCC never fuses (measured, gcc 14.2, with and without `-march=x86-64-v3`, with and without `-ffast-math`). `Driver::resolved_fp_contract()` now returns `Off` below `-O2` unless `-ffp-contract=`/`-ffast-math` is explicit; explicit flags still win at every level. Gate the FP-contraction parity gate (assembly level matrix + positive control at `-O2` + both flags + numeric agreement with GCC per level), mutation-checked. |

**Validation state.** `scripts/ci_local.sh --fast` is green on this tree (stamp
recorded in the snapshot ledger), the full regression corpus is green
(873 PASS / 0 FAIL / 8 SKIP), the 97-file reproducer corpus replays clean
(94 pass, 3 skipped for a missing Csmith harness, **0 failures** — 26 of them
only after the FP-contraction fix), and every new gate was mutation-checked
against a compiler built from the audit's commit.

## 2. Remaining work, in the audit's priority order

### 2.1 AUD-ICE-1, remainder — the rest of the panic-class sites
`scripts/audit_census.py` reports **917** panic-class sites overall and **109**
in `frontend/` + `ir/lowering/` + `driver/` (the audit's own census said 834 /
119 with its own stripping rules — the numbers are not comparable, the ratchet
is).  Plan, in this order:
1. `src/frontend/parser/declarators.rs` (13 `expect`s) and
   `src/ir/lowering/init_canon.rs` (12) — the same treatment as this session's
   builtin work: diagnostic with the node's span, `debug_assert!` for the true
   invariant, and a total fallback value.
2. `src/ir/lowering/expr_ops.rs:352`, `expr_assign.rs:981` (`unreachable!`
   reachable from source): reproduce each with a small ill-typed program first
   (the gate file is the natural home), then fix.
3. Extend `scripts/check_ill_typed_builtins.py`'s table to *every*
   `BuiltinIntrinsic` (wrong arity / wrong type class / non-constant where a
   constant is required) and lower the census baseline in the same commit.

### 2.2 AUD-SILENT-ZERO-1, remainder — delete the dead zero-emitting paths
`src/backend/{arm,riscv}/codegen/intrinsics.rs` still contain the "TODO:
consider emitting a runtime trap instead of silent zeros" arms and six
`unimplemented!` ICEs (`arm/.../intrinsics.rs:1020,1043,1060,1083,1119`,
`riscv/.../intrinsics.rs:509`, `riscv/.../f128.rs:373`, `i686/.../intrinsics.rs:1318`).
They are now unreachable from valid source (rejection happens earlier), which
makes them *deletable*: replace each with an explicit `error:` diagnostic path
or a real lowering, and add a `IntrinsicOp` × non-native-backend sweep to the
ill-typed gate. The "dest is not slot-homed" cases are RA-contract bugs — fix
the contract, do not trap.

### 2.3 AUD-CI-1, remainder
* Pin the third-party actions by commit SHA (the tags `checkout@v7`,
  `cache@v6`, `upload-artifact@v7`, `dtolnay/rust-toolchain@stable` are mutable
  supply-chain entries).
* Split the ~1,430-line `test` job into build → unit / corpus / asmdiff /
  encdiff / linker / kernel jobs sharing the build through an artifact, with
  `scripts/check_ci_gate_parity.py` updated in the same commit (it currently
  proves that the same *commands* run locally and in CI — a job split must keep
  that property, not weaken it).

### 2.4 AUD-DOC-1, remainder
* `grep -rn /home/user scripts` still finds ~30 scripts; migrate them to
  `tools/workspace_dirs.sh` (one script per commit, each verified by running it).
* `engineering/STATE.md` mixes "faster than GCC 14.2 across 39 benchmarks
  (0.8598)" with GCC 16.2 oracle pins and references an absent
  `/home/user/work/casefold.c`: regenerate it from the current benchmark
  artifacts, or date-stamp every number.

### 2.5 Feasibility-first backlog (audit items not started)
* **AUD-KNOB-1** — 475 `CCC_*`/`LCCC_*` literals, 284 undocumented;
  ~550 `env::var` reads outside `LazyLock`/`OnceLock` in a compiler that lowers
  on a worker thread. *Measure first* (`ltrace -c -e getenv` on `sqlite3.c`),
  then build the central knob registry as a new module under `src/common`
  (name, type, default, owner pass, kill-switch/debug/tuning class, one-line
  doc) with a generated knob reference under `docs/`, plus a CI check that no
  literal appears outside the registry. The census already
  ratchets the *count*, so the measurement can be a later commit without
  re-opening the debt.
* **AUD-MONO-1** — 196 functions > 300 lines, 77 > 500, 23 > 1,000
  (`python3 scripts/audit_census.py --top-fns 25`). Split behaviour-preservingly,
  one function per commit, proven by `scripts/check_codegen_refactor_identity.py`
  + encdiff/asmdiff + the linker suite. Start with `emit_intrinsic_impl`
  (5,839 lines) and `encode_mnemonic` (3,056), since AUD-ICE-1/SILENT-ZERO-1
  fixes land there anyway.
* **AUD-EXPRID-1** — `ExprId` is a heap address; `expr_ctype_cache` guards only
  the discriminant. Reproducer first (poisoning allocator or a
  `CCC_`-gated address-recycling hook), then replace `ExprId` with a
  parser-assigned `u32` stored in each `Expr`.
* **AUD-ARM-GSF-1** — the parked ARM global-store-forwarding peephole
  (`arm/codegen/peephole.rs:649`, 4 ignored tests) and the two x86 trampoline
  coalescing tests: reproduce, root-cause, then fix-and-re-enable or delete the
  pass together with its tests. `ignores_all` in the census ratchet (currently
  6) keeps the count from growing.
* **AUD-DEBUG-1** — `-g` emits line tables only (no `.debug_info`): implement
  minimal DWARF 5 for `-O0` (compile_unit/subprogram/formal_parameter/variable/
  base_type/pointer_type/structure_type with `DW_OP_fbreg` locations), gated by
  `gdb bt`/`info locals` on a 3-function sample, `readelf --debug-dump` and
  `llvm-dwarfdump --verify`. Below the performance work in priority, but it is
  what `pahole`/BTF tooling needs.
* **AUD-DUP-1** — `expand_rept_blocks` exists three times (shared + two private
  copies) and the inline-asm constraint parser is duplicated per backend:
  unify behind the encdiff/asmdiff corpora, then add a census check for
  same-named free functions across `backend/*/`.
* **AUD-BL-1** — the 2,000+-line `engineering/agent/BACKLOG.md` still mixes
  `FIXED`/`CLOSED`/`DONE` entries into its open sections and duplicates
  `IVOPTS-1`; move closed entries to `engineering/journal/`, merge the
  duplicate, and add the promised gate (every non-closed entry names an
  existing reproducer path).

### 2.6 Performance track (the actual goal)
The measurement infrastructure is in place; this session's correctness work
removed the false positives that made the FP corpus unusable, which is a
prerequisite for it. Next concrete steps:
1. `scripts/codegen_oracle.py --rank` over `tests/benchmark/programs/*.c` and
   the extracted workload kernels (zlib-ng, gzip, expat are already in-tree),
   using the local fastbuild binary plus the CE oracles (gcc16.2, clang, icx,
   icc). Store the ranking as the baseline for the next session.
2. `scripts/bench_kernels.py` for wall-clock A/B on the same kernels (no PMU on
   this host; the harness interleaves arms and fingerprints the timed
   function, so it is the honest alternative).
3. Then attack the worst gaps in `instruction count` first — the oracle is
   cheap and deterministic — and only promote a change to the runtime harness
   once the assembly-level gap actually closes.

## 3. Method notes worth keeping (so the next session does not relearn them)

* **Compare against the *same* target.** LCCC's absent-`-march` default is
  x86-64-v3; a GCC arm without `-march=x86-64-v3` will report every FMA/AVX2
  difference as a miscompile. `scripts/repros_replay.py` now encodes this
  (`GCC_ISA`), and it is why 26 "miscompiles" turned out to be an -O0
  contraction mismatch.
* **A corpus that is not replayed is not evidence.** The replay gate is fast
  (seconds for 97 files) and it is the only gate that exercises the historical
  crash/miscompile set end to end; it caught the FP-contract defect on the first
  run.
* **Mutation-check every new gate.** Each gate this session was run against a
  compiler built from the audit's commit and had to fail there. A gate that
  cannot fail is worse than no gate, because it reads as coverage.
* **Fix the shared primitive, not the symptom.** The temp-file fix, the
  source-manager sharing and the ISA-feature publication each fixed a whole
  class (arbitrary file write via temp names; location-less diagnostics;
  dispatch folds) rather than one call site.

## 4. Performance baseline measured this session (the research oracle)

Two artifacts are in the delivered workspace (`artifacts/`):

| artifact | what it is |
|---|---|
| `codegen-rank-baseline.json` / `.md` | `scripts/codegen_oracle.py --rank` over all 57 `tests/benchmark/programs/*.c` with the local fastbuild binary and the CE oracles (gcc16.2, clang, icx, icc), `-O2 -march=x86-64-v3` |
| `bench-kernels-baseline.json` | `scripts/bench_kernels.py --reps 5` wall-clock A/B against gcc/clang on the 15 hot kernels in `tests/bench/` |

Instruction-count ranking (screening metric, 110 functions): **74 behind, 10 tied,
26 ahead**, total gap 2,215 instructions.  The eight rows flagged "not
comparable" (different call counts) are the interesting ones: e.g.
`expat_siphash24:siphash24` shows 216 lccc instructions vs clang's 76, but the
call counts differ (1 vs 0) because clang **inlined** `sip24_init/update/final`
into it while lccc did not — so the row is an *inlining/cost-model* signal (plus
the stack-homing of the address-taken `struct siphash` state that follows), not
an instruction-selection defect.  That is the audit's §18/§25 area.

Runtime (best-of-5, interleaved, no PMU on this host):

| kernel | lccc ms | gcc ms | clang ms | vs gcc |
|---|---:|---:|---:|---:|
| classify | 21.36 | 11.54 | 9.11 | 0.54x |
| seg_gs_copy | 2.44 | 0.96 | 0.97 | 0.39x |
| seg_gs_fill | 1.41 | 0.76 | 0.46 | 0.54x |
| seg_gs_sum | 0.90 | 0.42 | 0.23 | 0.46x |
| varint | 51.02 | 32.54 | 35.09 | 0.64x |
| strlen_scan | 75.07 | 55.84 | 79.18 | 0.74x |
| adler32 / adler32_do8 / hashmix / memchr | — | — | — | 1.00–1.04x |
| **geomean** | | | | **0.765x** (vs clang 0.663x) |

**Highest-value lead:** all four `seg_gs_*` kernels (segment-relative GS
addressing, the kernel's per-CPU access shape) are 1.9–2.5x slower than GCC and
2.5–4.0x slower than clang, and they are *the same family*.  They are small,
self-contained, already gated (`tests/bench/k_seg_gs_*.c`, plus
`tests/regression/check_seg_*` and `scripts/check_seg_*`), so they are the ideal
first target for the next performance session: dump both compilers' assembly for
`k_seg_gs_sum`/`k_seg_gs_copy`, find the systematic difference (likely
redundant segment reloads or a missing address-mode fold), and only then touch
the pass.  `classify` (0.54x) and `varint` (0.64x) are the next two.

Nothing in this session changed code generation for optimized code — the
FP-contract change only affects `-O0`/`-O1` **defaults** (and explicitly matches
GCC), and the cpu-supports change only affects compile-time folding of a
builtin — so these baselines are directly comparable to the next session's
measurements.

## 6. Part 1c — probe hot path, GOT spelling, and what the red-team pass found

### 6.1 The probe paid a call per use; now it pays a branch

`__builtin_cpu_supports` initialised the probe words with an unconditional
`call __lccc_cpu_init` at the end of the entry block, so *every* probe
execution paid the call, the callee prologue and the caller-side register
save: 27 instructions per probe call in a leaf function
(`tests/workloads/dispatch/two_way_dispatch.c`, callgrind, x86-64, `-O2`).

`splice_cpu_probe_init` now splits the entry block at the first probe-word
read into `entry → (ready ? hot : cold)`, `cold: call __lccc_cpu_init; goto
hot`, `hot: <first read and everything after>`.  The guard dominates every
read, the call runs at most once per program, and the hot path is
`load ready / test / je`.  An entry block whose moved range carries phis keeps
the old unconditional call (phi operands name their predecessor block).

Measured afterwards, same workload and host: probe 1 200 117 866 vs control
(a volatile flag read) 700 117 783 over 50 M calls = 10 instructions per call
of residual overhead — 4 of them the frame the (still reachable) call forces
on the function, 3 the guard, 3 register shuffling.  GCC 14.2 and Clang 19.1.7
measure 0 (constructor + plain `__cpu_model` load); GCC 16.2 and Clang 23.1
emit exactly that shape in every oracle flag set, so the comparison is a
property of their design, not of the version.  One-time init: 117 867 (LCCC,
lazy) vs 121 045 (GCC, constructor).  Numbers, method and the honest
decomposition: `engineering/cpu-supports-runtime-model.md` §4.

Pinned by `check_cpu_supports_runtime.sh` §6 (one initialiser call per probing
function, each behind a conditional jump on `__lccc_cpu_ready`) and by the new
`tests/regression/builtin_cpu_supports_guard.c` (eight function shapes, GCC
differential).

### 6.2 Two defects the measurement/test pass found in our own work

1. **`builtin_cpu_supports_{default,raptor}.c` could not compile.**  Both
   looped over a name array (`__builtin_cpu_supports(names[i])`), which every
   one of GCC/Clang/LCCC rejects ("parameter to builtin must be a string
   constant or literal"); `run_regression.py --filter builtin_cpu_supports`
   showed 2 failures with the old files.  Rewritten with one literally-spelled
   probe per name; the filter is 3/3 green now.  This is the same trap that
   broke the competing PR #810's CI, so it is worth stating plainly: a corpus
   test must be a self-contained program with literal builtin arguments.
2. **`R_X86_64_GOTPCRELX` was emitted for instruction forms no linker may
   relax.**  The assembler chose the relaxable spelling from the REX prefix
   alone (a comment even claimed that was GAS's rule).  GAS 2.47 decides on
   the *instruction form*: `mov r,m`, `test`, the eight binops in both
   directions and indirect `call/jmp/push` get the relaxable spelling; stores,
   `lea` and immediate forms get plain `R_X86_64_GOTPCREL` even with a REX
   prefix (`gas/config/tc-i386.c`, `output_disp`).  Fixed in
   `gotpcrel_form_relaxable` + `gotpcrel_x_type`; the seven-form battery is now
   an exact match for the pinned GAS 2.47 oracle, and the gate
   `tests/regression/check_got_slot_local_addend.sh` compares against that
   oracle instead of a table in a comment.

### 6.3 New gate: GOT slots of local symbols, values *and* spelling

`tests/regression/check_got_slot_local_addend.sh` runs
`tests/data/got_slot_local_addend_{x86_64,i686}.s` — two locals of nonzero
offset read through their GOT slots, returning 3*5 + 5*2 = 25 — through five
pipelines (LCCC driver x86-64 default/-no-pie, i686 default/-no-pie, and the
LCCC object linked by GNU ld, the pipeline that isolates the assembler from the
LCCC linker; the fold defect showed up there as 145).  It then checks the
relocation spelling, pinned and (when present) against the GAS 2.47 oracle.

### 6.4 Open, analysed, not fixed: i686 keeps `R_386_GOT32` everywhere

GAS 2.47 emits the relaxable `R_386_GOT32X` for the i686 `mov`-load, binop,
`test` and indirect `call/jmp/push` forms and `R_386_GOT32` elsewhere; LCCC
emits `R_386_GOT32` for all of them.  The relocations are *correct* — every
pipeline in the gate returns 25 — so this is a size/relaxation opportunity
(a GOT entry and a load the linker may remove), not a miscompile.  Closing it
needs both halves at once: the assembler must offer the relaxable spelling and
`lccc-ld` must then handle it for every form that can carry it (today its
relaxation rewrites only the `mov` shape and writes the GOTOFF value
otherwise; for a local symbol with no GOT slot that is right for a load and
wrong for a store).  Recorded here rather than half-done.

### 6.5 Constant-expression contexts: verified, not assumed

Re-measured after the rebase on `6495e6eb`, GCC 14.2 vs this tree, all four
contexts and both scopes: file-scope `int v = rand();`, block-scope
`static int v = rand();`, file-scope `int a[rand()?2:1];` and
`static long w = time(0);` are rejected with GCC's exact wording
(`initializer element is not constant`, `variably modified 'a' at file
scope`).  The gate covers the same four contexts for the builtin itself.

## 5. Part 1b — rebase + follow-ups (same session, later)

* **Rebase.** The series was rebased onto the upstream tip
  (`7e944417` + `6495e6eb` "Add safe CFG threading and exact FP recurrence
  reuse"); nothing in that commit touches the files this series changes, and
  the rebase was conflict-free (`git cherry-pick -x`).
* **Defect found while implementing AUD-CPU-1 and fixed here:** the i686/x86-64
  assembler folded a *local* GOT-slot reference (`R_386_GOT32` /
  `R_X86_64_REX_GOTPCRELX` against a `.bss`/`.data` symbol) into a direct
  section reference, so the linker added the symbol offset twice. The asserting
  probes (`q1=25` expected) produced 145 with the pre-fix assembler under
  *every* linker. Fixed in `src/backend/elf_writer_common.rs`,
  `src/backend/generation.rs` and both `assembler/elf_writer.rs` backends.
* **New open item — AUD-CPU-3 (intrinsics are not ISA-gated).** `_mm256_add_epi8`
  under `-march=x86-64` emits `vpaddb` where GCC errors
  (`inlining failed in call to 'always_inline' '_mm256_add_epi8': target
  specific option mismatch`); same for `_mm_crc32_u8` (SSE4.2). 268
  `BuiltinIntrinsic::X86*` variants need an oracle-derived required-feature
  table and a lowering-time check; a partial gate was rejected as worse than
  the documented hole. Reproducer + design sketch:
  `engineering/cpu-supports-runtime-model.md` §7.
* **New open item — `__builtin_cpu_is` is unimplemented** (GCC provides it for
  CPU-family names). Recorded so the name space is closed deliberately.

## 7. Part 1d — byte_load extension, and three defects measurement found

Same session, after the AArch64/RISC-V oracle work.  Full narrative and tables:
`engineering/byte-load-reassembly.md`.

* **Landed:** `byte_load` folds big-endian runs (`load` + `bswap`) and runs
  narrower than their carrier (16-bit field promoted into `int`; `load u16` +
  zero-extending `cast`), and runs on AArch64 as well as x86-64/i686.  x86-64
  per-function counts against the oracles: `le16` 6→2, `be64` 24→4 (Clang
  23.1's scalar chain is 12), `le32`/`le64`/`body_le64` 2 → 2.  Callgrind
  82,382,843 vs 175,062,853 instructions on one binary, identical output.
* **Defect found and fixed in our own work (this is the one to remember):** the
  big-endian/narrow rewrite first loaded through the *resolved root* of the
  address expression for any run whose lowest byte was at the lowest offset —
  true by construction — so a run inside an object (`f->body`) read the
  object's first byte.  The benchmark kernel's known-answer test caught it
  (exit 6); the unit suite could not, because no fixture had ever placed a run
  at a nonzero offset.  Fixed by loading through the run's own byte pointer +
  `consume_dead_chain`; covered by `folds_a_run_at_a_nonzero_offset` and the
  executable footprint invariant `assert_loads_stay_in_footprint`.
* **Open — AUD-FRAME-1 (i686): parameter homes are callee-saved.** Every
  i686 function with a parameter pays `pushl %ebx; subl $8, %esp; …; popl
  %ebx`, i.e. 7 instructions for `int twice(int)` against GCC's 3, and 7 for
  `int deref(const int*)` against 3.  Functions without parameters are clean
  (`int con(void)` → `movl $3, %eax; ret`), so the trigger is the parameter
  home, not the frame machinery.  Reproduce: `lccc-i686 -O2 -S` on a
  two-function TU; expected shape is GCC's `movl 4(%esp), %eax` + body.
* **Open — AUD-FRAME-2 (AArch64): unconditional frame + dead callee-saved
  saves.** `int deref(const int *p)` is 7 instructions
  (`stp x29,x30,[sp,#-32]!; mov x29,sp; str x19,[sp,#16]; ldrsw x0,[x0];
  ldr x19,[sp,#16]; ldp x29,x30,[sp],#32; ret`) against GCC's 2, with `x19`
  saved and reloaded without ever being read; `int twice(int)` is 9 against 2.
  Visible in every function, so it is 4–7 instructions per function across the
  architecture.  Both items are location-assignment/prologue policy; the
  allocator's own notes warn that small edits there move benchmarks in both
  directions, so they need a paired-A/B session, not a drive-by patch.
* **Open — `movbe` peephole (x86 backend).** `load uN` + `bswap uN` on the same
  value should become one `movbe` where the target has it: GCC 16.2 and ICX
  both emit `movbe` for `be64` (2 instructions) where LCCC emits `movq` +
  `bswapq` (4 including the return move) and Clang 23.1 emits 12.  This is the
  whole remaining `be64` gap and it is a backend peephole, not a pass change.
* **Oracle harness defects found while measuring cross-architecture (all
  fixed, `tests/corpus/test_oracle_arch_guard.py`):** `--arch riscv64` with an
  x86 compiler id silently labelled x86 assembly as riscv64 (now refused by
  `_check_arch`); the x86 default flags were handed to the AArch64/RISC-V
  channels so every cross-architecture oracle failed to compile (now per-arch
  defaults); the per-architecture oracle sets pointed at dead ids and were
  outside `godbolt.py audit`, so nothing noticed (now named aliases, audited
  and current: arm64gcc 16.1.0, armv8clang 23.1.0, rv64gcc 16.1.0, rv64clang
  23.1.0).

## 8. Part 1e — MOVBE closed, and two claims red-teamed out

Same session, after Part 1d.  Full tables: `engineering/byte-load-reassembly.md`.

* **Closed — the `be64` gap.** `passes/load_bswap_movbe.rs` fuses
  `mov{w,l,q} MEM,%T; bswap{w,l,q} %T` into one `movbe{w,l,q} MEM,%T` and
  coalesces the allocator's copy of the result, so `be64` is now
  `movbeq (%rdi), %rax; ret` = **2 instructions, matching GCC 16.2 and ICX and
  beating Clang 23.1 (12)**.  All five kernel functions are 2/2/2/2/2 against
  2/2/2/2/2 for the best oracle.  `X86Isa::movbe` carries the permission
  (outside the SSE chain; `-mno-sse` must not remove it), driven by
  `Driver::resolved_movbe`; the pass is switchable with
  `CCC_PEEPHOLE_SKIP=movbe_fuse` and takes `movbe` as a parameter rather than
  reading the process-global mirror (a unit-test race the first revision had).
  Behaviour matrix measured: default 2, `-march=x86-64` 4, `-mno-movbe` 4,
  skip 4.
* **Retracted — `sip24_final` 165 -> 149.** That pair (in the byte-load doc and
  in an earlier version of the S08 commit message) does not reproduce: the
  chain in `sip24_final` is split across one basic block per `case` label, so a
  within-block matcher cannot fold it.  Today's numbers are 165 -> 165, and
  `sip24_update` 101 -> 76.  The stale numbers were replaced, not left standing.
* **New open item — folds across a split chain.** The evidence is on disk
  (`expat_siphash24.c`, asm quoted in the doc): a run whose OR-tree is spread
  over fallthrough blocks, each the sole continuation of the previous.  The
  matcher is within-block today; extending it needs a label-free path proof
  (every block on the path has exactly the chain's continuation as predecessor)
  plus the same `uniform_in_window` base rule.  This is the largest known
  remaining win in the byte-load family and it is measured, not guessed.
* **New open item — RISC-V codegen on the same kernels.** With the oracle
  finally measuring real RISC-V channels, the same five functions are
  **24/51/111/111/113 instructions** for LCCC against 5-23 for GCC 16.1 and
  Clang 23.1 — a bigger gap than anything on x86-64, and one this pass
  deliberately does not close (no RISC-V oracle widens unaligned loads).  It is
  a scalar-codegen item: the byte chains are not being folded by anything else
  either, and the RISC-V backend's addressing modes are the place to start.

### 8.1 Open item added this session: the 16-bit big-endian shape (BE16)

`(uint16_t)((p[0] << 8) | p[1])` — the `ntohs`/`get_unaligned_be16` shape — is
declined by `byte_load` on purpose
(`refuses_a_big_endian_run_narrower_than_its_carrier`): the chain's carrier is
32-bit, so reversing needs a 16-bit `bswap` followed by a zero-extension.
Measured cost of the refusal, one probe file (`-O2 -S`, one function per shape,
same flags):

| shape | LCCC | `CCC_PEEPHOLE_SKIP=movbe_fuse` | `CCC_DISABLE_PASSES=byte_load` | local GCC 14.2 |
|---|---:|---:|---:|---:|
| `be16` | 6 | 6 | 6 | 3 |
| `be32` | **2** | 4 | 12 | 3 |
| `be64` | **2** | 4 | 24 | 3 |

Against the oracles (this tree, `-O3 -march=x86-64-v3`,
`deliverables/artifacts/oracle/byte-load-shapes.{json,log}`) the shape is **6 against 2 for
GCC 16.2, Clang 23.1 and ICX** — every channel answers with one `movbe`, because
all three have MOVBE at that `-march`; local GCC 14.2 (no MOVBE in its baseline)
reaches 3 via `movzwl` + `rolw $8`.  It is the only shape in the family where
LCCC is behind the best channel (the other six are at parity, `be64` 2 against
Clang's 12).

The implementation, so the next session does not re-derive it:

* the run is 16 bits inside a 32-bit carrier, so `byte_load` should emit
  `load u16` (which zero-extends into the carrier) + `bswap` at 16 bits; the
  reversal writes only the low half and the zero-extension is already there, so
  no explicit `zext` instruction is needed;
* that requires ISel to lower a 16-bit `bswap` to `rolw $8` (the refusal exists
  precisely because this path has no lowering today), and the `movbe` fold can
  then consume the pair into `movbew`, which the encoder already carries
  (`assembler/encoder/sse.rs`); on i686 the baseline has no MOVBE, so the
  correct output there is `rolw`;
* AArch64 needs the twin (`ldrh` + `rev16`) — the pass is enabled there — and
  RISC-V stays off, so the differential gate must cover at least
  x86-64/i686/aarch64 and must assert the byte footprint exactly like
  `assert_loads_stay_in_footprint` does for the wide case.

Not attempted here: the last commit of a session is the wrong place for a
width-sensitive ISel change, and the little-endian twin of exactly this corner
is where this session's miscompile lived.

### 8.2 Delivery-integrity rule added this session

`git diff --diff-filter=D <base>..HEAD` must be empty except where a document
argues the deletion.  This session found **25 upstream files** (21 under
`work/`, plus `.base_ref`, `.claude/settings.local.json` and two
`builtin_cpu_supports_raptor` fixtures) deleted by the branch as an artefact of
rebuilding the tree after a session wipe; all but the documented fixture pair
were restored.  Details: `REDTEAM-2026-10-13-movbe-and-byte-load.md` §1.6.
