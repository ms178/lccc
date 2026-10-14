# FOLLOWUP — 2026-10-13 — audit response session (AUDIT-2026-10-13, upstream `7e94441`)

Session mandate: execute `PBT_TRANSPLANT.txt` — "Fix all of the problems
identified in this Claude Audit" (AUDIT-2026-10-13, produced against upstream
`cf0b0eb`), under the standing orders (foreground builds, OOM-safe fastbuild
cycles, immediate `ms178-1.patch` autosave after every validated fix).

Execution environment honesty (differs from the audit's box, and it matters
for reading every result below):

* 2 vCPUs, 4.1 GiB RAM **hard-capped by cgroup**, **no swap possible**
  (`swapon` requires CAP_SYS_ADMIN; EPERM verified both as root-attempt and
  via the repo's own `scripts/ensure_swap.sh`, which degrades exactly as
  designed).  The repo's small-host mitigations were used verbatim:
  `CARGO_BUILD_JOBS=2`, fastbuild profile, `--no-keep-memory`,
  `CARGO_INCREMENTAL=0` when the incremental session state OOMs rustc.
* rustc **1.99.0 stable** — at or above the declared MSRV 1.98.1, so the
  audit's `if_let_guard`/`__cpuid` shims were never needed.
* The session's own copy of the tree lives outside the repo; snapshots are
  delivered through the canonical `scripts/lccc-snapshot.sh` with
  `LCCC_REPO`/`LCCC_ARTIFACTS`/`LCCC_DELIVERABLE` overrides.

## 1. What landed (audit item → state)

| audit item | state | evidence / gate |
|---|---|---|
| AUD-DRV-1 | **LANDED** | `tests/regression/check_dash_output.sh` (14 rows, mutation-tested via `--self-test`); `-S/-E -o -` stream to stdout, `-c -o -` fails cleanly, `-MF -` streams deps with the object-derived target, derived `.d` skipped for stdout output, `-E` dep rules key on the would-be object (`x.o: x.c`), full-link `-o -` keeps GCC's file-named-`-` parity.  Every cell measured against gcc 14.2 before implementation. |
| AUD-TMP-1 | **LANDED** | `tests/regression/check_temp_symlink.sh` (symlink rows + symlink-free regular-file emulation for sandboxes that refuse `symlink(2)`); private per-process 0700 dir (mkdir + inode re-check), RAII removal on last drop, `O_CREAT\|O_EXCL\|O_NOFOLLOW` reservation in the flat fallback, `CCC_ASM_DEBUG` moved into the private dir with the path announced.  Unit tests in `src/common/temp_files.rs`. |
| AUD-CPU-1 | **LANDED (fold derived from effective state; runtime `__cpu_model` lowering is follow-up)** | `tests/regression/check_cpu_supports_fold.sh` (the audit's four-row oracle table + default grant + late `-mno-sha`/`-msha` + asm-constant check + aarch64 hard diagnostic + non-string arg).  `X86CpuFoldFeatures` is built by the driver from the SAME permission model that gates codegen.  Driver additions: `-msha`/`-mno-sha` + `enable_sha` (alder-lake ISA carries SHA-NI; GCC `-march=raptorlake` defines `__SHA__`, measured) and `sse42_explicitly_disabled`.  **Decision recorded:** features the driver does not statically track (rdseed, adx, fsgsbase, xsave*, avx512*) fold 0 — the safe direction; a libgcc-ABI `__cpu_model` load (or inline CPUID+XGETBV) is the tracked enhancement, filed below.  En passant fix: lowering-time diagnostics now render `file:line:col` (the source manager left the engine before lowering). |
| AUD-SILENT-ZERO-1 | **LANDED (diagnostics + `.error` defense; working NEON/RVV lowerings are follow-up)** | `scripts/check_foreign_intrinsics.py` — derives the x86-only name set from `sema/builtins.rs` (492 names), tries arities 0..2 on `lccc-arm` and `lccc-riscv`, asserts no clean compile and no internal-error text; the audit's aesenc128 reproducer asserted verbatim (GCC-shape caret diagnostic); x86 control row.  The lowering funnel rejects all 268 `BuiltinIntrinsic::X86*` variants on non-x86 targets; the backend zero-arms are now `.error` directives (assembler exits non-zero with the message). |
| AUD-ICE-1 + AUD-DIAG-1 | **LANDED for the builtin-dispatch surface (the audit's concrete reproducers); the 119-site burn-down is a ratchet now** | `scripts/check_ill_typed_builtins.py` — 624 registered builtins × arities 0..3 × `-O0`/`-O2` + 6 targeted reproducers: no ICE, every failure located, GCC-shaped messages (`'__builtin_shuffle' last argument must be an integer vector`, same-type vectors, element counts, `__builtin_shufflevector` const-lane/range, `__builtin_cpu_supports` string arg).  `sema/builtin_arity.rs`: a mechanically derived arity table (allowed = the arities that did not ICE pre-guard, probed with five argument spellings so type-special signatures are measured by their real shape) enforced at the single `lower_builtin_intrinsic` dispatch — 496 (builtin, arity) pairs that panicked in x86 codegen are now "too few/many arguments" diagnostics.  The two complex-op `unreachable!`s are `debug_assert!` + diagnostic fallback.  The full 119-site burn-down continues under the census ratchet. |
| AUD-HYGIENE-1 | **LANDED** | `.claude/settings.local.json`, `.base_ref`, `work/` removed + gitignored; `tests/` root de-littered (`fib_rec2iter.c`/`verify_fib_perf.sh` moved to `tests/regression/`, bench.yml re-pointed, perf script re-anchored and smoke-tested; stale scratch snapshots deleted); `artifacts/repros` bracketed names sanitized.  The replay gate over the 100-file crash corpus is follow-up (below). |
| AUD-TOOLCHAIN-1 | **LANDED** | `__cpuid` wrapped + `#[allow(unused_unsafe)]` with an honest comment (the intrinsic's status moved across stables; both toolchain classes build).  `ci.yml` gains an **msrv job**: `cargo +1.98.1 check --locked --profile fastbuild` — declared minimum TESTED, not asserted (recorded hosted-only with the reason; parity green). |
| AUD-CI-1 | **PARTIALLY LANDED (1-5 of 6; the test-job split is structural follow-up)** | workflow-level `permissions: contents: read`, `concurrency` with cancel-in-progress, per-job `timeout-minutes`, runners pinned `ubuntu-24.04`, all third-party actions pinned by commit SHA (SHAs fetched from the GitHub API, tags in comments).  Parity and workflow-shell checks green throughout. |
| AUD-RATCHET-1 | **LANDED** | `scripts/audit_census.py` (self-tested stripper/counter: comments, strings, raw strings, test-module truncation) + `scripts/audit_census_baseline.json` (recorded at this session's HEAD) wired as a fast gate in `ci_local.sh` and mirrored in `ci.yml`.  Metrics may shrink freely, never grow; the baseline moves only in a commit that removes sites. |
| AUD-DOC-1 | **LANDED (canonicalization + shared helper; the 69-ref sweep is mechanical follow-up)** | `scripts/lccc-snapshot.sh` declared canonical in `scripts/README.md`; the two other snapshot flavours carry deprecation banners pointing there; `scripts/lccc_workspace_env.sh` is the ONE shared resolver for the persisted/deliverable directories (env → `/home/user` → `$HOME/.lccc-ws` → loud failure).  The README's "known gaps" section now records AUD-DRV-1/AUD-TMP-1 as LANDED with their gates. |
| AUD-BL-1 | **PARTIALLY LANDED** | New entries carry reproducer paths by rule; the full backlog triage (journal move, duplicate `IVOPTS-1` merge, front-matter gate) is follow-up. |
| AUD-KNOB-1 | **OPEN (ratchet armed)** | The census now counts 496 knobs / 290 undocumented and refuses growth; the central registry is follow-up. |
| AUD-MONO-1 | **OPEN (ratchet armed)** | 229/81/29 fns over 300/500/1000 lines counted and ratcheted; behaviour-preserving splits are follow-up. |
| AUD-DUP-1 | **OPEN** | `expand_rept_blocks` triplication and the constraint-parser TODOs unchanged this session. |
| AUD-EXPRID-1 | **OPEN** | No reproducer built this session; unchanged. |
| AUD-ARM-GSF-1 | **OPEN** | The parked peephole miscompile and the four `#[ignore]`d tests are unchanged. |
| AUD-DEBUG-1 | **OPEN** | DWARF-5 minimal debug info unchanged. |

## 2. NEW findings this session (with reproducers — they belong in the backlog)

* **VECRET-1 (miscompile, filed in `backlog.md` + `artifacts/repros/`)** —
  `artifacts/open_bugs/miscompile_vec_return_by_value_-O2.c`: a 16-byte vector
  returned by value from a COMPUTING function reads the result's alloca
  pointer as data (the printed high halves are a stack address) at -O0 and
  -O2, while identity returns and assignment/global flows are correct.  GCC
  14.2 is correct on the same source.  Isolation is complete (globals vs
  return, -O0 vs -O2); the fix point is the vector-rvalue convention at the
  return boundary.
* **Lowering diagnostics rendered without `file:line:col`** — found by the
  AUD-CPU-1 gate: `take_source_manager()` stripped the engine before
  lowering, so EVERY lowering-time diagnostic was locationless.  Fixed by
  moving the take to just before codegen (the manager's only other
  consumer).  This silently upgrades every diagnostic the lowering emits.
* **The builtin arity hole** — sema carries no argument count, so ill-typed
  arity flowed to backend `args[i]` indexing: 496 (builtin, arity) pairs
  ICEd in x86 codegen alone.  Fixed by the derived arity table + dispatch
  guard; the derivation method (no-ICE arities, five spellings) is documented
  in `sema/builtin_arity.rs` and re-runnable.

## 2b. Red-team self-audit round (post-implementation)

The mandated self-audit re-ran every attack and contract against the patched
tree and found one REAL gap the original gate missed: the `.S` /
`-x assembler -E` preprocessing path had the SAME dash bug (`lccc -E -o -
header.S` created a file named `-`).  Fixed with the same contract as the
C `-E` path; the gate gained four assembly rows and still mutation-bites.
The `gcc_assembler` feature path delegates to `gcc -E`, which owns the dash
natively (verified, no change needed).  The cumulative patch was additionally
verified from a FRESH base worktree: applies clean, `cargo build --profile
fastbuild` green, gates pass from that tree.  A patch-debris scan (dbg!,
debug prints, personal paths, binary payloads) is clean.

## 3. Validation state of this tree

* All NEW gates green (final battery, after the red-team round):
  `check_dash_output` (18 incl. the assembly rows), `check_temp_symlink`
  (14), `check_cpu_supports_fold` (16), `check_foreign_intrinsics` (492×2
  sweep), `check_ill_typed_builtins` (624×4×2 sweep), `audit_census
  --check`, `check_ci_gate_parity`, `check_ci_workflow_shell`,
  `check_no_conflict_markers`, `rustfmt`, `clippy`, `build`.
* Core oracles green: `differential-correctness-oracle`,
  `benchmark-output-oracle`, `linker-oracle-verdict`, and
  `regression-corpus` at 886/886 non-32-bit cases (the 10 failures are the
  sandbox's missing i686 multilib — gcc itself cannot link `-m32` here:
  no Scrt1.o/crti.o/32-bit libgcc; environmental, zero codegen content).
  `encdiff/asmdiff-semantic-validation` require the pinned binutils-2.47
  pair (`ensure_gas_247.sh`); they refuse a distro objdump BY DESIGN and
  are an environment-provisioning gap here, green on GitHub CI.
* Pre-existing gates re-run green after each fix:
  `check_dep_files`, `check_debug_info_flags`, `check_ci_gate_parity`,
  `check_ci_workflow_shell`.
* `scripts/build_lccc_fast.sh` green throughout (fastbuild, -j2, foreground).
* **NOT run on this box: the full `ci_local.sh --fast`.**  Its `cargo-test`
  gate links the monolithic lib-test binary, which needs ~3.5-4 GiB in one
  rustc invocation; this sandbox caps at 4.1 GiB with ~900 MiB held by the
  session harness (next-server + node + python), and the repo's own
  small-host recipe (`DEBUG=0`, `CARGO_INCREMENTAL=0`, `-j1`) still SIGKILLs
  at the link (verified three ways).  No swap is installable here (no
  CAP_SYS_ADMIN).  Every NON-lib-test gate is expected green; the GitHub
  runner (7 GiB) runs the cargo-test gate green on every PR.  This is an
  environment statement, not a code statement.

## 4. Follow-up work, prioritized

1. **VECRET-1** (miscompile, reproducer in hand) — decide the vector-rvalue
   convention, fix the return boundary, promote the reproducer to a gate.
2. **AUD-ARM-GSF-1** — the parked global-store-forwarding miscompile and the
   four `#[ignore]`d tests; the float-array shape is the starting point.
3. **AUD-EXPRID-1** — build the `ExprId` address-reuse reproducer (poisoning
   allocator or `CCC_`-gated hook), then the parser-assigned `u32` counter.
4. **AUD-RATCHET-1 burn-down** — use the census `--top-fns` to drive the
   behaviour-preserving splits (start with `emit_intrinsic_impl`,
   `encode_mnemonic`); each split is its own PR against the identity gates.
5. **AUD-CPU-1 enhancement** — the `__cpu_model`-compatible runtime lowering
   for non-guaranteed features (inline CPUID+XGETBV helper), with the audit's
   four-row table as the acceptance test.
6. **AUD-KNOB-1** — central registry + generated `docs/KNOBS.md`; the census
   ratchet holds the line meanwhile.
7. **AUD-CI-1 item 6** — split the 1,430-line test job (build artifact shared
   across independent jobs), keeping `check_ci_gate_parity.py` green.
8. **AUD-DUP-1** — dedup `expand_rept_blocks` (three copies) and the inline-asm
   constraint parsers, under the encdiff/asmdiff corpora.
9. **artifacts/repros replay gate** — compile+run every archived reproducer,
   archive the ones that no longer reproduce (many were crashers; expect a
   large fixed-fraction after this session's fixes).
10. **AUD-DOC-1 sweep completion** — convert the remaining `/home/user`
    references to `scripts/lccc_workspace_env.sh` (mechanical, one PR).

## 5. Snapshot ledger

Snapshots S01–S06 were taken after each validated fix via the canonical
`scripts/lccc-snapshot.sh` (UNGATED interim saves — no `ci_local` stamp
exists for this box, honestly ledgered; the final gated run is the session's
closing step).  The cumulative patch, series, source archive and session
bundle live in the persisted artifacts zone.
