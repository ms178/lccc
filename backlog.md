# LCCC Engineering Backlog

Last rebuilt: **2026-09-09**, against `main` @ `cdb01750` (upstream current),
from the instruction-selection / codegen oracle census (`scripts/codegen_oracle.py
--rank`, `-O2 -march=x86-64-v3`) across the workload kernel corpus plus the
session's A/B measurements. Rebased and re-measured after the 2026-09-09 session
(I: constant-size `__builtin_memcpy` → native `Memcpy`; II: `fold_ptr_deref_through_stack`
aliasing-store soundness fix; III: `aggregate_sroa` load-forwarding aliasing-store
soundness fix). See
`engineering/FOLLOWUP-2026-09-09-memcpy-lowering-and-fp-soundness.md`.

**Session-2026-09-09 worst-10 measured on the screening VM** (paired median vs
GCC, `-O2`, 9 rounds): aggregate **geomean 1.1707**. `sha256_transform` 1.733,
`expat_xml_scan` 1.301, `zstd_count` 1.222, `sqlite_varint` 1.191, `mandelbrot`
1.151, `binary_trees` 1.105, `sieve` 1.084, `glibc_strstr` 1.062,
`loop_patterns` 1.043, `struct_copy` 0.966 (faster). Results at
`results/worst10_session/results.json`. The prior given ratios were
sha256 1.803 / struct_copy 1.448 / mandelbrot 1.385 / expat 1.373 /
zstd 1.276 / loop_patterns 1.240 / sieve 1.215 / binary_trees 1.193 /
glibc_strstr 1.167 / sqlite_varint 1.166 — **8 of 10 improved**, with the two
remaining large gaps being deep RA/branch-bound codegen issues (below).

Ranked by **measured impact**. Every entry states its evidence, the specific
blocker, and what "done" means. An item with no reproducer does not belong here.
Several entries are deliberately *not* implemented because the safe version is
not yet reachable — those say so rather than being quietly dropped.

**Attribution rule.** A ratio is only meaningful when both arms were measured in
the same window and against the same baseline. Never compare ratios across
reports to claim a regression; use a paired same-window A/B with kill switches.

## Session 2026-09-11 — rebase to `417951a4`, S05/S06/S07 landed, worst-10 re-triage

Base `ms178/lccc` main `@ 417951a4` (PR #490); patch series ledger `S04`–`S07`
in `/home/user/ms178-1.patch` (57,656 bytes, APPLIES-CLEAN, bundle test-cloned).
Same-window full-corpus A/B (screening VM, `-O2`, 9 reps, 39/39 correct):
geomean (LCCC/fastest-ref) 0.7777 → 0.7857 with S05 (neutral within noise).
Worst-12 this window: lz4 1.941, mandelbrot 1.673, find_bit 1.404, expat 1.275,
sqlite_varint 1.244, sha256 1.210, spectral_norm 1.193, zstd 1.137, sieve 1.123,
switch_dispatch 1.102, binary_trees 1.094, nbody 1.089. Evidence and root
causes: `FOLLOWUP-2026-09-11-rebase-perf-s04-s07.md`. Landed this session:

- S05 inline-single-site-static: correctness fix (GCC called-once parity),
  zstd gate 131 → 135 re-baselined with runtime improved. Done.
- S06 cmp-branch-fusion-generalize: 4 latent-miscompile holes closed, 16 unit
  tests, corpus byte-identical (zero fire — all 16 real setCC sites proven
  correctly-unfused). Done; no follow-up except future-shape coverage.
- S07 ifcombine-profitability-guard: pre-filter 4 (reject unless the loop can
  go branch-free), lz4 +4.7%, 38/39 files byte-identical, classifiers still
  vectorize. Done.

New backlog items (ranked by measured impact):

### PF-LZ4-1 · Loop-idiom recognition: byte-copy → `memcpy`, byte-compare → word-compare — ~10x

lz4's *true* work gap is 8–11x (PASSES scaling 24 → 2400: ratio 2.75 → 7.6 →
10.2; fixed-cost subtraction removes the startup artifact — see INF-HARNESS-1).
Asm-constructed root cause: match-extend loop at 9 insns/byte
(`movzbl/movzbl/cmpl/jne` + pointer bump), literal-copy loop at 7 insns/byte;
GCC emits word-at-a-time compare + `call memcpy@PLT`. LCCC has no loop-idiom
pass. "Done" means:trip-count-form coverage for `for (i = 0; i < n; i++)
dst[i] = src[i]` (copy) and `while (p < end && *p == *q)` (extend) with proven
aliasing/overlap/trip-count preconditions in this IR, plus A/B proof on lz4
(and no movement on the other 38). S07's +4.7% (skip-path short-circuit) is
banked and independent. Risk: high (loop legality); reward: highest in corpus.

### PF-MB-1 · Mandelbrot loop vectorization — 1.67x

Same static insn count as GCC (54 vs 55) but GCC emits 9 packed-double vs our
1: the hot FP loop does not vectorize. Reproducer:
`tests/benchmark/programs/mandelbrot.c`. "Done" = vectorized hot loop + A/B
proof. Needs vectorizer triage (why the loop is refused) first.

### PF-FB-1 · `linux_find_bit` hot-loop diff — 1.40x

Our `bsfq` idiom IS present, yet 176 vs 142 insns and 1.40x slower — the gap
is loop structure/branching, not idiom selection. "Done" = hot-loop
LCCC-vs-GCC diff with a classified gap + fix or a proof of RA-bound status.

### INF-HARNESS-1 · Subtract startup / scale short benchmarks

lz4's A/B ratio (1.94x) understated the true 10x work gap: ~2 ms fixed cost
(startup + `fill_source`) dominated the 7 ms measurement. "Done" = harness
reports fixed-vs-work decomposition (or scales workloads / subtracts a
calibrated startup) so no benchmark with fixed cost > 10% of measured time is
reported bare. Without this, short-kernel tuning is blind.

**Environment.** This is a screening VM (2-core Xeon @2.6 GHz, 1.9 GB RAM, no
PMU). The authoritative runtime report is the in-repo CI report for the real
i7-14700KF / Raptor Lake. On this VM the durable evidence is the instruction /
load / store / spill census (godbolt oracle) and the correctness + regression +
benchmark-output gates, which are PMU-independent.

---

## Tier 1 — measured, reproducible, largest first

### RA-PRESSURE-3 · Loop-phased register rotation (sha256 round loop — worst gap)
*Status:* open. The single largest remaining **runtime** gap for the synthetic +
workload corpus.

*Evidence (oracle census, plain `-O2`):* `sha256_transform`'s round loop is
**218 instructions against clang 126** (83 loads / 25 stores / **64 spills**,
15 branches). GCC's is 137 insns / 27 loads / 19 stores / **21 spills**. The
benchmark ratio is **1.73–1.80× vs GCC** — the worst kernel in the corpus.

*Mechanism:* the 64-iteration round loop rotates the 8 SHA state words a…h. LCCC
stages the rotation through the stack: **7 `StoreRbp` + 6 `LoadRbp` per
iteration** (`.LBB9`/`.LBB10`), because the RA cannot express the *cyclic copy
chain* (new_a=t1+t2, new_b=a, … new_h=g; the old h dies) as a register
permutation. GCC keeps the whole rotation in registers and unrolls 4×
(`addq $4,%r8; cmpq $256,%r8`), so the rotation is 6 `movl` at the unrolled
block boundary.

*What "done" means:* the round loop carries a ≤ that many spills; `sha256_transform`
≤1.05× vs GCC. Two sub-problems to attack separately, each with a kill switch:
1. **Phi-copy cycle resolution** — investigate `CCC_DEBUG_COALESCE` on this loop
   and the copy-web / coalesce handling of a *recurrence* (`span_recurrence` is
   already computed; the question is whether the rotation web can be admitted and
   expressed as register swapping instead of memory).
2. **Loop unrolling** (2×/4×) so the rotation happens once per unrolled body and
   the counter advances by the unroll factor; verify against register pressure.
   `loop_unroll.rs` may already have the cost model; confirm it fires here.

### RA-PRESSURE-4 · Aggregate/struct register pressure — 1.15–1.45×
*Evidence:* `struct_copy`'s `main` **156 insns vs clang 76** (52 / 32 /**77**
spills). On this VM it measured ~parity, but the Raptor Lake report shows 1.45×.
*Root:* struct temporaries (ParticleGroup/Particle) are aggregate-homed and the
per-field SROA/scalarization is not fully reaching the register allocator.
*Done when:* `struct_copy` ≤1.05× on the Raptor Lake report; `CCC_NO_AGGREGATE_SPLIT`
is already the escape hatch.

### PF-CHACHA-1 · `chacha20_core` ARX schedule — 222 vs icx 63 (at v3)
*Evidence:* oracle census at `-O2 -march=x86-64-v3`: `chacha20_core` 222 insns /
24 loads / 10 stores / 18 spills vs **icx 63**. ICX's ARX schedule is dramatically
tighter. At plain `-O2` the benchmark ratio is ~1.04–1.08× (near parity), so the
v3 vectorized path is *not* the benchmark bottleneck — separate the two.
*Done when:* the v3 ARX path gets closer to ICX (vectorized QR); the benchmark's
plain-`-O2` ratio stays ≤1.05×.

### PF-SCHEDULER-1 · Message-schedule vectorization of sha256
*Evidence:* GCC vectorizes the `m[i]=SIG1(m[i-2])+m[i-7]+SIG0(m[i-15])+m[i-16]`
sliding-window expansion with XMM/SSE even at plain `-O2`. LCCC computes it
scalar (`roll`/`rorl`/`shrl` per word). This is the second large sha256 win.
*Blocker:* it is a strided/sliding recurrence; needs unaligned vector loads at
constrained offsets plus a sliding-window SLP recognition. `vec_arx`/`arx_vectorize`
are the likely homes.
*Done when:* sha256 schedule is vectorized at `-O2` and the whole kernel ≤1.15×.

### PF-TLS-1 · TLS segment access
*Status:* partially closed upstream. *Evidence:* `tls_seg_access` 2.19× → ~1.06×
(after IV-widening + block-layout fixes). The remaining gap: two of three
`&tls_slots` computations are duplicates not CSE'd; dynamic-offset TLS reads may
still stage the thread pointer in the prologue.
*Done when:* ≤1.05× and every dynamic-offset TLS read is a single `%fs:` operand.

### PF-CLS-1 · Byte-classifier chains (expat_xml_scan) — 1.31–1.37×
*Evidence:* `expat_xml_scan` 1.31× (VM) / 1.37× (Raptor Lake). The kernel
`expat_utf8_name_length` is at **79 vs gcc 78 insns** (gap 1), yet it is the
benchmark hotspot — so the gap is *branch prediction / scheduling*, not
instruction count. The pipeline `if_convert → range_fold → set_membership` is
starved at the first link for the *counting* spelling of `pred && n++`.
*Blocker:* structural — each test's hit edge funnels through one shared increment
block, so the join phi has two incomings; the last member of `a||b||c` branches
to the hit block *and* falls through to the join (critical edge). Two attempts
reverted for a miscompile.
*Done when:* the single-entry/single-exit region is if-converted to predicate
arithmetic; the counting spelling gets the same `Select`s as the boolean
spelling. **Split the critical edge on the last member first.**

### OP-VEC-1 / PF-ADLER-1 · Non-reduction FP vectorization & adler accumulator recurrence
- `spectral_norm`, `nbody`: need multi-store scatter + computed-invariant dot
  analysis.
- `zlib_ng_adler32`: DO8 recurrence `s1+=*buf++; s2+=s1;` — lccc is last on this
  kernel (oracle); would be a clean SLP/reassociation target. Block layout no
  longer distorts it.

---

## Tier 2 — MachInst / instruction selection

Coverage is ~85%+ corpus-wide (a regression test fails if any class the layer
owns drops back out — the fallback to text emission is silent).

### MI-CLOBBER-1 · Clobber modelling, then `Call` — 7.8% of instructions
**Currently correct, not defective.** `MachInst::Call` emits a bare `call target`
with no clobber model; what keeps caller-saved values sound is precisely the
rejection (returning `false` flushes/emits before the call). The flush boundary is
the clobber model. Done when MachInst carries a per-instruction clobber set and
`Call` lowers. Adding `Call` uses before that is a miscompile.

### MI-XMM-1 · Vector/FP register class — 1.3%
119 rejected `Store(float)`. MachInst models only the 16 GP families.

### MI-PARAM-1 · Remaining `ParamRef` cases — 1.1%
The provably-no-code subset lowers (963 → 96 rejections). What remains is
emissive: alloca-homed params and stack-passed args. Any replacement must keep
the pinned rule that the fallback reads the parameter's **incoming** register
even when a caller-saved pre-store of a *different* parameter aliased that name.

### MI-ROTATE-1 · Rotate idiom follow-ups
Native `IrBinOp::RotateLeft/Right` (variable amounts, all four backends,
MachInst path) landed upstream; this session's audit confirmed the
cast-peeling soundness fixes hold at `-O2`. Still open:
1. Sub-word rotates whose complements meet only at the narrow width
   (`(x16<<8)|(x16>>8)` arrives promoted to i32; needs the truncation-aware
   pattern / a consuming `Cast(i32→u16)` rewrite to a narrow rotate).
2. Rotate-count staging polish (`movslq %esi,%rdx; mov %edx,%ecx`) — harmless but
   sloppy.

### MI-ENCODE-1 · Encoding-level differential
The MachInst suite has seven layers / 36 tests incl. execution vs GAS 2.47.
Comparing emitted *bytes* against GAS's own encoding would cover what execution
does not. `scripts/insndiff.py`/`encdiff.py` already exist.

---

## Tier 3 — verifier and infrastructure

### VER-DOM-1 · Def-dominates-use in the IR verifier
Six structural properties, thousands of configs, zero violations — but none is
dominance. That is how an SSA violation once shipped past every gate.
Cooper-Harvey-Kennedy dominators over RPO is the next step.

### INF-BENCHGATE-1 · Run the benchmark output gate in CI
`scripts/check_benchmark_outputs.sh` compiles/runs + oracle-diffs every
`tests/benchmark/programs/*.c` at -O0/-O1/-O2/-O3 in ~4 min with no timing. It
should be unconditional in CI (a miscompile once passed all 563 regression tests
because those kernels were only ever built by the timing harness).

### INF-FRESHCLONE-1 · CI must build from a fresh clone
Two files were historically missing from a commit while the author's tree was
fine. A CI job that clones fresh and builds would catch both.

### INF-GAS-1 · GAS 2.47 must be re-provisioned per session
`.cache`/`target`/`/swapfile` are excluded from snapshots, so
`scripts/ensure_gas_247.sh` must be re-run after every environment wipe.

### INF-LINK-1 · Linker oracle
Honour the pinned toolchain: lld 23.1, mold 2.42 (X86+i686-only preset),
bfd 2.47. Keep `scripts/` linker oracle wiring updated with the user's build and
version preferences.

---

## Closed this cycle

| Item | Outcome |
|---|---|
| **Constant-size `__builtin_memcpy` → native `Memcpy`** | `read64`-style unaligned scalar loads lowered to a libc `call memcpy`, leaving a **store-to-temp + reload** in every hot loop (zstd_count, glibc_memcmp, chacha20 data loads). Now lowered to `Instruction::Memcpy`; SROA forwards the load to the source and removes the dead copy. zstd_count 1.34× → **1.22×** (paired CI [1.206, 1.233]); glibc_memcmp_common_alignment 88 insns (gcc 81, clang 130); benchmark-output gate still 180/180. Only the `__builtin_*` spelling is rewritten (a user `memcpy` keeps exact semantics); `memmove` is excluded. |
| **`fold_ptr_deref_through_stack` aliasing-store soundness** | Latent miscompiler exposed by the memcpy change. The peephole folded `movq (%ptr),%rax; movq %rax,slot; ...; movsd slot,%xmm` into `movsd (%ptr),%xmm` without bailing on an intervening store to memory through a data register that could write `(*ptr)` (e.g. `movq $0,(%rsi)` with p==q). `fp_liveness_ptr_deref_alias_negative` returned `3 1`; now bails on any `Other{dest_reg:REG_NONE}` in the scan window and returns `3 3`. |
| **`aggregate_sroa` load-forwarding aliasing-store soundness** | Second latent miscompiler exposed by the memcpy change. The SROA load-forwarding pass reads the copy source at the *use* site but only treated an intervening store as clobbering when its pointer had the *same SSA value* as the source (`pr == sr`). Two distinct pointer values (e.g. two params passed the same address — `u64 bits; memcpy(&bits,p,8); *q=0; memcpy(&d,&bits,8)` with p==q) alias at runtime, so the forwarded `load *p` read the post-store value: returned `1` instead of `3` at `-O2/-O3`. Now a store (or `Memcpy` dest) is proven safe only when the source is a private non-escaping alloca or the target is one; otherwise assume aliasing. New regression `memcpy_unaligned_load_fwd.c`. |
| **chacha20 / ARX pipelines** | 10.01× → ~1.05–1.08× vs GCC via the aggregate-SROA fix + native rotate (upstream #440) + this session's validation. The remaining gap is RA rotation (RA-PRESSURE-3), not instruction count. |
| **IV widening** | Fires transparently to constant scales (`Shl`/`Mul`); variable scales deliberately excluded. `sieve` −21.6%, `nbody` −3.0% (same-window A/B). |
| **Block layout** | RPO linearization now starts from the existing order and fixes only contiguity; no 19% adler32 cost. |
| **Structural IR violations** | 0 configs at all six opt levels via `CCC_VERIFY_IR`, a permanent suite gate. |
| **Machine-level loop inversion** | `memchr` −49.7% → parity with GCC. |

---

## Method notes that keep paying off

- **The benchmark suite is the oracle.** Every candidate change is evaluated by
  a paired same-window A/B with kill switches, not by cross-report ratio
  comparison.
- **Instruction / spill census beats wall time on this VM.** The godbolt oracle
  (`scripts/codegen_oracle.py --rank`) gives deterministic insns/loads/stores/
  spills/branches per function, which is PMU-independent and far less noisy than
  sub-20 ms wall-clock on a 2-core sandbox.
- **The stable gates are the contract.** `ci_local.sh` runs eight gates; a change
  must keep `run_correctness.py`, `run_regression.py`, and
  `check_benchmark_outputs.sh` green before it is considered.
- **A new kernel-level load forwarding exposed a real latent bug** — the pattern
  for finding these is to increase optimizer reach, then run the existing
  NEGATIVE probes, which are exactly the ones that fail loudly.

## S09: cond-store rewrite use-without-def (PRE-EXISTING, FIXED 2026-09-11)
PR #492 CI red on `vec_cond_store{,_sse2}` (`emit.rs:2443` unwrap:
`Sle(Value(27), 122)` with no definition). Bisection blamed the S07
if-combine guard as the *trigger*, but `CCC_NO_IFCOMBINE=1` panics on
the pristine base binary too: the defect is a latent vectorizer bug the
guard newly exposes by declining. Per-pass IR trace proved `vectorize`
deletes the definition while keeping the use: `rewrite_conditional_store`
empties every non-first chain block, but `emit_cond_mask_selects`
re-emits non-first compares with their original operands verbatim — the
`Cast` defining `Value(27)` died in the second condition block.
Same-class hazards in the same functions: reused compares from cleared
folded-tree blocks, store computations referencing cleared defs or the
dropped address GEP, escaping uses of cleared values, and unconditional
exclusion of a store-block GEP that is not the store address.
Fix (`src/passes/vectorize.rs`, both chain and folded forms): operand
availability gate — values defined outside the cleared blocks pass
through, values defined inside get their pure def chain re-materialized
ahead of the selects (casts/copies/unary/non-trapping arithmetic/address
math/compares; trapping int div/rem never speculated; an identical
already-dominating computation is reused, so the cast-in-condition shape
emits zero new instructions), and anything else — or any surviving use
of a dead definition — declines the rewrite (scalar stays byte-exact).
The `k_inplace_three` uncombined shape now vectorizes fully (32-wide
AVX2 byte lanes) instead of crashing; the combined (folded-mask) path
is behavior-preserving. Validated: repro + `CCC_NO_IFCOMBINE=1` clean,
both CI-failing tests pass, full CI suite (cargo test, 563-test
regression corpus with `CCC_VALIDATE_SSA=1`, benchmark outputs,
differential correctness, loop alignment) green.

## S10: vectorizer IV-live-out miscompile (PRE-EXISTING, FIXED 2026-09-11)
`wideret` (byte copy with IV live-out) miscompiled at default `-O2`: `wsum`
got the vector iteration count (15) instead of the trip count (511).
Root cause: the map and stencil transforms redefine the scalar counter once
per vector iteration but never rewired escaping IV uses to the remainder IV
— the reduction paths already called `rewire_escaping_iv_uses`, the map and
stencil paths simply never did. A stencil probe miscompiled identically
(`lstencil=64` for a `1..511` loop). Fix (`src/passes/vectorize.rs`):
generalize `rewire_escaping_iv_uses` (`&ReductionPattern` → `iv: Value` +
pass tag; also cover `Store.ptr`) and call it from `transform_map_vector`
and `transform_stencil_vector` (outside-label snapshot before the remainder
commit, rewire after); retarget exit-block phi loop-side incoming labels at
the remainder header; and gate both analyzers with `loop_escape_closed`
(single exit via the header, no loop-defined value consumed outside except
the IV — fail closed). Zero-cost repair: no new arithmetic, loops stay
vectorized (`wsum`: 256-bit U8 + 1 rewired use; stencil: 22 changes +
1 rewired use). Validated: `wsum=511`, `lstencil=511`, reduction still
exact, full CI suite green (cargo test, 748-test regression corpus with
`CCC_VALIDATE_SSA=1`, benchmark outputs, differential correctness, loop
alignment) + strict clippy clean.

## S08-next: loop-idiom v1.1 roadmap (pass landed opt-in, M1+M2+M3 green)
v1 matches ZERO lz4 loops (measured): hot loops are single-block (`body size
1`: wildCopy8/32, memcpy_using_offset, decompress inners), pointer-IV (`IV
not int` x4), param-rooted (`roots not unique`). In order of impact:
1. **Single-block loops** (header==latch, load/store in header): biggest
   real-world coverage win; rewrite is position-agnostic, matcher block
   scan needs the header-load/store case + latch==header phi edges.
2. **Pointer-IV loops** (`for (p=s; p<e; p++) *d++=*p++`): match `Ult` on
   pointer phis; trip count = `end - start`; exit values analogous.
3. **Preheader splitting**: `main loop@17: bail (preheader not Branch
   header)` — clone the preheader edge when shared (standard; mind
   critical-edge + phi updates).
4. **Restrict-param roots** (needs frontend+IR work): `__restrict__` params
   as unique roots unlocks libc-style `dst/src` copies; lz4 literal path
   needs (1)+(2)+this. Match-copy smear must NEVER match (overlap).
5. Compare idiom (`while (p<e && *p==*q)`) as designed in module docs.
Each extension reuses the validated M2 rewrite; flip `CCC_LOOP_IDIOM`
default-on only after corpus A/B + fuzz on the final guard set.
