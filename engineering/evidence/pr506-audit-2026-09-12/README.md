# PR #506 Red-Team Audit — x86 RORX, default-on loop idiom, width fixes, unroll retune

Date: 2026-09-12. Auditor: Arena agent (adversarial). Scope: `6f43fe45` vs `a0e03144`
(14 files, +396/−132). Method: full-diff read + gate-chain tracing to main +
liveness/escape/trap analysis. The PR ran **zero builds** ("No local build run");
every claim below was re-verified against the diff, not the description.

## Verdicts (agree / disagree + why)

| # | Slice | Verdict |
|---|-------|---------|
| 1 | RORX isel + MachInst + alloc + emit arms | **AGREE** — amount math provably sound (`rem_euclid`, Rol→Ror, nonzero gates); S32/S64-only; liveness `Read+PureWrite` correct; single ctor (isel); text syntax `rorxl/q $i,%s,%d` correct |
| 2 | RORX gate chain | **AGREE (wart)** — `bmi2=false→Never`, `bmi2→Always/WhenItSavesAMove` traced through `emit.rs:1186`, `cpu_model prefer_shlx/shlx_saves_move`, cli `-mbmi2`/v3/native. No SIGILL risk at baseline. WART: SHLX-conflated, no independent kill switch → perfection adds `CCC_NO_RORX` |
| 3 | RORX acc/legacy text paths (alu, emit) | **AGREE (wart)** — `src==dst` 3-operand form legal; amount-0 emits nothing = correct no-op (value already in acc; safer than `ror $0` which clobbers OF). WART: silent-nothing shape → perfection uses `debug_assert!` + unconditional emit |
| 4 | loop_idiom default-on + self-loops + Param/Other→memmove | **AGREE CONDITIONAL** — memmove predicate correct (memcpy kept only for provably-distinct Global/Alloca pairs); escape check (matcher ~L1160-1255) closes the exit-phi hole for the new body-BinOp arm; whole-loop DCE makes load/store order irrelevant. CONDITIONS (done in perfection): remove 3 dead fns, Div/Rem trap-bail, predicate simplification, stale-comment fix, test updates, full validation (suite+outputs+torture+fuzz+SSA) |
| 5 | loop_idiom body-BinOp tolerance | **AGREE (hardened)** — safe for escapes (proven above); latent trap-removal vector (dead `Div/Rem` DCE'd) predates the PR for header BinOps and widens to bodies → perfection bails on `Div/Rem` anywhere in header/body (zero recall cost: no copy idiom contains division) |
| 6 | bit_idioms `width>=64 → true` | **AGREE** — fixes debug-panic (`1u64<<64`) + release wrong-answer (`c<1`); + unit test |
| 7 | loop_memset 64-bit trip counts (for + while) | **AGREE** — old `% (1<<64)` was panic (debug) / `%1=0` refuse (release, safe but pessimal); fix unlocks 64-bit-IV memsets; + for/while regression tests |
| 8 | loop_unroll retune (4×@≤40, 2×@≤80) | **REVERTED (measured dead)** — corpus sweep with temporary size instrumentation: sha256's loop never reaches the table (bails at steps 1–8; the "~40-insn SHA-256 round" justification is fiction — 4×/2× assemblies byte-identical); all 12 corpus loops that query the table have bodies ≤19 where both tables agree. Zero effect + false comment → full revert (table + test) |
| 9 | vectorize window 5..=64 + mod.rs comment | **REVERTED (measured dead + backwards)** — trip sweep: zero const trips in 17..=64 corpus-wide (observed: 11, 16×3, 10⁷, non-const); the complete unroller's bound is `2..=16` in code, so the PR's "limit <= 64" comment is wrong (old "limit <= 16" was right) and trips 17–64 were never "eaten". Full revert (window + comment) |
| 10 | global_alloc hook (`pipeline.rs`) + `mod` line | **DISAGREE (dropped)** — references `global_alloc.rs` which is NOT in the PR: certain E0583 build failure (CI cause #0). No implementation exists → dropping loses nothing. P0-A stays a follow-up; hook warts (duplicated 20-line block, force-beats-killswitch, MAX=0 runs one func) die with it |
| 11 | PR description reliability | **UNRELIABLE** — hallucinates `encode_bmi2_rorx()` and `--total-push-budget` (neither in diff); omits dead `roots_distinct`, `n_self`/`n_param` test flips; "zero-fuzz" with zero builds. Audit-the-diff only |

## CI failure mechanisms (reproduced by inspection; full-build repro skipped as certain)

1. `src/backend/mod.rs`: `mod global_alloc;` with no file → E0583 (build gate).
2. Dead `env_flag_truthy`, `is_unique_root`, `roots_distinct` → `-D warnings` clippy gate.
3. (Latent, not CI-run) `check_loop_idiom.sh`: `n_self`/`n_param`/inert expectations
   invalidated → updated in perfection (flips + memmove assertions + default-on cover).

## Why the fold is safe to land default-on

- RORX: fires only when `opts.bmi2` (explicit `-mbmi2`, v3 profile, native-detect);
  baseline codegen byte-identical (verified: corpus A/B + `asmdiff`).
- loop_idiom: default-on proven by `cargo test` + regression `.sh` + benchmark-output
  parity + C torture + fuzz + `CCC_VALIDATE_SSA` (see validation log in patch).
- Unroll/vectorize: landed only at measured-good settings (see §8–9 data).
