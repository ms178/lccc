# Session 44 — re-base onto PR #179 + soundness/robustness fix sweep (2026-08-22)

Base: `ms178/lccc` main `58dfe66` (merge of PR #179). Build: `target/fastbuild`
(`opt-level=1`, no LTO, 2 jobs, 4 GB swap). Host: 2-core VM, no PMU.
Working tree re-based from the pre-PR#179 session state; all session-41 work
preserved and re-validated; four new defects found and fixed.

## 1. What upstream merged while this session ran (PR #179)

- `redundant_ext.rs`: first-comma→rfind operand splitting (SIB-safe zero-extend
  tracking; also un-stales a byte fact after opaque SIB `movq` loads); 32-bit
  ALU writes tracked as upper-32-zero (`cmpl`/`testl` deliberately excluded).
- `alu.rs`: 3-operand immediate `imul` for constant multiplies (both plain and
  fused mul-add paths; 64-bit form restricted to signed i32, 32-bit form takes
  any 32-bit pattern — `const_as_imm32_typed`); BT bit-index consumed from its
  own register instead of `%rcx` staging (guards: shared-dest, XMM homes).
- `vector_temp_promotion.rs` v2 (adopted after red-team): escape-gated
  promotion, object-root load fusion, `write_may_clobber` slot+source
  invalidation, `VecStoreI64x2` memory barrier, width-typed
  `pointer_vector_arg_width`, movnt align-16 relaxation, six Pmovzx/Pmovsx-256
  extensions. `tests/linker/setup_oracles.sh` pins `MOLD_LTO=OFF` and writes
  `ORACLE_REVISIONS.txt`. `aarch64_fuzz.py` pins `-ffp-contract=off`.
- Audited during the re-base: `roots_proven_distinct`'s `Alloca` arm is sound
  (fresh-per-invocation allocas can't be hit by direct param/global writes;
  load-derived pointers stay root-less and fail closed), the fused-mul-add
  immediate path never clobbers the product in `%rax`, and root propagation
  goes through GEP/Copy/Cast/Add/Sub/Select/Phi only — never through memory.

## 2. Session-41 work re-validated on the new main

| Item | File | Result on 58dfe66 |
|------|------|-------------------|
| Hot-loop Phase-1 homes key on **use-site** loop depth | `regalloc.rs` `hot_loop_home` | ra01 proxy 124→117 insns, 34→20 stack refs; Phase-1 now assigns all six loop-carried values |
| ABI-hinted ParamRefs never evicted | `live_range.rs` `select_evict_victim` | sqlite VDBE corruption class closed |
| rbp(6) accepted as callee-saved prestore home | `prologue.rs` | `op` param pre-stored correctly |
| `# LCCC_PARAM_ABI_READ` marker + pin | `peephole/passes/mod.rs` | pinned ABI reads survive copy-propagation |
| gzip 1.14 30/30 + doc roundtrip | — | **PASS** (fresh obj-dir build, lccc CC) |

## 3. Defects found and fixed this session

### 3.1 Missing-in-repo regression runner + oracle opt-outs

Prior sessions quoted "382/382 lccc regressions" but no runner existed in the
tree. Added `tests/regression/run_regression.py`: sweeps `tests/regression/*.c`
plus the benchmark programs, honours `.flags` / `.env` / `@PROFDIR@` PGO
roundtrips / `LCCC_NO_COMPARE`, parallel, JSON report, and reports ENOENT
exec failures (e.g. a missing i386 loader after a host reset) instead of
crashing. Three tests turned out to be **runner-acceptance bugs, not compiler
bugs** — each test's own self-checks pass under lccc while the GCC byte-compare
is invalid for a different reason; documented via `.env` opt-outs:

- `has_attribute_in_code.env` — GCC 14 evaluates `__has_attribute` only in
  `#if`; in code position it folds to 0 and fails the test's own checks.
- `builtin_cpu_supports_raptor.env` — lccc folds against the fixed Raptor
  Lake allowlist (by design); GCC does runtime CPUID and correctly reports
  AVX-512 on this Xeon VM. Host-dependent vs host-independent oracle.
- `fp_domain_crossing.env` — raw accumulator prints differ from GCC by a few
  ulps (legal reassociation); every self-check line passes under both.

### 3.2 vector_temp_promotion: constant-address load sources

`invalidate_for_value_write` treated a `Const` load source as untouchable
("the backend materializes it directly"). A write through a plain parameter,
a global symbol, or an opaque pointer may legally target that absolute address
(the caller can pass it; a linker script can place a symbol there), so the
forward could survive an intervening store and the promoted consumer would
re-read post-store memory. Fixed: a constant-address source now survives only
writes provably confined to an alloca-derived pointer (`write_root =
Some(PointerRoot::Alloca(_))`). Two regression tests added.

### 3.3 gen_lcccsimd.py: immediate builtins declared one parameter too many

Every imm-taking builtin in `include/lcccsimd.h` was declared as
`fn(long, <real params…>, int __imm)` while the user-facing macro passes
`builtin(__imm, <real args…>)` — the immediate arrives through the leading
`long` dummy slot, so the trailing `int __imm` made the prototype one longer
than the call. Invisible until P0-05's fixed-prototype arity diagnostics,
which then rejected every `_mm_shuffle_ps`/`_mm_round_ps`/`vpternlogd512`-style
call: the whole `tests/intrinsics` suite (t128_fp / t256_fp / t512_int)
failed to compile ("too few arguments … expected 4, have 3"). Fixed in the
**generator** (52 proto sites, incl. the ternary+imm group) and the header
regenerated — `intrinsics 3/3 PASS` now.

## 4. Validation battery (all on the final tree)

| Gate | Result |
|------|--------|
| `cargo test --lib` (fastbuild) | **1037 pass**, 6 ignored |
| Regression corpus (run_regression.py, 379 = 346 + 33 benchmarks) | **372 pass / 0 fail / 7 documented skips** |
| `check_sema_constraints.sh`, `check_bit_test_canonical.sh` (fastbuild CCC) | pass / pass |
| gzip 1.14 (fresh extract, SHA-256 `01a7b881…` pinned) | **30/30** + doc roundtrip |
| `phi_cfg_fuzz.py` 0:600 × 3 levels | **1800/1800** |
| `differential_fuzz.py` 0:500 × 3 levels | **1500/1500** |
| `alias_fuzz_m32.py` 0:540 × O2/Os | **1080/1080** |
| `tests/intrinsics` (t128_fp, t256_fp, t512_int) | **3/3** (was 0/3) |

No PMU evidence: VM screening only, per project policy.

## 5. Follow-up work (handover for the next session)

1. **RA-01 remat + RA-02 IV homes on real gzip** — the ra01 proxy is fixed;
   re-measure gzip 1.14 `longest_match` stack refs (the RA veto metric) and
   compare against the P0-03 pin (303 insn / 119 stack vs GCC 114/0).
2. **RA-05/RA-06** — wire hole-aware `segments` into the linear scan and
   implement reload-at-next-use (largest remaining spill-traffic lever).
3. **RA-03** — Adler DO8 `sum2`/`n` next-use priority (VM screening 1.51× vs
   GCC after this session's work; gcc/clang/icx keep 0 stack refs).
4. **IS-02 / AB-01** — XMM class for `double` fields; kill the
   `movq %xmm,%rax` shuttle (struct_copy 1.54× screening).
5. **OP-13** — dead-store elimination gated on `segments` (DCE currently pins
   every Store).
6. **OP-32** — wire `alias::forms_disjoint` into GVN load CSE.
7. **OP-05 / IS-04** — non-reduction vectorization + ICX-style `vfmadd231pd`
   YMM accumulators (nbody/spectral 3–9× screening gaps).
8. Keep `CCC_NO_DEAD_EVICT`-style experiment logs honest: the zero-future-use
   eviction search was measured (Adler +15%) and reverted this session; do not
   resurrect without the Adler/gzip oracles.
9. Host-reset hygiene: recreate swap, reinstall rust to `/opt`, reinstall
   `gcc-multilib libc6-dev-i386` (m32 tests exec-fail with ENOENT otherwise),
   re-fetch the pinned gzip tarball (`.cache` is snapshot-excluded), and
   `chmod +x` stripped scripts — `scripts/arena_session_restore.sh` covers
   most of this in one call.
