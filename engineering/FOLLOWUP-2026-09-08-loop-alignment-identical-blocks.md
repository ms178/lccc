# Follow-up: hot-loop alignment, GAS alignment grammar, and the identical_blocks miscompile — 2026-09-08

**Base:** `main` @ `2026172` (upstream), session commits on top.
**Scope of this session:** (1) assembler-level `.p2align N,fill,max-skip`
support with relaxation-safe markers; (2) a dedicated hot-loop alignment
pass with an oracle-calibrated directive policy and full `-falign-*` flag
machinery; (3) a critical pre-existing `identical_blocks` peephole
miscompile that the new alignment directives exposed deterministically,
root-caused and fixed; (4) the outer-loop vectorization gap analysis with
Compiler Explorer evidence and an implementation plan for the next
session; (5) a second pre-existing -O3 miscompile (partial unroll of a
loop with a side-effecting latch) found by the new kernel corpus and
fixed — §3.

---

## 1. What landed

### 1.1 Assembler: GAS-parity `.p2align N[, fill][, max-skip]` (commit `0b8558b`)

`AsmItem::Align(u32)` became `Align { align: u64, fill: Option<u8>,
max_skip: Option<u64> }`; the parser accepts the full GAS grammar for
`.p2align` / `.align` / `.balign` on x86-64 and i686 (shared parser), and
the shared ELF writer honors it through jump relaxation. Every semantic
was verified against a locally built **GNU as 2.47** (the certified
oracle; `scripts/ensure_gas_247.sh x86_64-linux-gnu`) before
implementation:

| Input | GAS 2.47 behavior | lccc now |
|---|---|---|
| `.p2align 4,,10` needing ≤ 10 bytes | pads | pads |
| `.p2align 4,,10` needing > 10 bytes | no padding, **addralign still 16** | same |
| `.p2align 4, 0x5a` in `.text` | pads verbatim with 0x5a | same |
| `.p2align 4, 0x90` in `.text` | optimal multi-byte NOPs | same |
| `.p2align 4, 300` | fill truncated to 0x2c | same |
| `.p2align -1` / `.p2align 64` | warn, clamp exponent to 63 | same (no warning yet — see TODO) |
| `.p2align 63` at offset 0 | accepted, addralign stays 1 (signed overflow internally) | same |
| `.byte 1; .p2align 63` | error "jump over nop padding out of range" | same error |
| `.balign 12` / `.align 6` / `.balign -1` | error "alignment not a power of 2" | same |
| `.p2align 0` | aligns to 1, nothing happens | same |

Two writer-level fixes rode along:

* **Markers are recorded even when the initial padding is zero.** Jump
  relaxation only shrinks code; an exactly-satisfied alignment can need
  padding after relaxation. Previously the marker was only created for
  material padding, so a satisfied alignment could silently end up
  unaligned in the final object. The fixup pass now re-evaluates the
  fill and max-skip decisions against the relaxed offset (two new
  differential cases cover both re-evaluation directions).
* `sh_addralign` handling for the GAS signed-overflow corner (exponent
  63) is overflow-safe (`div_ceil`, u64 alignment math).

Evidence: `tests/asm-diff/p2align-maxskip.casefile` (21 cases incl. two
relaxation re-evaluation cases and four reject cases); full corpus
**776/776 vs GAS 2.47** (`scripts/asmdiff.py --as <gas-2.47>`).

### 1.2 Hot-loop alignment pass (`src/passes/loop_align.rs`)

A dedicated post-layout pass replaces the previous two alignment
mechanisms:

* the PGO-only `pgo::block_align` 16-byte hints (now the *hotness input*
  for the pass, not the directive source), and
* a dead `.p2align 5` emission path in `machinst_emit.rs`
  (`emit_machinsts` scanned for `MachInst::Label` backward branches, but
  **no lowering ever constructs `MachInst::Label`** — verified by grep;
  the code could never fire and is removed with an explanatory comment).

The pass runs **post-optimization, post-label-renumber, post-PGO-layout**
in `driver::pipeline`, when the block order is the emission order. It
finds natural loops on the final CFG, classifies each loop body as
vector (contains a SIMD intrinsic — `IntrinsicOp::is_vector_op()`, new)
or scalar, and records directives keyed by block label; `generation.rs`
emits them immediately before the block label.

**Directive policy (oracle-derived, GCC 16.2 / Clang 23.1 / ICX latest /
ICC 2021.10, x86-64-v3, verified 2026-09-08):**

| Case | GCC 16.2 | Clang 23.1 | ICX | lccc now |
|---|---|---|---|---|
| vectorized loop header | `.p2align 5/4/3` cascade (plain 32 B), **even under `-fno-align-loops`** | `.p2align 4` | `.p2align 4, 0x90` | `.p2align 5` |
| scalar loop header | `.p2align 4,,10` + `.p2align 3` | `.p2align 4` | — | `.p2align 4,,10` + `.p2align 3` |
| -O1 | loop headers aligned | `.p2align 4` | — | aligned (as -O2) |
| -Os / -Oz | **no directives at all** | none in text | — | none |
| function entry -O1..-O3 | 16 | 16 | 16 | 16 (unchanged) |

Microarchitectural rationale (Raptor Lake): the DSB tracks 32-byte
windows inside 64-byte lines; a 32-byte-aligned vector-loop head always
starts a fresh window, and the bounded 16-byte scalar cascade keeps the
loop inside few fetch windows while capping padding at 10 bytes (the
measured gzip `longest_match` bloat from *unbounded* 32-byte scalar
alignment is exactly what the max-skip clause prevents).

One deliberate improvement over GCC: `-fno-align-loops` also suppresses
the 32-byte vector-loop alignment (GCC's vectorizer alignment cannot be
turned off), so the whole policy is A/B-testable.

**Flags (GCC grammar, validated):** `-falign-loops[=N[:M]]`,
`-falign-jumps[=N[:M]]`, `-falign-functions[=N[:M]]` and the `-fno-`
forms; non-power-of-two / non-numeric values are rejected at parse
(GCC parity). `-falign-functions=64` produces `.p2align 6` and
composes with `-fpatchable-function-entry`. Kill switch:
`CCC_NO_LOOP_ALIGN=1`.

**Audit:** `scripts/check_loop_alignment.py` reconstructs the CFG from
the emitted assembly (dominator-based natural-loop discovery, mirroring
`loop_analysis::compute_loop_body` exactly, incl. self-loop bodies),
classifies loops by SIMD mnemonics (scalar SSE excluded: `movss` is not
packed), and asserts the full directive contract: default policy at
-O1/-O2, no padding at -O0/-Os/-Oz, `-fno-align-loops` suppression,
custom directives, kill switch, function-alignment matrix, and flag
rejection. PASS.

### 1.3 The miscompile the alignment pass flushed out (FIXED)

`tests/regression/vectorize_int_map_lanes.c` at `-O2 -march=x86-64-v3`
SIGSEGV'd **20/20 runs** with loop alignment on, passed with
`CCC_NO_LOOP_ALIGN=1`; the pre-alignment baseline passed. Root cause
chain (each step verified experimentally):

1. The IR was **identical** with/without the pass (the alignment pass is
   pure analysis) — the divergence was in codegen's text post-processing.
2. `dmesg` captured the faulting instruction: `vmovdqu %ymm0,(%rdi)` in
   `fill`'s 64-byte-stride zeroing loop, with **no setup between the
   loop guard and the first store** — pointer/counter/accumulator never
   initialized.
3. Bisect with `CCC_PEEPHOLE_SKIP` isolated **`identical_blocks`**.
4. The pass's block scan stopped at any directive that is not
   `.loc`/`.file`, so a `.p2align` between a guard's `jae` and the next
   label **severed the fall-through edge** (the adjacency test
   `X.end == B.start` failed with the directive line in between).
   Blocks entered only by fall-through therefore recorded an **empty
   predecessor set**, and "empty == empty" satisfies the
   identical-predecessors merge condition — a reachable setup block was
   merged away while its guard still fell through into it at runtime.
   This was a **latent pre-existing bug**: the old PGO-only
   `.p2align 4` block alignment could trigger it too; the new pass made
   it deterministic at every -O2 loop.

**Fix (all four pieces, in `identical_blocks.rs`):**

* Alignment directives (`.p2align`/`.align`/`.balign`) are transparent
  in the block scanner, the block-terminator scan, and the
  clean/flag-dependence predicates — the edge model sees through
  padding, as it already did for `.loc`/`.file`.
* **Function entry regions are first-class predecessors**: the lines
  before a function's first `.LBB` block were never modeled as a block,
  so entry-region jumps and the positional entry fall-through were
  invisible (blocks entered only from there had empty pred sets and
  merged — that is how the *positive-control* tests merged historically).
  Both edge kinds are now attributed to a per-function pseudo label
  (NUL-prefixed, cannot collide with a real label).
* **A block with a fall-through entry can never be the deleted
  duplicate**: predecessor sets record label names, not edge kinds, so
  "identical preds" alone cannot prove the deleted block's entries are
  all rewritable jumps (P can jump to the canonical and fall through
  into the duplicate, making both pred sets `{P}`). Fall-through entries
  are tracked explicitly now.
* **Empty predecessor sets never merge** (defense in depth: empty means
  "no entry the model can see", which is dead code at best and an
  invisible entry at worst). This also covers `loop`-instruction targets
  (`loop` classifies as `LineKind::Other`, so such targets have no
  recorded pred — they can no longer be merged).
* Merged-away duplicates keep their trailing alignment directives: the
  padding belongs to the *following* block's label, which survives the
  merge (dropping it would silently unalign a loop header the layout
  pass deliberately aligned).

Two new unit tests pin the contract
(`alignment_directive_does_not_sever_fallthrough_entry`,
`jump_entered_identical_blocks_still_merge` — the second is the positive
control proving sound merges still fire), and the end-to-end reproducer
is `vectorize_int_map_lanes.c`, which stays in the GCC-differential
regression suite.

### 1.4 Validation matrix (all green, fastbuild @ session HEAD)

| Gate | Result |
|---|---|
| `cargo test --lib` | **2071 passed**, 0 failed |
| `scripts/run_regression_suite.sh` (GCC differential) | **PASS=655 FAIL=0** (AB-diff 0) |
| `scripts/asmdiff.py` vs GAS 2.47 | **776/776** |
| `scripts/check_benchmark_outputs.sh` (39 workloads × 4 levels) | **PASS=180 FAIL=0** |
| `scripts/check_loop_alignment.py` | PASS |
| `tests/regression/run_pgo_roundtrip.sh` | OK (incl. sections+alignment roundtrip) |
| i686 shared-writer path (`lccc-i686 -falign-loops=16`) | assembles to valid ELF32 |
| paired A/B wall-clock (5 workloads, 9 reps) | geomean 0.9949 — no measurable difference on the VM (expected: alignment pays on real silicon DSB; the policy is GCC's hardware-validated one) |
| code-size cost | +78…+289 bytes of directive text per benchmark file (most padding suppressed by the max-skip clauses) — same class as GCC -O2 |
| compile-time cost | within noise (42 vs 44 ms on sha256_transform) |

---

## 2. Outer-loop vectorization gap (oracle evidence + plan)

Probing `outer_vec_shapes.c` (s1–s8, kept in the session workspace)
against the oracles at `-O2 -march=x86-64-v3`:

| Shape | GCC 16.2 | Clang 23.1 | lccc -O2 |
|---|---|---|---|
| s1 `dst[i*4+j] = src[i*4+j]*3+1` (const trip 4) | flattens → 1× `vmovdqu` 4-lane loop | flattens → 4× unrolled YMM loop | inner fully unrolled, **scalar ×4** |
| s2 f32, trip 8 | flattens → YMM loop | flattens | **scalar ×8** |
| s3 `dst[i][j] = src[i][j]+7` (row pointers advance) | **misses** (scalar) | vectorizes with runtime alias checks | scalar |
| s4 inner reduction, row reset | flattens → YMM + horizontal add | flattens | scalar ×8 chain |
| s5 i64 trip 4 | flattens | flattens | scalar ×4 |
| s6 i16 trip 4 | flattens (packed) | flattens | scalar ×4 |
| s8 stride-2 in j | vectorizes | vectorizes | scalar |

lccc's loop vectorizer only processes **innermost** loops; a constant-trip
inner loop is fully unrolled by `loop_unroll` at -O3 before the
vectorizer ever sees the nest, and the unrolled scalar copies never
re-form as a vector loop.

**The differentiation target is s3**: GCC misses it, Clang needs runtime
alias checks — a restrict-aware flattener wins outright.

**Plan (next session, in dependency order):**

1. **Perfect-nest flattener** (`passes/loop_flatten.rs`, new): for an
   outer loop O with unique preheader and a single dominated inner loop
   I where (a) I's header is O's only block aside from O's own
   preheader/latch skeleton (perfect nest), (b) I's IV init is const-0
   and its bound is loop-invariant in O, (c) I's exit block is O's
   latch, and (d) every memory access in the nest is
   `GEP(base, affine(iv_o, iv_i))` with the same base+elem size —
   substitute `iv_f = C_o*iv_o + iv_i` (C from the affine proof),
   rewrite the exit comparison, delete I's CFG skeleton. Then the
   existing map/reduction vectorizer sees a plain 1-D loop. Guard on
   `opt_level >= 2 && !size_opt`, kill switch `CCC_NO_LOOP_FLATTEN`.
2. **Correctness oracle first**: extend the probe corpus into
   `tests/regression/loop_flatten_*.c` with checksummed outputs
   (sizes 0..17 + boundary cases: n=0, n=1, non-multiple trips,
   negative strides via descending loops, s8's stride-2 rejection).
3. **s3 (row pointers)**: teach the flattener that `dst`/`src` advancing
   by a constant per outer iteration is the same affine proof on the
   *pointer* phi — this is the shape GCC still misses.
4. Re-run the Godbolt oracle matrix on the probe corpus; the target is
   to beat GCC on s1–s6 and s3 outright (fewer instructions, no runtime
   alias checks thanks to `restrict`).
5. A observed peephole gap to fix on the way: `movl $1, %ecx; addl
   %ecx, %eax` should be `addl $1, %eax` (immediate staging survived in
   the s1 unrolled body) — small `local_patterns` fix, separate commit.

---

## 3. Second miscompile found the same day: partial unroll of a loop with a side-effecting latch

Landing the §2 probe corpus as `tests/regression/outer_loop_shapes.c`
immediately paid for itself: kernel s4 (per-row reduction, the most
classic perfect-nest shape) **miscompiled at -O3** — odd rows kept their
previous contents while even rows were correct (20/20 deterministic;
GCC agrees with itself at every level).

Root cause (loop_unroll.rs `do_unroll`): the partial unroller clones
`body_work` but NEVER the latch — after unrolling by k the latch runs
once per k source iterations, shared by all clones. Its eligibility
analysis (check 6) validated `body_work` for calls/atomics but nothing
validated the latch. s4's shape after inner-loop full unrolling puts the
row-sum store (`out[i] = s`) in the OUTER latch alongside the IV bump,
so the ×2 unroll computed both row sums but stored only row i's — the
clone's sum was dead, DCE ate its loads, and the loop looked perfectly
healthy in the final IR.

Fix (check 7b in `analyze_loop`): a candidate is rejected unless the
latch is pure IV bookkeeping — no side-effecting instruction (any
Store, Call, InlineAsm, atomics, alloca family, volatile load) and no
latch-defined value that escapes the latch (any use in another block's
instructions, terminators, or phi incomings; the IV increment itself is
the sanctioned exception, retargeted in place by Step 4). Pure in-latch
chains whose results stay in the latch remain eligible.

* Unit tests: `side_effecting_latch_blocks_partial_unroll` (with a
  positive control that the same loop with the store in the body still
  unrolls) and `escaping_latch_def_blocks_partial_unroll`.
* End-to-end: outer_loop_shapes.c matches GCC at -O0/-O1/-O2/-O3/-Os/-Oz
  × {default, -march=x86-64-v3} (12 combos), plus PGO generate/use
  roundtrip and i686 ELF32.
* Perf: paired same-window A/B pre-fix vs post-fix binary,
  34 benchmarks × 9 reps → geomean 1.0009 (below the 1% noise floor).
  The rejected unrolls were of loops the old binary miscompiled, so
  nothing legitimate was lost.
* Full matrix after the fix: 2073/0 lib tests, 656/656 regression
  (corpus now included), 776/776 asmdiff vs GAS 2.47, 180/180
  benchmark output gate, loop-alignment audit PASS, clippy clean.

Known limitation (deliberate): loops whose latch carries real work are
now simply not unrolled. The proper follow-up is latch cloning — make
`do_unroll` clone the latch per iteration like GCC does — which is the
same machinery the outer-loop flattener (§2) needs for its epilogue.

**Also landed (peephole):** `fold_staged_imm_into_alu`
(local_patterns.rs, skip key `staged_imm_alu`) folds an immediate staged
through a register into its single adjacent ALU consumer
(`movl $1, %ecx; addl %ecx, %eax` → `addl $1, %eax`) under the shared
three-proof liveness contract (dataflow / block-local write-before-read /
whole-function textual uniqueness). This was the `+1` staging surviving
4× per iteration in s1's unrolled map body where GCC emits the immediate
form directly. Five unit tests including the conservative no-CFI
fragment contract and both declination cases (later read; register used
in the consumer's address). Validation identical to §3's matrix (2078/0
lib tests, 656/656, 776/776, 180/180, audit PASS, clippy clean);
perf A/B geomean 0.9993 (VM noise floor — the win is 4 fewer
instructions per unrolled body where the shape occurs; .text −64 B at
-O3 on the kernel corpus).

## 4. TODO / known gaps (ranked)

1. **Outer-loop flattener** (above) — the largest measured codegen gap
   in the corpus; full plan in §2. The correctness oracle
   (tests/regression/outer_loop_shapes.c, all 8 kernels checksummed
   against GCC across 12 opt-level/march combos) is already in the tree
   and wired into the regression suite.
2. **Latch-cloning partial unroll** (§3 above) — restores ×2 unrolling
   for latch-store loops (row-sums, strided writes) instead of
   rejecting them; shares machinery with the flattener epilogue.
3. **GAS warning parity**: `.p2align -1` / `.p2align 64` should print
   "alignment too large: 63 assumed" (currently silent clamp). Cosmetic
   but cheap.
4. **`-falign-loops` arm/riscv backends**: the flags parse for all
   targets but only x86-64 consumes them (AArch64/RISC-V emission is a
   follow-up; their function alignment is unaffected).
5. **Real-hardware alignment A/B**: the VM cannot measure DSB effects;
   run the paired A/B (`scripts/perf_ab.py --env CCC_NO_LOOP_ALIGN=1`)
   on the i7-14700KF with `perf stat` (frontend-bound slots) to
   calibrate whether the scalar cascade's max-skip of 10 is right for
   Raptor Lake (GCC's value) or should be tuned.
6. **Linker oracle** (standing user request): extend the linker
   comparison tooling to the mold X86/i686 build presets (mold's CMake
   `-DCMAKE_CXX_COMPILER_TARGET` presets — look up exact option names
   before building) so lccc-ld is compared against mold 2.42 / lld 23.1
   / bfd 2.47 on the same footing as the compiler oracles.
7. **`identical_blocks` residual audit**: `jcxz/jecxz/loop` classify as
   non-branches (`loop`) or branches (j*) — `loop` is now safe via the
   empty-preds invariant, but a dedicated classification for the
   `loop`/`jecxz` family would make the edge model complete rather than
   safe-by-invariant.
8. **ChaCha20 quarter-round AVX2 vectorization** (carried over from
   STATE.md: ICX does it in 74 insns vs lccc 592) — after the flattener,
   the interleave machinery may apply.

## 5. Harness notes

* GAS 2.47 lives in `/home/user/.cache/gas-2.47-x86_64-linux-gnu/bin/as`
  (snapshot-excluded — rebuild with `scripts/ensure_gas_247.sh
  x86_64-linux-gnu <prefix>` after a wipe; the riscv64 default prefix is
  what the script's no-arg form builds).
* The 4 GB `/swapfile` must be recreated after a harness wipe
  (`scripts/ensure_swap.sh`; the restore script does this).
* Session snapshots: `scripts/lccc-snapshot.sh "<slug>" "<desc>"` —
  canonical deliverable `/home/user/ms178-1.patch`, ledger + tarball +
  bundle under `/home/user/artifacts`.
