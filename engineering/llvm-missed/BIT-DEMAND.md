# Bit demand and Boolean cardinality

This batch adds **60 reduced expression contracts with 32- and 64-bit native
companions** (120 width-specific witnesses). Width, signedness, operand-order,
and arithmetic-tree variants are tests, not extra implementation families.
`d_xor` and `d_or` are controls: main already removes their masks in the backend.
They are not claimed as newly improved assembly. The other 58 contracts require
an individually checked mechanism improvement; parity fusion is measured by
removing a population count even when instruction counts tie.

## Implemented mechanisms

| Mechanism | Contracts / important cases | Exact boundary |
|---|---|---|
| Carry-aware demanded bits | `d_add`, `d_sub`, `d_mul` | A demanded bit needs every lower input bit through its position. |
| Constant multiplication demand | `d_mul_tz` | Strip the factor's trailing zeros before computing the input demand. |
| Shift and rotation demand | `d_shl`, `d_lshr`, `d_ashr`, `d_rotate` | AShr must demand the sign bit when any sign copy is observed. Invalid ordinary shifts fail closed. |
| Unary and Boolean demand | `d_not`, `d_neg`, `d_and`; XOR/OR controls | Negation has carry dependencies; Boolean operations are bit-local. |
| Permuted demand | `d_bswap`; IR bit-reversal tests | Apply the inverse bit permutation to the observation mask. |
| Remainder demand through arithmetic | `d_rem_shl`, `d_rem_lshr`, `d_rem_add`, `d_rem_mul` | The constant divisor must be nonzero and divisible by the required low-bit modulus. This extends integer observations's direct remainder observation. |
| Trailing-count invariants | `tz_odd`, `tz_variable_odd`, `tz_neg`, `tz_isolate`, `tz_or_neg` | Preserve the exact lowest set bit, including zero preservation for defined count forms. An even multiplier is not interchangeable. |
| Triangular count invariants | `tz_{xor,or,add,sub}_shift`, `lz_{or,xor}_shift` | Positive, in-range shifts in the appropriate direction; no path-sensitive nonzero assumption. |
| Count through AND-NOT shifts | `tz_andnot_shift`, `lz_andnot_shift` | The least/most significant set bit cannot be cleared by the corresponding shifted copy. |
| Population-count permutations | `pc_swap`, `pc_rotate`; IR bit-reversal tests | Integer bit permutations preserve cardinality. |
| Population parity | `parity_add`, `parity_sub`, `parity_xor`, `parity_not`, `parity_even_xor` | Fuse private count trees via XOR; even-width complement and even-population constants preserve parity. |
| Complementary partitions | `pc_partition`, `pc_variable_partition`, `pc_complement` | Count disjoint parts once or return the word width. |
| Symmetric difference | `pc_symmetric_difference` | `P(x|y)-P(x&y) = P(x^y)`. |
| Boolean cardinality synthesis | The final 18 JSON contracts | Evaluate four per-bit valuations, then synthesize one count or twice one count. No SAT/SMT solver runs inside the compiler. |
| Terminal boolean return | x86 `return_bool` | Clear eax before a non-rax-reading integer flag producer; write al directly; remove the temporary extension. |

The synthesizer handles, among others:

- `P(x&y)+P(x^y) -> P(x|y)`;
- `P(x|y)-P(x^y) -> P(x&y)`;
- `P(x|y)-P(x&~y) -> P(y)`;
- `P(x)+P(y)-2*P(x&y) -> P(x^y)`;
- `P(x)+P(y)-P(x^y) -> 2*P(x&y)`;
- `P(x)+P(y)+P(x^y) -> 2*P(x|y)`;
- `2*P(x|y)-P(x)-P(y) -> P(x^y)`;
- complementary De Morgan expressions, exclusive-part subtraction, and
  three-/four-way partitions.

`P` means word population count. These are exact integer identities, not
floating-point algebra or ordered-comparison cancellation.

## Integration, safety, and cost

`bit_idioms_demand.rs` reconstructs a bounded speculative expression only for a
private path under a constant AND. Original producer values are not repurposed.
Fresh partial arithmetic is explicitly unsigned: discarding an inner signed
mask must not create a signed-overflow assumption. Zero-demand branches are
removed without changing constant shift amounts. Aliases and count-result
casts participate in private-use checks.

The local SSA index starts from a full function scan and is updated after each
committed rewrite, including newly emitted definitions and use edges. Later
consumers see canonical counts rather than resurrecting obsolete input trees.
Multiply-defined destinations reject the entire new sweep before any edits.
Fresh IDs and source spans are preserved; exhausted ID/node budgets fail closed.

The demanded walk is limited to 32 visits. Count normalization is bounded and
rolls back rather than committing a partial normalization at its limit.
Cardinality synthesis has at most two Boolean leaves, 32 arithmetic visits,
64 Boolean visits, and coefficient magnitude 16. It requires at least two
private count producers, consistent word widths, and a constant/Boolean/twice-
Boolean output table. Arithmetic-only paths and every count/cast alias must be
private. A strength-reduced `count+count` may consume exactly its two uses,
never an additional external use. Arbitrary three-input Boolean synthesis,
shared-count duplication, and general integer-cast motion are not enabled.

The new sweep follows the existing observer with fresh indexing. Its early
entry runs only when unsigned remainder exists, before division expansion.
Both entries inherit the exact `bit_idioms` disable token. A similarly named
prefix token must not disable them.

The x86 return rewrite is terminal and local. It refuses rax aliases in the
comparison, memory comparisons, preserved/high-byte temporary registers,
intervening labels/CFI, nonterminal extensions, protected/inline-assembly
regions, and mismatched extension widths. All supported integer conditions
are tested. Alignment defaults, size-mode alignment, and explicit alignment
options are unchanged. The pass has an exact `CCC_PEEPHOLE_SKIP=return_bool`
ablation control.

## Correctness evidence and reproduction

The native checker requires GCC **16.2**, optionally Clang **23.1.3**, enables
`CCC_VERIFY_IR=abort`, treats compiler warnings as errors, and tests O0, O1,
O2, O3, Os, O3+g and O3+fwrapv for both source widths. Each suite checks 65,548
input pairs, including sign-bit, zero, all-ones, and shift-boundary values.
Undefined-zero CLZ/CTZ builtins are not called with zero.

IR tests cover signed/unsigned 32/64-bit semantics, native count-result casts,
shared aliases/counts, invalid shifts, zero/nondivisible remainder divisors,
carry/sign demand, non-SSA rejection, bounded walks, double-run idempotence,
verifier invariants, all 14 nonconstant Boolean tables, two-term combinations,
weighted count ownership, and count/parity compositions.

The optional Z3 script performs **71 checks**, including exact 32/64-bit
word-demand laws, integer remainder lemmas, per-bit cardinality identities,
and counterexamples to omitted guards. Count-position checks use an 8-bit
symbolic domain: they are **not** a full-width formal certificate for the Rust
implementation. Every UNKNOWN is explicit and fails the command.

```sh
scripts/build_lccc_fast.sh
python3 tests/regression/check_llvm_bit_demand.py \
  --compiler target/fastbuild/lccc --gcc /path/to/gcc-16 \
  --clang /path/to/clang-23.1.3 --baseline /path/to/saved-main-lccc
python3 scripts/prove_bit_demand.py --output work/bit-demand-proofs.json
python3 scripts/compare_integer_observations.py --suite bit-demand \
  --compiler target/fastbuild/lccc --baseline /path/to/saved-main-lccc \
  --gcc /path/to/gcc-16 --clang /path/to/clang-23.1.3 \
  --output work/bit-demand-code-size.json
```

Add `--wide` to the size comparison for the 64-bit companion. Run the proof
script in an environment with `z3-solver` installed; it is not a compiler or CI
dependency. Native checks are wired into both hosted CI and the local CI gate.

## Runtime experiment

`compare_llvm_dispatch.py` builds identical GCC-16.2 driver objects for saved
before/after compilers. It measures direct and indirect calls at explicit
16/32/64-byte callee alignments, four link paddings, a fixed available CPU,
seed-178 interleaving within adjacent layout blocks, two warmups, and configurable timed rounds (12 in the final paired CPU-time run). Checksums must agree
across every variant and mode. Compiler/source hashes, raw seconds, and machine
information accompany the medians. `--reference` adds an older compiler at
16-byte alignment; `--ablate-return-bool` isolates the return rewrite. Use
`--suite bit-demand` for the new witness cohort.

The historical indirect regression's old raw artifacts are unavailable. Fresh
controlled measurements, not the historical percentages, are the evidence for
this change. Layout effects are real; alignment experiments are not proposed
compiler defaults. The host's hardware-cycle perf-event probe returned ENOENT,
so no hardware-counter explanation or IPC claim is asserted. Shared-host wall
timings include process startup and cannot establish universal superiority.

### Recorded results and open acceptance gate

The final fastbuild, all-target tests (**5,448 library + one additional test**,
10 inherited ignored), strict Clippy, seven-mode 120-witness native differential,
IR verifier, exact policy controls, and 71 optional proof checks pass. Clippy's
policy probe documents one newer lint name absent on the pinned MSRV; this is
an existing compatibility entry, not an emitted build warning.

At O3/v3, both widths have **55 fewer / 5 equal / 0 more** instruction counts
versus saved main. Totals including RET are **472 -> 234** (32-bit witnesses)
and **490 -> 226** (64-bit witnesses). These totals do not predict throughput.
Against GCC 16.2 the fewer/equal/more counts are 36/21/3 and 35/22/3; against
Clang 23.1.3 they are 31/15/14 and 31/14/15. No universal compiler lead is claimed.

**The requested performance acceptance gate remains OPEN.** The return rewrite
is a tested code-size improvement, **not a demonstrated across-layout runtime
fix**. The final paired child-CPU-time deltas at the unchanged 16-byte alignment
are below (negative means faster). Earlier wall-only controls are retained too,
not discarded in favor of a winning run.

| Cohort / mode | pad 0 | pad 16 | pad 32 | pad 48 |
|---|---:|---:|---:|---:|
| Existing 45, direct vs main | -0.30% | +0.45% | +0.46% | +0.11% |
| Existing 45, indirect vs main | +1.00% | +1.09% | -0.50% | +0.53% |
| Existing 45, indirect vs pre-integer observations | -4.89% | +1.55% | -5.74% | -3.14% |
| New 60, direct vs main | **+11.69%** | -4.07% | **+4.18%** | -2.25% |
| New 60, indirect vs main | -0.67% | -7.92% | -4.73% | -5.39% |

The return-pass ablation is also mixed. The historical 7.9% indirect result was
not reproduced as a persistent across-layout loss, and the fresh experiment
exposes **direct-dispatch layout regressions** despite substantially smaller
callees. The evidence does not justify claiming the user's runtime requirement
is met. No blanket alignment change or benchmark-specific optimization disable
has been used to conceal that result. This work must remain a draft until that
performance question is resolved or the acceptance scope is explicitly revised.

`bit-demand-{code-size,wide-code-size}.json` holds per-function static counts;
`bit-demand-{dispatch,new-cohort}-{wall,cpu}.json` holds both timing protocols;
`bit-demand-runtime-summary.json` retains the compact default-alignment deltas;
`bit-demand-proofs.json` retains every stated proof/guard result.

## Upstream scope

Demand reconstruction relates to the masked-arithmetic reductions discussed in
LLVM #230022. Odd-product count invariants relate to reductions around #213877;
this does not implement its general path-sensitive CFG problem. The permutation
work does not solve every nested reconstruction in #219334. The cumulative
inventory remains scoped per reduced pattern; no report is declared globally
solved merely because one of these witnesses improves.
