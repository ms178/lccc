# Batch 6: conditional scalar facts and x86-64 hot-web promotion

## Scope and sources

This is the ten-report scalar cohort selected for batch 6, together with the
requested x86-64 hot-loop callee-saved-register mechanism. The baseline is the
latest main after PR #887, `0c4f3929dd75354d80bae881b6d579e2426fe789`.
The earlier batch-5 head `b215478` remains historical context, not the final
comparison baseline.

Bodies and comments were retrieved through the GitHub API. Exact upstream
identities, timestamps and body/comment digests are in `batch-006-sources.json`.
An upstream report is a source of a reduced contract, not an implementation
specification or proof. None is marked globally solved by this batch.

| LLVM report | Reduced contract and treatment |
|---|---|
| [231586](https://github.com/llvm/llvm-project/issues/231586) | A masked equality implies a separate inequality; remove the redundant Boolean OR/AND condition using known-zero/known-one facts. |
| [231063](https://github.com/llvm/llvm-project/issues/231063) | Under an equality guard, substitute a zero operand in an add/subtract comparison. Shared producers and other comparison users remain untouched. |
| [230800](https://github.com/llvm/llvm-project/issues/230800) | Intersect a small unsigned modular range with a masked equality; also remove the redundant evenness predicate when an affine equality fixes the value to two. |
| [230021](https://github.com/llvm/llvm-project/issues/230021) | Recombine scaled unsigned quotient and remainder with a shared remainder result; separately handle a proven high-bit mask. Runs before constant-division lowering. |
| [229537](https://github.com/llvm/llvm-project/issues/229537) | A conjunction of exact facts proves that both Boolean select arms agree when the guard is true. Also handles the signed-negative/zero-select example. |
| [229332](https://github.com/llvm/llvm-project/issues/229332) | Existing pipeline already reuses the inverse comparison in the reduced C call-spanning example. Retained as a control, not counted as a new optimization. |
| [217230](https://github.com/llvm/llvm-project/issues/217230) | Cancel aligned additions around a logical shift when the final low-bit observation hides every possible wraparound carry. |
| [214397](https://github.com/llvm/llvm-project/issues/214397) | A unique conditional predecessor supplies unsigned-range facts; the already-ifconverted nested-select form is handled locally as well. |
| [213708](https://github.com/llvm/llvm-project/issues/213708) | Low-bit-isolation equality/inequality against zero or one. Zero observations were already handled; observations against one are newly reduced. |
| [210131](https://github.com/llvm/llvm-project/issues/210131) | Zero observations of `x | zext(x != 0)` reduce to the same observation of x. Only a proven canonical Boolean may pass through casts. |

These are nine reports with newly implemented reduced forms and one
already-handled report, not ten independently new optimization families.
The native fixture has 13 newly reduced entry points, two already-handled
controls and three distinct-semantics negative controls.

## Integration and proof boundaries

`bit_idioms_batch6.rs` is a consumer-rewrite helper of the existing bit idiom
pass, not a new independently scheduled pass or environment authority. The
existing `CCC_DISABLE_PASSES=bit_idioms` token covers both the pre-division and
late entry points. No new environment reads are added.

The helper snapshots definitions and use counts. It declines the entire sweep
on duplicate SSA definitions, duplicate block labels, malformed source-span
lengths, exhausted fresh-ID space or a dense-ID watermark above one million.
The ID budget is checked before allocating the snapshot. Plans are complete before mutation,
expansions are applied in reverse coordinate order, and every inserted node
inherits its consumer's original source span. Producers, stores, calls,
volatile loads and other users are not mutated or removed by these plans.
Normal DCE may subsequently retire unused pure instructions.

Facts are bounded exact-bit observations, not a general symbolic execution
engine. Recursive walks are depth-limited, fact sets are small, each consumer
has at most 512 definition lookups, and finite modular-range enumeration admits
at most 16 values. Exhaustion loses an optimization. Predecessors come from the
existing linear-time CFG builder, including switch and indirect edges; a fact
is never inferred merely from one convenient predecessor of a join. Entry
blocks and ambiguous true/false edges supply no incoming facts.

Conditional substitution introduces new pure work only at the consumer, where
its transitive operands already dominate. It does not globally replace a
shared producer based on a condition that is only locally true. Select arm
agreement requires canonical Boolean arms; two arbitrary nonzero integers are
not equated merely because both are true. Integer casts are not generally
transparent. Non-Boolean select reduction is restricted to nested constant
choices; it is not an unconditional constant folder and leaves count-idiom
accumulator networks intact. The Boolean-OR rule may traverse casts only after finding a
canonical comparison result, whose zero/one values survive those casts.

### Modular shift cancellation

For width W, aligned A and a valid logical shift s,

```
((x + A) mod 2^W) >> s = ((x >> s) + (A >> s)) mod 2^(W-s).
```

A final mask `2^k-1`, with `k <= W-s`, hides that wrap. If
`((A >> s) + B) mod 2^k == 0`, the additions in
`(((x + A) >> s) + B) & (2^k-1)` disappear. An unaligned A, an invalid shift,
an arithmetic right shift, a visible carry or an unsupported width declines.
The production arithmetic rules admit U32/U64, not signed division or FP.

### Scaled quotient/remainder

For nonzero unsigned D, `x = D*q + r`, so
`q*(D*S) + r*S == x*S (mod 2^W)`. A shared remainder is retained; only the sum
is replaced. At least one old scaled arm must have a single use. Power-of-two
quotient/remainder spellings and the pipeline's canonical `r+r` scaling are
recognized explicitly; arbitrary reassociation is not assumed.

For the masked form, the quotient contribution must have at least k low zero
bits. A mathematical upper bound `(D-1)*S <= 2^W-1` proves that the old remainder
mask equals clearing the low k bits throughout its whole possible range. Thus
`q*(D*S) + ((r*S)&M)` becomes `(x*S)&~(2^k-1)`. Missing relevant mask bits,
insufficient quotient alignment, mismatched inputs/divisors, signed division,
zero divisors and overflowing remainder bounds decline.

The `0x7000` mask control is distinct from the `0x7800` positive witness, but
can legally simplify by clearing 12 rather than 11 low bits. It must not use
the positive witness's mask. The unit suite checks both legal reductions and
refuses a genuinely holey `0x6800` mask. Negative solver controls disprove a
specific invalid replacement, not every possible optimization of a near miss.

The shared-remainder witness removes the quotient-dependent multiply by 12,
but currently ties its baseline at 13 instructions because of preservation
moves. Its native gate checks that mechanism directly, while the other twelve
new witnesses require fewer instructions. No code-size win is claimed for the
tie.

## x86-64 callee-saved hot-web steals

The existing i686 phase is factored into `backend/regalloc/hot_web.rs` and made
available on x86-64's actual filtered callee-saved pool. This is distinct from
the earlier local-exchange heuristic that regressed gzip. The AArch64-specific
loop-pin policy is retained separately.

A missed eligible loop-phi owner is ranked by its whole web's loop-weighted
uses. The chosen register must already be held and belong to the backend's
available callee-saved pool. The combined cost of **every member actually
removed by whole-group eviction** must be strictly lower than the incoming
web's heat. This fixes underpricing nonoverlapping members of an evicted copy
web. Saturating arithmetic prevents a hot victim from wrapping into a cheap
one. Equal-cost ties are deterministic by register ID and candidate value ID.

Other loop-pair holders, ABI-hinted holders and holders lacking coverage are
protected. Coalesced members are never promoted independently of their owner.
All overlapping holders are accounted for; disjoint survivors keep their
homes. Propagation reestablishes copy-web assignments after the sweep.
The phase runs before segment fill and final phi application, with the existing
post-allocation interference verifier/repair still in place.

No caller-saved or APX register is reclassified as ABI-preserved. Reserved or
clobbered registers are not added to the pool. Frame-pointer policy remains
owned by the backend. The pool test does not depend on XMM availability, so
GPR-only code can benefit. The existing controls are extended to both x86
modes: `CCC_NO_HOT_WEB_STEAL` disables the phase and `CCC_HOT_WEB_STEAL=N` caps
successful promotions (default 3). The implementation does not assert that
three is universally optimal.

`hot_web_callee_saved.c` exercises call-spanning recurrences, cold non-rematable
values with rare-path reads, unsigned wraparound, zero/small/odd trips and a
volatile memory-carried reference. The separate `hot_web_loop` benchmark times
only the optimized recurrence, not that deliberately slow reference. It is a
synthetic register-pressure witness, not an application-speed claim.
The `llvm_batch6_scalar` benchmark separately calls the thirteen new scalar
contracts with deterministic mixed boundary/random inputs and an observed
checksum. It shares the exact native entry points rather than duplicating
their implementations. Both new workloads have bounded instrumentation-size
macros and entries in `callgrind_sizes.json`.

## Reproduction and evidence

Use fastbuild for lccc/lccc-ld and a swap file of at least 3 GiB. On a constrained
4 GiB host, use `-j1`; concurrent Rust code generation and whole-tree rustfmt
can otherwise exhaust memory. Cached compiler packages are extracted into a
private prefix, never over the live root filesystem.

```sh
RUSTFLAGS='-D warnings -C link-arg=-Wl,--no-keep-memory' \
  cargo build --profile fastbuild --bin lccc --bin lccc-ld -j1
RUSTFLAGS='-D warnings -C link-arg=-Wl,--no-keep-memory' \
  cargo test --profile fastbuild -j1
RUSTFLAGS='-D warnings -C link-arg=-Wl,--no-keep-memory' \
  cargo clippy --profile fastbuild --all-targets -j1 -- -D warnings
python3 tests/regression/check_llvm_missed_batch6.py \
  --gcc /path/to/gcc-16 --baseline /path/to/saved-b215478-lccc
python3 scripts/prove_llvm_batch6.py --output proofs.json
CCC_VERIFY_IR=abort CCC_VERIFY_REGALLOC=1 \
  python3 tests/benchmark/run_benchmarks.py --compilers lccc,gcc \
  --gcc /path/to/gcc-16 --candidate before=/path/to/saved-b215478-lccc \
  --cflag=-march=x86-64-v3 --cflag=-fPIE --cflag=-pie \
  --skip-perf --no-pmu-probe --strict --json correctness.json
```

The Rust tests exercise actual emitted plans using an independent scalar IR
interpreter, exhaustive low-word domains, high-sign-bit and wrap boundaries,
shared users, canonicalized doubled remainders, signed/width refusals,
non-Boolean arms, CFG joins/switches/indirect edges, source spans and fresh-ID
exhaustion. The native gate checks five optimization levels plus debug/wrapv,
exact GCC16.2 output, disabled controls and per-function assembly mechanisms.
It is wired to both hosted CI and the local gate mirror.

`batch-006-proofs.json` contains **72 independent Z3 obligations: 58 algebraic
laws and 14 expected counterexamples**, with no UNKNOWN accepted. Most laws use
exact 8/16/32/64-bit vectors. The unsigned division law uses bounded mathematical
integers with modular output instead of a divider circuit; this distinction is
recorded per row. This is not a proof of the Rust matcher or emitted machine
code. Mutation/refusal tests and native differential execution remain separate.

Final-source measurements and exact-source/binary provenance are in
[BENCH-006.md](BENCH-006.md), with raw runtime, correctness and static-count
JSON companions. The 46 original workload binaries remain unchanged against
latest main. The two synthetic witnesses improve by 7.60% and 4.50% in this VM;
these are not application or EPYC results. Hosted status is recorded on PR #891;
earlier-head green checks are not relabeled.
