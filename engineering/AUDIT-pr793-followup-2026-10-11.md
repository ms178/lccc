# PR793 follow-up: independent review, measured changes, and remaining losses

Base: **636289180fe6b05e6b2c19c8c7a4cf11d922a2e2**, latest observed
`ms178/lccc` main (merge PR793). This supersedes the PR791 document as the
current session's disposition; it does not rewrite that historical evidence.
No numerical quality score or claim of compiler-wide soundness is made.

## 1. Findings F1–F7: disposition and qualifications

| Finding | Work performed | What the evidence does and does not establish |
|---|---|---|
| F1: missing guard codegen contracts | Added baseline/v3 `clz_guard31`, narrow-result and negative input-cast guard checks. Positive W−1 uses one BSR/LZCNT and no safety OR. Negative narrowed/widened **input** cases retain a guard. | The no-OR assembly assertion alone does **not** establish Select→Copy: CVP can remove that OR independently. The IR matrix below checks the simplifier result directly. |
| F2: unchecked narrowing **result** casts | Expanded the requested 48 combinations to **72**: four literal carriers × three fallbacks × six result-cast forms (none, same-width, I8, U8, U16, wide). Added C narrow-to-ushort positive W−1 and negative W fallback/width mismatch cases. | CLZ32(x\|1) lies in 0…31 and CLZ64(x\|1) in 0…63. Even I8 holds all these values, so result narrowing is safe for the proved W−1 Copy path. This does **not** justify peeling an input truncation or building a defined-zero count of the wrong width. |
| F3: no count cases in oracle/golden gates | Delta corpus **5→9**, adding `clz_full`, `ctz_full`, `clz_guard32`, `ctz_guard64`; golden gate **9→11**, adding portable counts and safe guards. Regenerated both committed baselines with the tools. | No ceiling was raised to conceal a regression. Older unrelated baseline rows also refreshed downwards; those are not credited as gains from this patch. Initial strict CE revalidation hit HTTP 429; a later cooled-down retry passed. Both results are retained (§6). |
| F4: unconditional i686 execution dependency | Shared execution capability probe; optional unavailable hosts emit `SKIP-RUN`; `LCCC_REQUIRE_I686=1` fails closed. CI sets required mode. Both new performance regressions join the i686 O1/O2/O3 leg. | A compiler accepting `-m32` is not enough: the probe must link **and execute**. Supported execution passed. A host-compiler wrapper rejecting only `-m32` produced optional exit 0/SKIP-RUN and required exit 1/FAIL. |
| F5: literal interpretation consistency | Threaded arithmetic width through `add_const`/`is_incremented`, converted popcount `with_const` matching to the effective typed ring, and hardened constant/range proof boundaries found during the wider inspection. | Carrier-zero equality is conservative: an accepted numeric zero is safe, but it misses some different carrier representations of ring-zero. It is not a universal license to use carrier magnitudes for range proofs. See §2. |
| F6: positional tuples and ambiguous polarity | Introduced `ZeroMaskArms { tested, zero_arm, nonzero_arm, ty }` and `Expansion { block, index, dest, replacement }`; renamed misleading true/false arm names. | Reverse-coordinate deferred expansion, destination checks and span replication remain intact. Boolean mux def/use accounting stays immediate. Tests/lints, not renaming alone, support this refactor. |
| F7: baseline/v3 distinction | Explicitly retained below and in this session's conclusions. | Historical portable CLZ **9→5 on baseline x86-64**, but **4→4 on x86-64-v3**. That previous merged improvement is not a new speedup earned by this follow-up. |

The review correctly identified valuable coverage and maintenance gaps. Its
absence of a found soundness defect is not a proof that all adjacent helpers
are sound. Likewise, neither a proposed “10/10” checklist nor a partially green
GitHub run is final acceptance.

### Additional gate integrity fixes

The delta updater now accumulates a complete replacement before atomic
publication: an unavailable oracle cannot leave a partially rewritten golden
file. The codegen updater likewise refuses missing sources/failed or incomplete
updates, preserves the previous baseline on failure, and publishes atomically.
Seven offline tests cover each updater. The restored delta-gate mock's module
alias error was corrected and retested; earlier failures remain in raw logs.

The first complete fast run found two integration failures (174 passed, two
failed, five skipped), retained as `ci-fast-first.log`. Neither was waived:

- The parity checker could not see a policy test nested inside salvage. The
  policy test is now a directly registered gate in **both** CI mirrors, rather
  than added to an orphan allowlist.
- The oracle partition self-test assumed exports had a literal, single-line
  source spelling. New CLZ/CTZ kernels use function-generating macros. Replaced
  the brittle regex with host object compilation and globally defined function
  symbol inspection. This handles attributes/macros and rejects static symbols,
  comments, data and declarations without definitions; a new offline fixture
  verifies those negatives. No corpus entry was skipped to obtain green CI.

## 2. Literal and proof-boundary audit

- Numeric constants must be interpreted in the **operation's effective ring**,
  not solely by the Rust enum carrier. A narrow negative literal sign-extends
  before reduction into a wider operation, whereas a positive carrier alias can
  reduce modulo a narrower operation. `const_in_ring` is used at the audited
  mask/increment sites with explicit widths.
- `high_bits_zero` had an additional range-proof hazard beyond the review's
  listed mask comparisons. Treating I8(−128) as the carrier magnitude 128 in a
  U32 AND can incorrectly prove that only 16 low bits remain. The IR witness's
  original narrow-shift expression yields **0xffff**; the tempting unsound
  rotate would yield **0x80ff**. The regression rejects that rewrite. This is an
  **IR proof-boundary witness**, not a newly demonstrated frontend-C wrong-code
  report. Frontend reachability of that representation remains unproven.
- Truncating casts are not generally increment-preserving. The increment
  matcher now remains conservative across them; normal same-width aliases and
  typed-ring increment constants continue to match.
- Historical tests using a carrier value in a 64-bit operation were corrected
  to reflect actual 64-bit semantics rather than enshrining an accidental
  32-bit truncation. Rejecting that old expectation is not a regression fix by
  definition; the typed arithmetic decides the expectation.

This is a focused audit of the touched bit-idiom/simplifier paths and evidence
tools, not an exhaustive audit of every compiler pass or backend.

## 3. Two earned generated-code optimizations

### 3.1 Even-product signed division by two (`spectral_norm`)

The simplifier proves that factors differing by an odd Add/Sub/Xor constant
have opposite parity; their product is therefore even modulo 2^W. For the
matching signed-half lowering, that fact permits dropping the truncation-bias
sequence and using an arithmetic right shift. Width, sign-shift, product
identity and final-shift checks are required. The argument also covers wrapped
MIN; it does not assume that signed source multiplication never overflows.
Odd products, unrelated roots, wrong shifts and mixed widths are negative
controls. Two Rust regressions plus a 10,000-case C differential fixture cover
the positive/negative cases, O0–O3 and 64/32-bit execution. Memcheck reported
zero errors and zero leaks for the dedicated fixture.

- `half32` and `half64`: **10→5 instructions** locally.
- `odd32`: 10→10; unrelated-product control: 9→9; `xor64`: 11→6.
- CE `half32`: LCCC **5**, GCC 16.2 **7**, Clang 23.1.0 **7**, ICC 2021.10.0
  **8**, ICX `cicxlatest` **7**. ICX's rolling identifier is not an invented
  numbered release. Static instruction counts are not execution times.
- Direct candidate/old-LCCC Callgrind, N=128:
  **8,673,549 / 11,302,666 = 0.7673896583**, **23.26% fewer simulated Ir**.
  I1 misses 1424/1424, D1 read+write misses 1932/1932, LL misses 2969/2969;
  conditional mispredicts **8570/8555 worsened**, indirect 233/233.
- Separate pinned native confirmation, full default size, 11 alternating
  pairs and two warmups: old median **269.304 ms**, candidate **216.031 ms**,
  **19.78% lower elapsed time**. All eleven pairs favored the candidate.

### 3.2 Same-predicate nested Select folding (`aarch64_select_patterns`)

The generic bit-idiom pass collapses nested Select arms that repeat the exact
same predicate, including both arm polarities. It requires matching integer
result types and follows Copy aliases only for the condition: a truncating
predicate cast is **not** equivalent (the C regression includes 256).

This rewrite updates `defs` and `use_counts` **immediately**, before Boolean
mux producer repurposing; deferring the bookkeeping would leave the subsequent
matcher inspecting stale IR. Tests check polarity, type mismatches, source
spans, idempotence and the truncating-condition negative. It is not a special
case keyed to the benchmark name or to AArch64.

- x86-64 `select_pressure`: **12→7 instructions**; GCC/Clang/ICX 6, ICC 7.
- Direct Callgrind, SELECT_ITERS=20000:
  **866,018 / 966,018 = 0.8964822602**, **10.35% fewer simulated Ir**.
  I1 misses 1356/1357; D1 1874/1876; LL 2893/2896; conditional mispredicts
  3322/3325; indirect 231/231.
- Pinned native confirmation, default size, 11 alternating pairs/two warmups:
  old median **135.35 ms**, candidate **123.23 ms**, approximately
  **8.95% lower elapsed time**. Ten pairs favored the candidate, one did not.

Both native comparisons used the same `-O2 -march=x86-64-v3 -fwrapv`, the same
pinned CPU, matching outputs, and no concurrent build or profiler. Ratios above
compare with the **old LCCC**, not GCC. The harness's printed sign-test values
use a normal approximation; the committed summary computes exact two-sided
Binomial(n,½) tails from saved pairs: 0.0009765625 and 0.01171875 respectively.
These are two targeted comparisons, not a corpus-wide performance guarantee.

## 4. What the actual native worst15 say

Full-size 43-workload corpus, candidate LCCC versus **local GCC 14.2.0**, common
`-O2 -march=x86-64-v3 -fwrapv`, three paired rounds/one warmup, all outputs
correct. **This is a low-power screen, not reliable rank separation.** Several
programs run below 20 ms. Ratios are the median of paired ratios, which need not
equal the quotient of separately reported medians.

| Rank | Workload | LCCC/GCC runtime | Inspection / remaining work |
|---:|---|---:|---|
| 1 | nbody | 1.721 | Actual hot pair loop contains stack homes at −0xa0/−0xa8 and extra FP register moves around arithmetic/FMA. Existing extract-home experiments are not a justification to flip a global knob; allocate/rematerialize under a measured cost model. |
| 2 | expat_siphash24 | 1.585 | Hot `sip_round` plus call boundaries and register traffic dominate the bounded profile. The local assembly retains calls from update/final paths; study selective inlining and live-range pressure rather than unlimited inlining. |
| 3 | expat_xml_scan | 1.307 | Hot document scanner's classification/control-flow chain; range/bitset dispatch and block layout remain candidates, not measured fixes. |
| 4 | matmul | 1.235 | Vectorized loop quality, address generation and pressure need equal-work comparison; native medians only about 6.7/5.4 ms make this a particularly weak timing verdict. |
| 5 | mandelbrot | 1.203 | Divergent FP iteration/control-flow and register transport; establish the exact hot loop before changing scheduling or vectorization. |
| 6 | hash_table | 1.202 | Probe-chain/address-generation and memory behavior; this was the costly full-size case (~13.6/11.1 s). It was measured, not silently replaced by a tiny benchmark. |
| 7 | zstd_count | 1.126 | TZCNT is already present in actual candidate assembly; this is not a missing-CTZ fix. Compare loop/tail/pointer handling. Final matched-flags CE coverage completed after a rate-limit cooldown. |
| 8 | spectral_norm | 1.119 | Exact-half improvement landed and is independently confirmed above, but the candidate still trails GCC in this screen. Do not call the gap closed. |
| 9 | sqlite_varint | 1.113 | Small-value/long encoding paths, redundant mask materialization and control flow remain; stack arrays are not automatically spills. |
| 10 | tls_seg_access | 1.107 | Direct FS-relative accesses are already present. Examine TLS-base reuse and loop address generation; do not “fix” this by illegally hoisting observable TLS stores. |
| 11 | strlen_bench | 1.106 | Width/tail/load/branch choices and memory-level effects; a bounded instruction win would not establish native throughput. |
| 12 | glibc_memcmp | 1.101 | Equality fast paths and word/byte mismatch handling. Some stack stores materialize source-level word bytes; blindly deleting all stack traffic is unsound. |
| 13 | switch_dispatch | 1.094 | Dispatch lowering and branch behavior; wrap semantics materially affect the comparison and must remain common. |
| 14 | sha256_transform | 1.091 | Rotate generation alone is not the problem; inspect schedule/live ranges and message-array traffic. The stack-resident message schedule is real program data, not all allocator spills. |
| 15 | fannkuch | 1.085 | Permutation/reversal loops and indexed loads; trip-count-aware unrolling is a hypothesis requiring new tests and measurement. |

Only `spectral_norm` among this **native** worst15 changes generated code in
this patch. The other fourteen are unresolved, not asserted fixed. The select
workload was a **bounded Callgrind** worst15 case, not a native worst15 case.

The original report's aggregate geomean 0.7748 is dominated by recursion/constant
propagation wins (`fib` ~0.012, `ackermann`/~constant-recursion ~0.015). It does
not erase the 21 raw policy-classified apparent losses. Those classifications
were overconfident with n=3: `bench_rank.py` now preserves the raw interval but
withholds a significance verdict below seven pairs (or an unknown sample
count), places such verdicts in **unknown**, and displays quality warnings in
text and Markdown. Seven samples is only a minimum policy, not a guarantee of
sufficient statistical power.

## 5. Bounded Callgrind worst15 and rejected evidence

The initial exploratory sweep applied `-fwrapv` only to `arith_loop`. Inspection
found overflowing signed arithmetic in `loop_patterns` too. **Those corpus
rankings were superseded**, not mixed with the final data. Both complete
43-workload arms were rerun with common wrapping semantics. Size macros live in
`tests/benchmark/callgrind_sizes.json`; defaults of the native corpus remain
unchanged. Alias workloads sharing a source still run their own macro sets.

Final bounded corpus: both phases **43/43 comparable, zero failures**. At each
phase, native and instrumented exit/stdout agree with GCC. Exactly **41/43
candidate binaries are byte-identical to old LCCC**. Tiny 14/24-Ir changes in
some identical programs are startup/environment effects, not optimizations.

| Old bounded rank | Workload | Old LCCC/GCC Ir | New LCCC/GCC Ir |
|---:|---|---:|---:|
| 1 | ring_fifo | 2.4978 | 2.4978 |
| 2 | histogram | 2.0508 | 2.0508 |
| 3 | ascii_case_fold | 1.8335 | 1.8335 |
| 4 | glibc_strstr | 1.7725 | 1.7725 |
| 5 | expat_siphash24 | 1.6503 | 1.6503 |
| 6 | spectral_norm | 1.4005 | **1.0747** |
| 7 | binary_search | 1.4003 | 1.4003 |
| 8 | aarch64_select_patterns | 1.3544 | **1.2142** |
| 9 | nbody | 1.2853 | 1.2853 |
| 10 | lz4_match_extend | 1.2551 | 1.2551 |
| 11 | linux_rbtree | 1.2522 | 1.2522 |
| 12 | expat_xml_scan | 1.1808 | 1.1808 |
| 13 | switch_dispatch | 1.1595 | 1.1595 |
| 14 | loop_patterns | 1.1552 | 1.1552 |
| 15 | double_reduction | 1.1258 | 1.1258 |

The two corpus A/B improvements are 11,302,656→8,673,539 Ir and
966,064→866,064 Ir. Small differences from the dedicated measurements above
come from command/path/startup context. Do not splice counters from different
runs into a more flattering ratio.

Exclusive hot-function annotation for the original screen localized FIFO push/
pop, histogram accumulation, ASCII folding, string search, SipHash rounds and
FP kernels. It is retained as a diagnostic; it is not treated as the final
wrapping-semantics ranking. Remaining high priorities include:

1. FIFO mask/modulo/address and call overhead; histogram dependence-aware
   vectorization/private accumulation; ASCII range predicates and loop form.
2. String/match search tail and mismatch control flow; red-black tree pointer
   chasing/branch layout; search initialisation versus lookup phase separation.
3. SipHash inlining/register pressure and nbody FP homes/transport, followed by
   the native worst15's large-throughput cases before tiny sub-20-ms screens.

### Explicitly rejected metric

Automatic hot-loop selection by largest induction stride and matching function
name does **not** prove equal semantic work. In multi-phase `main`, it compared
unrelated phases, producing spectacular ~197.82× binary-search and ~160× Expat
“density” ratios. Those ratios and the ~5.1642 geomean are **not accepted
performance evidence**. The tool now warns in text/Markdown and emits
`semantic_loop_equivalence_verified: false` in JSON. Nine heuristic unit tests
pass, but parser tests do not validate semantic loop alignment. A trusted next
comparison needs explicit region/work-unit mapping or matched disassembly
traces; no static-count slogan substitutes for that.

## 6. Validation, provenance, and actual limitations

### Executed validation before the final publication gate

- Rust fastbuild O1, `-j2`, all targets: library **4299 passed, 0 failed,
  7 ignored**; remaining target tests passed. `cargo fmt --all -- --check` and
  `cargo clippy --profile fastbuild --all-targets --locked -j2 -- -D warnings`
  passed. The ignored cases are not silently counted as passes.
- Required-i686 salvage passed on the new compiler before the final gate;
  optional unavailable and required-failure policies were separately exercised.
  The final gate also includes the expanded four-fixture i686 O1/O2/O3 leg.
- Offline Callgrind contracts/wrapper **36 tests**, loop heuristic **9**, ranking
  quality **1**, golden updater **7**, delta updater **7**, and oracle offline self-tests **27** passed.
- Refreshed golden gate: **11/11 PASS**, including both new count workloads.
- Memcheck dedicated even-product regression: zero errors/leaks. This is not a
  whole-corpus Memcheck run or a substitute for semantic differential testing.
- Full-size native **43/43 correct**; bounded old/new Callgrind **43+43 correct**;
  targeted native confirmations **11+11 pairs**, all matching outputs.

### Remote oracle status: do not hide the outage

The initial 41-unique-source × four-vendor CE survey completed all 164
comparisons with no errors at `-O2 -march=x86-64-v3`. The final common-`-fwrapv`
survey completed 157/164; one retry recovered one additional ICX result.
Six comparisons were still HTTP-429 rate-limited after that immediate retry,
three each for `zlib_ng_adler32_combine` and `zstd_count`. After the first fast
CI run provided a substantial cooldown, one further bounded retry recovered
**all six**: final matched-flags coverage is now **164/164**, all four vendors
for every source. Reports from each attempt remain separate. The earlier
non-fwrapv results are not relabelled.

The extra **strict** live delta revalidation initially failed on rate limiting.
A later cooled-down strict retry **passed with exit 0**, all nine entries,
no errors/failures. Both the failed `oracle-final` and passing
`oracle-recovered` artifacts are retained. The outage is neither hidden nor
misrepresented as a compiler/codegen failure.

Remote GCC 16.2 is a **codegen oracle**; local timed GCC is **14.2.0**. Local GNU
binutils/linker and build tool versions are in the environment logs. The fast gate itself provisioned and exercised GNU assembler/objdump
**2.47.20260726** as separate assembly oracles. That is not the toolchain used
for the timed C binaries. No local GCC16.2-plus-binutils2.47 timed-toolchain
build, new mold build, PMU measurement, whole application
benchmark or complete archpkgbuilds-scale audit is claimed in this follow-up.
Compiler Explorer executions, when used elsewhere, are not timing oracles.

### Environment and reproduction

2-vCPU constrained VM, about 1.9 GiB RAM, **6 GiB active swap**. Repo/toolchain
and build intermediates use the filesystem with the most free space. Compiler
builds use fastbuild O1 and two jobs. Valgrind **3.24.0**, pinned simulated
I1=32768,8,64 / D1=32768,8,64 / LL=33554432,16,64. Ir is simulated retired
instruction work, **not PMU instructions/uops/cycles or native speedup**. Cache
and predictor counts are simulated and may disagree with native rankings.

Committed machine-readable distribution/provenance:
`engineering/evidence/pr793-followup-2026-10-11.json` (both compiler hashes,
all 43 source/binary identities and bounded results, native screen, paired raw
timings, controls, vendor metadata and limitations).

External persisted evidence is under `artifacts/pr-followup/`: native corpus,
ranked JSON, native A/B JSON, final/earlier oracle reports, final gate/logs and
compressed raw profiles/assemblies/binaries. `intermediate-logs.tar.gz` retains
superseded, failed and interrupted logs; absence of a loose log does not mean
that history was deleted. The final snapshot ledger/manifest is the authority
for the **exact-tree** local-fast gate, not this document's prose.

```sh
# Native screen; for stronger conclusions increase rounds and duration.
python3 tests/benchmark/run_benchmarks.py --compilers lccc,gcc \
  --lccc target/fastbuild/lccc --reps 3 --warmup 1 \
  --cflag=-march=x86-64-v3 --cflag=-fwrapv --no-pmu-probe \
  --artifact-dir /large/bench --json /large/bench.json --strict
python3 scripts/bench_rank.py /large/bench.json --worst 15 --best 3

# Each arm against the same local GCC and checked-in bounded sizes.
python3 scripts/callgrind_corpus.py --mine /large/old-lccc --ref gcc --out /large/cg-before
python3 scripts/callgrind_corpus.py --mine target/fastbuild/lccc --ref gcc --out /large/cg-after

# Final local acceptance, never the slow/full gate in this session.
LCCC_REQUIRE_I686=1 bash scripts/ci_local.sh --fast
```

## 7. Follow-up acceptance criteria, not promises of completed work

1. Retain fail-closed baseline updates and explicit rate-limit status. Future
   remote revalidation should use bounded retries/cooldowns, not soften strict
   failures or relabel old flags. This session's delayed revalidation completed.
2. For each remaining native loser, isolate equal-work hot regions and add
   correctness/negative tests **before** an optimization. Start with nbody,
   SipHash, XML scanning and hash probes; require paired native confirmation
   and Callgrind attribution, then inspect the full regression distribution.
3. Replace heuristic cross-compiler loop pairing with explicitly verified work
   units; preserve a fail/unknown state when equivalence is unavailable.
4. Extend confidence beyond one VM, tiny screens and extracted kernels to
   actual gzip/zlib-ng/Expat/SQLite/Linux/glibc workloads. The present kernel
   corpus is useful but not evidence that all those full applications are fast.

**Conclusion:** F1–F7 received concrete code/tests/gate/documentation work; two
new compiler optimizations have independent semantic and measured support.
There is no newly observed regression in the exercised corpus, but fourteen
native worst15 gaps are unchanged, spectral still trails GCC there, remote
failures required a documented retry, and compiler-wide perfection is
neither established nor asserted.
