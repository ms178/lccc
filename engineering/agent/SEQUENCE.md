# Execution sequence

Derived from the open items in [`BACKLOG.md`](BACKLOG.md) and the queue in
[`../tasks/`](../tasks/README.md). Re-oracle and check gzip `longest_match`
stack-mem after each item. Vetoes: sqlite/expat miscompiles.

## Phase 0 — Hygiene & enablers (parallel-safe, low risk)

| Step | ID | Work | Gate |
|------|-----|------|------|
| 1 | MS-08 | Consolidate the 50+ `CCC_` env knobs (regalloc 33, live_range 6, x86 prologue 11) into a `RaConfig` struct (single source of truth) | byte-identical asm on the kernel corpus |
| 2 | MS-10 | Run `CCC_VALIDATE_SSA` over the regression corpus in the codegen gate | watermark + duplicate-def clean |
| 3 | MS-04 | Make OnceLock env caches parameterizable so unit tests are independent | `cargo test --lib` |
| 4 | MS-09 | Close the peephole UTF-8 audit: verify no shipped binary carries corrupted asm; write the audit report | audit doc in `evidence/` |

## Phase 1 — P0 codegen (the big three)

| Step | ID | Work | Gate |
|------|-----|------|------|
| 5 | RA-06 / RA-06a / PF-05 | Reload-at-use + arithmetic-chain copy webs in the scan. Reuse the sweep-eviction traffic model; start with call-site splits; extend copy webs through the arithmetic chain so ONE range carries a recurrence | adler ≤1.15× GCC kernel; gzip gate; `CCC_VERIFY_REGALLOC` clean |
| 6 | OP-05b | Multi-store scatter vectorization (nbody `fx/fy/fz[i] ±= …`) + computed-invariant dot (spectral `A(i,j)` affine in j) | nbody/spectral A/B at -O3 v3; checksums bit-identical |
| 7 | PF-17 | Root-cause the 15 loop-rotation miscompile shapes (list in DECISIONS.md), harden, then default-enable at -O2+ | regression corpus green; geomean improvement on the 9-kernel suite |
| 8 | RA-01b | Marching-pointer recurrences: extend GlobalAddr remat through Copy chains; prefer SIB-index forms for IVSR pointer recurrences | nbody stack refs 159 → <20 |

## Phase 2 — glibc & kernel gates

| Step | ID | Work | Gate |
|------|-----|------|------|
| 9 | LK-19 | IFUNC end-to-end: `__attribute__((ifunc))` in sema/lowering, `R_X86_64_IRELATIVE` for data words, ld.so semantics | glibc configure accepts `--enable-multi-arch`; IRELATIVE relocs in a probe |
| 10 | LK-24 | External-PIE startup SIGSEGV triage (LD_DEBUG, `_r_debug`/`_rtld_global`/TLS init) | staged-loader smoke exits 0 |
| 11 | MS-11 | glibc `make check` triage harness: classify {miscompile, unsupported-feature, environment} | harness committed; first triage table |
| 12 | KERNEL B1/B2 | objtool `.discard.annotate_insn` empty-data assembler bug; statement-expression + inline-asm typing (blocks `net/*`) | objtool passes on lccc objects; net/*.o compile |

## Phase 3 — P1 measured gaps

| Step | ID | Work | Gate |
|------|-----|------|------|
| 13 | PF-06 | isort secondary-IV strength reduction (marching register) | isort 43 → ≤25 insns |
| 14 | IS-11 / IS-12 | C `__ffs` if-tree → `andn`+cmov chain (not tzcnt) | find_bit vs gcc CE |
| 15 | FE-25 | Real `-march=native` for Raptor Lake (AVX2/BMI2/F16C/GFNI/VNNI where present); cost model queries the host | feature detection on the 14700KF |
| 16 | PERF-3 | TLS base CSE + `%fs:sym@tpoff(,%r,scale)` addressing | tls_seg_access ≤1.3× GCC |
| 17 | OP-25 / OP-26 | Inline `name_cont`; pressure-aware inline budget | expat scan; sqlite frame |
| 18 | IS-07 / IS-08 / IS-13 | cmov limit idiom; remaining load-cast folds; ALU+mem for spilled | oracle counts |

## Phase 4 — P2 / research (pull from BACKLOG)

LK-20/21/22/23 linker polish · OP-03/07/08/10/11/14/16/17/19/20/21/22/23/24/26/27/29
· FE-02/03/04/05/06/07/08/09/10/11/12/13/14/15/17/18 · AB-01/02/03/04/05/11/12/15 ·
PG-01/02/04/05/07/08/09/10 · LK-03/07/08/09/10/11/12/13/15/16/18 ·
MS-03/05/06/12/13/14 · RA-07/08/09/10/11/15/16/17/20/22 · EVEX/AVX-512 assembler
(mathvec blocker) · fix_dash (needs qemu-user + riscv64 sysroot).
