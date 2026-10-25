# Fixed SSE2 shuffle composition

## Audit and accounting

This batch reads **26 previously unreviewed original LLVM issue bodies and all
accessible paginated conversation comments**, not just their titles. The source
receipt is [shuffle-composition-sources.json](shuffle-composition-sources.json):
original URLs, update times, archive SHA256, API comment counters and a specific
mechanism/refusal for every report. All 26 counters agree with the accessible
comments. Linked Godbolt/Alive2 pages, hidden history and review threads were not
retrieved or executed; upstream closure is not evidence of a local fix.

**All 26 original report scopes remain deferred.** The new implementation is a
useful, deliberately narrower SSE2 seed, not 26 solved bugs. The inventory now
has **170 reviewed scopes: 52 limited, 118 deferred; 2,941 unreviewed** out of
3,111. Earlier receipts retain their historical totals.

| Reports | Missing mechanism / why not claim coverage |
| --- | --- |
| #29612, #86068, #120764 | Two-source/partial-mask composition, wider shapes and target costs |
| #40778 | Partial stores, memory demand and dead-store proof |
| #46238 | Min/max/FMA/funnel intrinsic factoring, poison and FP side effects; only private integer/bitwise binops are admitted here |
| #55066, #98346, #121823 | AVX2 arbitrary-byte/zero-producing masks, legalization, constants and port pressure |
| #55994, #111611, #120772, #144656 | Length-changing splats, extraction, truncation, concat/interleave |
| #60632, #144227, #170665 | Demanded elements, reductions, shared users, dominance-safe reuse and cycles |
| #62688, #96884, #104482, #156853, #219717 | Byte provenance, bitcasts, widening and target-specific legalization |
| #63946 | Actual shared eight-dword compare/reduction network; private four-dword 57-then-78 is only a related seed |
| #69929 | Fixed-order recurrence PHIs/history reduction |
| #86079 | Partial shuffle/fneg and sign-bit mask costs |
| #120775 | Select distribution can grow three instructions to four |
| #173035, #230345 | Shared producers and accurate retained-cost accounting; do not credit instructions that calls keep alive |

## What changes

The GNU four-lane 32-bit single-source shuffle previously emitted opaque
memory-to-memory `Pshufd128`. For x86 with effective SSE2 permission, ordinary
(default-address-space, nonvolatile) operands now emit canonical typed
`VecLoad → VecShufdI32x4 → VecStore`. Whole-vector SSA can expose the chain to
register algebra. The F32/I32 **home family is preserved**; the permutation itself
moves raw 32-bit images, including signaling-NaN payloads, without FP arithmetic.
Both operand expressions are still evaluated once, including an unused source.
Mixed-source, volatile, named-address-space and non-SSE2 paths keep their existing
lowering. This does not reinterpret GNU `-1` lanes as LLVM poison: the existing
choice of lane zero is retained.

The existing x86 machine optimizer already composes adjacent PSHUFDs (including
the native inverse/composition examples here). This is **not** credited as a new
backend mechanism. The typed bridge reduces redundant copies; doing composition
in register IR also exposes identity/removal before register allocation. Only
private-binop factoring demonstrates additional shuffle removal against the
isolated `vecrelations` disable control.

After promotion, the register matcher applies two exact rules:

* Compose a **private** `shuf(shuf(x, a), b)` using
  `c[l] = a[b[l]]`, for all 256-by-256 immediate pairs. Identity (`0xe4`) becomes
  a Copy. Shared inner producers are not bypassed for speculative savings.
* For two private same-mask shuffles, factor
  `op(shuf(x,m), shuf(y,m)) → shuf(op(x,y),m)` for 128-bit bitwise AND/OR/XOR and
  modular 8/16/32-bit ADD/SUB. Each dword contains complete arithmetic lanes,
  so duplicating/discarding dwords commutes with those lane-local operations.
  SUB operand order is preserved. F32/F64 XOR is only register-image bitwise XOR.

There is no FP arithmetic factoring (discarded lanes could change exceptions),
no arbitrary dword factoring of I64 ADD/SUB (carries cross dword boundaries), no
256-bit extension, no cast peeling, and no shared-producer removal credit.
Nonconstant/out-of-range masks and malformed arities refuse. Replacement SSA IDs
are fresh, source spans follow the existing replacement framework, and normal
DCE removes only genuinely dead private producers. No global expression cache
or cross-block rewrite is introduced. The misleading shuffle opcode lane
documentation is corrected to the actual PSHUFD selector encoding.

## Evidence and limits

`src/passes/vec_shuffle_tests.rs` exercises every immediate composition through
the production matcher and an independent lane interpreter (65,536 pairs), every
mask across the admitted integer families, byte/word carry boundaries, subtraction
order, and refusals for shared/mismatched masks, copies, invalid immediates, FP
arithmetic, I64 arithmetic and wider registers.

`tests/regression/shuffle_composition.c` provides 26 out-of-line native kernels:
16 compositions (including float payloads and second-source selection), five
private-binop cases and five mixed/shared/mismatched controls. Its independent
unsigned scalar oracle checks **425,984 lanes**, including high-bit values,
NaN encodings and random images, and verifies evaluation of both operand calls.
`check_shuffle_composition.py` checks modern GCC and optional exact Clang,
optimization/debug/AVX2/non-SSE2 modes, i686 compile-only, isolated pass-disable
correctness and actual emitted shuffle counts. The disable control leaves the
new typed bridge and vector SSA active; it is not the frozen baseline.

Hosted CI runs the native checker with the existing packed-vector gate and the
exact Debian GCC lane. Broad local CI is intentionally delegated to GitHub.
Runtime evidence is measured separately by `bench_shuffle_composition.py`, using
frozen pre-change binaries, separate kernel/driver translation units, no LTO,
pinned execution, two warmups and alternating paired samples. Calls and memory
overhead are included. Only measured kernels on this host support speed claims;
fewer instructions are not a claim of universally optimal or “godlike” runtime.

### Measured runtime receipt

[Raw paired samples](shuffle-composition-runtime.json) use 67,108,864 iterations,
2 warmups and 15 alternating pairs per kernel on this Intel(R) Xeon(R) Processor @ 2.60GHz VM,
pinned to one allowed CPU. Baseline and candidate compiler hashes, source hash,
compiler identities, checksums, all samples and paired bootstrap intervals are
in the receipt. The frozen baseline also independently passed the full native
fixture with checksum `d03014c622561964 425984`.

| Kernel | Median paired baseline/candidate | Paired bootstrap 95% interval | GCC/candidate median | Clang/candidate median |
| --- | ---: | ---: | ---: | ---: |
| Inverse permutations | 1.026x | 1.007–1.045 | 0.994x | 1.002x |
| Composed permutations | 1.319x | 1.313–1.343 | 1.017x | 1.007x |
| Private ADD | 1.019x | 1.015–1.026 | 0.997x | 0.983x |
| Private SUB | 1.028x | 0.989–1.031 | 1.091x | 1.042x |
| Shared-producer control | 1.428x | 1.416–1.482 | 0.825x | 0.884x |

Ratios above one favor the candidate. SUB's interval includes one, so these
samples **do not establish a SUB speedup**. Private-binop shuffle removal gives
only small runtime gains in this call/memory-bound test; the larger composition
and shared-control gains primarily reflect copy removal by the typed bridge.
The shared-control gain is not evidence of relaxing the private-producer rule.
The candidate remains slower than modern vendors on that control, and these
microkernels do not establish whole-program, every-CPU or “godlike” performance.
