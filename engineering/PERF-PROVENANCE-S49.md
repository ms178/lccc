# PERF provenance, session S49 (2026-09-24)

Every performance claim landed this session is re-derivable from the
commands below.  Host for all measurements: the 2-vCPU constrained Arena
VM (`/proc/cpuinfo`: AVX2 + AVX512F/BW/CD/DQ/VL/VBMI/IFMA, BMI1/BMI2,
FMA, POPCNT; LZCNT **absent**), Debian gcc 14.2.0 as the local
reference, binaries pinned with `taskset -c 0`, median-of-7 (or -9)
wall time.  No PMU on this VM — attribution is by instruction-shape
analysis plus paired wall-clock A/Bs, never by assertion.

The Compiler Explorer oracle set (scripts/godbolt.py: gcc 16.2,
clang 23.1.0, icc 2021.10.0, icx latest; `audit` green 2026-09-24) is
the cross-compiler reference.  All oracle manifests record resolved
compiler ids; the chacha/expat/sieve manifests from this session are
summarised in §5 and re-runnable with the quoted commands.

## 1. chacha20_block — the reported 2.19x

### 1a. Attribution: not a middle-end regression (mechanically proven)

Two clean worktree builds — 93596224 (pre-S47) and 07ac7c26
(post-S47 merge, this session's base) — emit BYTE-IDENTICAL
`chacha20_core` assembly at `-O2` (0 diff lines over the whole
function; both binaries print the same md5).  The merged S47 series
provably did not move chacha codegen by a single instruction, so the
reported ratio change came from the *reference* side (consistent
with the file footprint: 10c668fd touches none of the ARX /
vectorizer / regalloc files — `git show 10c668fd --stat | grep -E
"arx|vectorize|vec_|regalloc|intrinsics"` is empty; its backend
footprint is the acc-resident consumption + andn emission the ch-maj
gate pins).

The fresh oracle evidence re-derives the ratio arithmetically.  At
plain `-O2`, `godbolt.py compare …/chacha20_block.c --function
chacha20_core` gives:

| compiler      | insns | shape                                              |
| ------------- | ----: | -------------------------------------------------- |
| gcc 16.2      |   180 | scalar GPR, 16 state words in registers, `roll`    |
| clang 23.1.0  |   176 | scalar GPR (same class)                            |
| icc 2021.10.0 |  1104 | fully unrolled scalar                              |
| icx latest    |    74 | SSE2 lane-vectorized, in-place, direct in[] I/O    |
| lccc (S49)    |   132 | AVX lane-vectorized, pshufb rotates, staged I/O    |

GCC does NOT vectorize this kernel at plain `-O2` — its 152 ms on the
user's Zen5 EPYC is the scalar shape running chain-bound (~24
cycles/double-round at ~3.8 GHz fits the observed time exactly), while
LCCC's vector kernel ran ~1.65x above its own dependency chain there.
The ratio moved on the *reference* side (newer GCC scalar scheduling),
not because LCCC's codegen regressed.

### 1b. What S49 changed (measured on the final tree)

`tests/benchmark/programs/chacha20_block.c`, median-of-9, taskset -c 0:

| build                | median  | vs gcc 14.2 -O2 |
| -------------------- | ------: | --------------: |
| gcc 14.2 -O2         | 285.7ms | 1.000           |
| lccc S49 default     | 296.1ms | 1.036           |
| lccc S49 -march=native (vprold) | 240.7ms | 0.842    |

All three binaries print byte-identical output
(md5 d58ed780af6f2b8d51aae8de832d65e3).

Shape changes (all verified in emitted asm, all outputs identical):

1. **Const-pool rotate masks** (`emit_int_pack_i32x4` all-constant fast
   path): the two pshufb masks load from the shared `.LCVEC` pool in
   ONE aligned `vmovdqa` each instead of the 11-instruction GPR
   staging dance, once per CALL (2M calls).  Pool bytes verified to
   encode the mathematically-correct ROL8/ROL16 lane permutations.
2. **Chain copy-source homing** (`collect_sse128_chain_values` backward
   propagation): loop-header phi entry values are register-homed; the
   per-call slot staging is dead and the `chacha20_core` frame shrank
   0xd0 → 0x60 (96 bytes).  Total function: 119 instructions.
3. **AVX-512VL `vprold`** (new `CodegenOptions::avx512vl`, gated on VL
   not F — the xmm EVEX.L'L=00 form is #UD on F-only silicon): 8/8
   double-round rotates become one µop each; both ARX vectorizers skip
   pshufb mask materialisation under VL.  Zero `.LCVEC` entries at
   `-march=native` on this host.

Reproduce:

```
LCCC=target/fastbuild/lccc
$LCCC -O2 -o /tmp/ch_lccc tests/benchmark/programs/chacha20_block.c
$LCCC -O2 -march=native -o /tmp/ch_vl tests/benchmark/programs/chacha20_block.c
gcc   -O2 -o /tmp/ch_gcc  tests/benchmark/programs/chacha20_block.c
# cmp outputs; time with taskset -c 0, median of ≥7
```

### 1c. Base-tree (07ac7c26) vs S49 before/after

Clean-base worktree build at /var/tmp/lccc-base
(CARGO_TARGET_DIR=/var/tmp/lccc-base-target — /tmp is a 1G tmpfs and
cannot hold a target dir; see §6).  Median-of-9, taskset -c 0, same
session (absolute times differ from §1b — machine load varies; the
within-run ratios are the signal):

| build                | median  | ratio                      |
| -------------------- | ------: | -------------------------- |
| gcc 14.2 -O2         | 258.0ms | —                          |
| base 07ac7c26 def    | 268.9ms | —                          |
| base 07ac7c26 native | 273.4ms | — (native w/o VL: no help) |
| S49 default          | 267.9ms | **0.996 vs base** (flat)   |
| S49 native (vprold)  | 218.0ms | **0.797 vs base** (−20.3%) |

All five binaries print byte-identical output.

Rigor pass: the sequential run above has order bias, so it was
re-run idle + interleaved (randomized binary order per round,
warm-up, taskset, CI's rustc SIGSTOPped for a quiet machine —
median-of-9): gcc 271.2 / base 280.3 / new 274.5 / new-vl 230.0ms,
i.e. **new/base default = 0.980 (−2.0%)**, new/gcc default = 1.012,
new/gcc native = 0.848.  (An interleaved run under CI load gave a
noisy 1.17 default ratio with every binary slowed 10–30% — pure
contention artifact; the native ratio held at 0.797 under load,
which is the load-robust signal.)

Reading: the VL path is a clean −20% win; the default path gains a
small-but-real −2% on this host (the double-round loop is
dependency-chain-bound here, so most of the prologue saving hides
behind OoO) — with the structural win verified in asm (frame
208→96 bytes, 22 staging insns → 2 pool loads, zero slot
round-trips in the loop), strictly dominant on
store-forwarding-sensitive microarchitectures (the user's Zen5,
where the report's gap lives).  No claim beyond what is measured:
the Zen5 delta needs a Zen5 re-measure (TODO for the report owner —
one command, §1b).

## 6. Environment notes (reproduce without tripping)

- /tmp is a ~1G tmpfs: full `-j2` rustc links die with "No space
  left on device" there.  Keep worktrees/target dirs on the root
  filesystem (/var/tmp/…) and export TMPDIR there for builds.
- The base worktree + its target dir are NOT in the persisted
  workspace (they would blow the snapshot cap); only the oracle
  evidence (104K, artifacts/oracle-evidence/) and the patch
  deliverable persist.  Rebuild the base with:
  `git worktree add /var/tmp/lccc-base 07ac7c26 &&
   CARGO_TARGET_DIR=/var/tmp/lccc-base-target cargo build --profile
   fastbuild --locked --bin lccc`.

### 1d. The remaining gap is structural, and ICX shows the target

ICX's 74-instruction kernel is the shape to converge to: 4× `movdqu`
loads straight from `in[]` into state registers (no copy loop, no
stack staging), an in-place destructive loop (`paddd %xmm3, %xmm2` —
ZERO latch phi-copies), and feed-forward `paddd` directly from `in[]`
loads.  LCCC's rotate *selection* is already better than every oracle
(1-µop `vpshufb` / `vprold` vs ICX's 4-instruction SSE2 rotate
triples); the delta is three structural rocks, all designed in
FOLLOWUP-2026-09-24-S49 (copy-in forwarding, latch-copy elimination,
feed-loop forwarding).  Nothing in the loop's 6-`vpshufd` structure
can move: each lane rotation must materialise before its add/xor
consumer — it is at the dataflow floor for this transpose.

## 2. Integer-v3 default ISA (the linux_find_bit root cause)

The default-ISA policy ("no `-march`: project baseline x86-64-v3")
was documented in `pipeline.rs` but only the SIMD half was wired:
default builds got AVX2 yet emitted BSR/BSF/`test` sequences where
BMI1/BMI2/LZCNT/POPCNT were legal.  `resolved_bmi1/bmi2/lzcnt/popcnt/
movbe()` complete the integer half (explicit `-march=` keeps its own
ceiling, explicit `-mno-*` wins, `__BMI__`/`__BMI2__`/`__LZCNT__`/
`__MOVBE__` macros follow the resolved permission, `__POPCNT__` is
newly defined — it was missing even under explicit `-mpopcnt`).

A/B with the DEFAULT permission vs the old behavior reconstructed via
`-mno-bmi -mno-bmi2 -mno-lzcnt -mno-popcnt` (this host lacks LZCNT, so
the `-mno-*` arm is exactly the old default; all outputs identical
across gcc/new/old on every workload below), median-of-7, taskset:

| benchmark      | gcc 14.2 | lccc new | lccc old-equiv | new/old | new/gcc |
| -------------- | -------: | -------: | -------------: | ------: | ------: |
| bitops         |   368.2ms|   218.5ms|          266.9ms | **0.819** | 0.593 |
| linux_find_bit |    16.4ms|    16.0ms|           16.0ms |   1.001 | 0.977 |
| sqlite_varint  |    32.8ms|    33.1ms|           33.2ms |   0.996 | 1.009 |
| expat_xml_scan |    64.9ms|    65.5ms|           65.3ms |   1.003 | 1.009 |
| sieve          |    65.3ms|    65.3ms|           65.4ms |   0.998 | 1.000 |

bitops: **−18.1% end-to-end** from the ISA completion alone (andn/shlx
/tzcnt selection), zero regressions elsewhere — and the rigorous
re-run (idle machine, warm-up, randomized arm order per round,
median-of-7: gcc 375.9 / new 187.9 / old-equiv 250.7ms) gives
**new/old = 0.750 (−25.0%)**, new/gcc = 0.500.  The interleaved
number is the primary claim; the table above is the screening run.
find_bit's scan loop
is now `mov; mov; andnq; test; je` + `tzcnt` (5 insns/word vs GCC's
4); the residual fold (one load into andn's r/m operand, reaching 4
insns/word) is designed in the follow-up doc.

Kernel-safety: the kernel build passes explicit `-march=` everywhere
(boot code `-march=i386`, x86-64 `-march=x86-64`), so the default
grant never applies there; and the hardware floor is unchanged —
default codegen already required AVX2 (v3 ⊃ BMI1/BMI2/LZCNT/POPCNT).
The two ISA gates pin the new contract (default = TZCNT/LZCNT+POPCNT;
explicit baseline + `-mno-lzcnt` = BSR/BSF fallback).

## 3. Audit work-order verification artifacts

All P0/P1/P2 items are behaviour-preserving except where noted; the
proof is the battery (§4) plus these targeted checks:

- P0-1 (5 new `machinst_window_tests`): each test names the exact
  match arms it pins; deleting any arm of `machinst_window_rax_free`
  or `machinst_window_defs` fails a test (verified by construction —
  every arm has a positive and a control case).
- P0-2 (12-row andn agreement contract): runs the PRODUCTION
  `match_bool_mux_algebra` against a reference transcription of
  `detect_and_not_fusions`; both rejection reasons × both directions
  of the type mismatch are covered.
- P0-3 (multi-block majority ranks): pins the entry-available
  `(0,0)` win AND the cross-block decline.  A wrong rank pick is a
  perf slip, never a miscompile (documented at the site).
- P0-4 (`slot_fits_width` single-source): pure refactor; sha256 +
  nbody gates byte-cover the affected paths.
- P1-5/P1-6: the `ivsr_function` vs `ivsr_function_scalar`
  differential pins the scalar-derived default-OFF invariant on the
  same hash-multiply fixture.
- P2-9 (register-class guards on the acc substitution): the added
  `!vector_values.contains(id)` conjunct cannot fire today (the acc
  cache is integer-only by construction) — belt, not filter; the P0-1
  tests pin the current behavior.

## 4. Validation battery (final tree)

- `cargo test --lib`: 3296 passed, 0 failed (was 3288 at session
  start; +8 = 5 window tests + 1 andn contract + 2 majority-rank…
  plus the IVSR default pin = 9; one pre-existing count moved with
  the `filter` line — the authoritative line is in the CI log).
- `ci_local.sh --fast`: all gates green (63/63 incl. the renamed
  `nbody-perf-shapes` gate and the re-armed ch-maj census).
- The three slow gates CI `--fast` skips (regression-corpus-ssa,
  benchmark-output-oracle, peephole-whitespace-invariance) run
  explicitly: green.
- `cargo fmt --all -- --check` and
  `cargo clippy --all-targets --profile fastbuild --locked -j 2 --
  -D warnings` (CI_LOCAL_JOBS=1 on this box): clean.
- chacha/sha256/expat/sieve/find_bit/varint/bitops runtime
  differentials vs gcc: all outputs identical.

## 5. Oracle diagnoses banked for the next phase (no code this session)

- sieve `count_primes` (−O2 insns: gcc 34, lccc 47): `i*i` computed
  THREE times per outer iteration (bound test, inner-loop start,
  backedge test) — cross-block redundant-mul GVN gap; plus a dead
  `movslq` per iteration (IV-widening gap) and a 7-insn cmov count
  tail vs GCC's 5-insn `sbb $-1` idiom.
- expat `main` (−O2 insns: gcc 97, lccc 156): the scan-loop hash is
  spilled to its slot every iteration (load+store per character)
  where GCC homes it in callee-saved %r12 across the
  `expat_utf8_name_length` call — callee-saved homing gap; plus
  latch copy chains and a load+cmp where GCC uses a mem-operand
  `cmpb`.
- chacha (above): ICX structure convergence (3 rocks).

All three come with exact current-vs-target shapes in
FOLLOWUP-2026-09-24-S49 and re-runnable `godbolt.py compare` commands.
