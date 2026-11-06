# Backlog ROI sweep — 20 measured items (2026-11-05)

Base: `3596c7bf4d39c8fb33befcbf0a07be6f668cf7c3` (`main`, post-merge #1109).
Method doctrine: [`engineering/agent/RULES.md`](../engineering/agent/RULES.md).
Executable half: [`scripts/check_backlog_roi.py`](../scripts/check_backlog_roi.py)
(20 invariants, 20 negative controls, `--self-test`, `--json`).
Regression half: [`tests/corpus/test_backlog_roi.py`](../tests/corpus/test_backlog_roi.py).

## Why these 20

The backlog is 3,045 lines in `backlog.md` plus a separate ranked catalog in
`engineering/agent/BACKLOG.md`.  A textual "is it closed?" reading is exactly
the guesswork the method doctrine forbids, so every candidate was **probed**
against the rebuilt compiler and the live tree before admission:

| Measurement | Result |
|---|---|
| `cargo build --profile fastbuild --locked`, `RUSTFLAGS=-D warnings` | exit 0, 4m20s, zero warnings |
| Toolchains restored from `harness-cache-8` | GCC 16.2.0, Clang 23.1.5, GDB 18.1, Rust 1.99.1, mold 3.0.0, QEMU 11.1.3, binutils 2.47 |
| GP-22 ledger rows executed against the binary (`-S` SHA-256 + `-dM -E` bytes) | 17 rows, **5 false classifications**, 17 dead `probe_id`s |
| GCC 16.2 / Clang 23.1.5 `-dM -E`, 4 dialects | 30 lccc-only, 13 GCC-only, 28 value-spelling differences (all numeric synonyms) |
| `tests/regression/sse2_codegen_matrix.json` | 122 rows, provenance still named Clang 23.1.4 |
| `gcc-16 -m32` before/after the pinned bookworm multilib bundle | FAIL (`bits/libc-header-start.h`) → PASS (4-byte pointer) |
| Full corpus (at sweep start, before any change) | 1268 tests, 0 failures, 6 skips |

Selection rule: **ROI = provable defect × cost of the gate that prevents its
return ÷ implementation risk.**  Items whose premise was already false on this
tree were demoted to *ratchet* (keep them closed) rather than reimplemented;
items whose measurement was stale were re-derived.  No item is marked closed
without a test that fails when it regresses.

## The 20 items

| # | ID | Backlog source | Measured state before | Decision & change | Regression |
|---|---|---|---|---|---|
| 1 | GP-22/1 | `backlog.md` §12 | 17 ledger rows, 5 contradicted the binary | Reclassified 3 `Inert`→`Refused` and 2 `Inert`→`Implemented` by measurement; tracing `-Ofast` further exposed a macro-only implementation (finding 10), `-O`-level fast-math ownership (11) and a silent unoptimized-code cliff at numeric `-O<n>` (12) | `option_effect_ledger.py`, 3 Rust unit tests, non-vacuous composition gate, `test_ofast_composition.py` (9 tests), `test_numeric_o_levels.py` (8 tests), GDB runtime suite |
| 2 | GP-22/2 | §12 | 17 `probe_id`s, **zero consumers** | New probe registry; every accepted row must resolve to an implemented probe | `static_problems` + 16 tests |
| 3 | GP-22/3 | §12 | registry could gain orphans silently | Exact set equality between ledger probes and registry; duplicates rejected | check 3 + negative control |
| 4 | GP-22/4 | §12 | priority set asserted only in prose | 7 priority spellings must all be ledgered | check 4 + Rust unit test |
| 5 | GP-22/5 | §12 | no gate executed the ledger | `check_driver_option_contract.sh` now runs `option_effect_ledger.py --lccc "$CCC"` in the existing fast lane | check 5 + gate run (exit 0) |
| 6 | GP-22/6 | §12 | local/hosted parity unaudited for this gate | Both `ci_local.sh` and `ci.yml` must name the driver gate | check 6 |
| 7 | GP-23/1 | §12 | macro-parity gate already existed and is wired | **Ratchet, not reimplementation**: pin the three budget tables and both wirings | check 7 |
| 8 | GP-23/2 | §12 | `BUFSIZ` positive control could be deleted | Pin the `FORBIDDEN_LIST` and its `BUFSIZ` sentinel | check 8 |
| 9 | GP-05 | §12 | historical drift `__GNUC__ 14`; live value is 16 | Pin `__GNUC__` to the GCC 16 major in `predefined_macros.rs` | check 9 |
| 10 | MB-01/1 | `backlog.md` §4 | `-fno-plt` described as a silent no-op; it emits GOTPCREL | **Closed upstream**; ratchet the effect probe so the stale claim cannot return | check 10 |
| 11 | MB-01/2 | §4 | same for `-fvisibility=hidden` | Ratchet the `.hidden` effect probe | check 11 |
| 12 | MB-40 | §9 | blanket `arg if arg.starts_with("-f") => {}` made refusals unreachable | Ratchet: strip comments, then reject that arm; also require `lookup` consultation | check 12 + ledger static |
| 13 | MB-41/1 | §9 | backlog headings had no parseable-ID invariant | Parse `### ID · class · evidence` headings; ≥20, well-formed ids | check 13 |
| 14 | MB-41/2 | §9 | `R` items with no reproducer invisible | Every `R` item must carry a fenced reproducer or an inline command | check 14 |
| 15 | MB-42 | §9 | capability probe's evidence basis could rot | Pin `NEGATIVE_CONTROLS` and the exit-status record | check 15 |
| 16 | MB-45 | §9 | "gap" counts could lose their oracle | Pin the named GCC/Clang comparison oracle in the performance catalog | check 16 |
| 17 | FOLLOWUP-7/2 | §7 item 2 | SIMD matrix provenance said "Measured with Clang 23.1.4" | **Re-derived** all 122 rows against Clang 23.1.5 / GCC 16.2.0 (`--update-baseline`); `--check` and `--self-test` PASS | `test_oracle_provenance.py` (6 tests) |
| 18 | FOLLOWUP-7/2b | §7 item 2 | `--update-baseline` **dropped the note** on regeneration | Generator now derives `note` + `rederived_with` from the banner that measured the data | 5 negative controls |
| 19 | FOLLOWUP-7/7 | §7 item 7 | CE serves 23.1.0 while the pin is 23.1.5 | **Closed upstream**; ratchet the alias-table provenance instruction | check 19 |
| 20 | CACHE-REDTEAM/1 | post-merge #1109 audit | stale cache tag before #1109: 5 obsolete blobs, no GDB | Ratchet the reviewed lock (tag 8, GDB entry, zero obsolete filenames) and the publisher's content guard | check 20 + `test_agent_cache_publisher.py` |

## Red-team findings on the merged patch (#1109)

1. **The ledger was decorative.**  `probe_id`s existed only as strings in
   `option_table.rs`; no gate consumed them.  Five rows were factually wrong.
   A trust contract that nobody executes is worse than no contract, because it
   invites a reviewer to believe it.  Fixed by item 1/2/5.
2. **`--update-baseline` destroyed provenance.**  It rebuilt `provenance`
   from scratch with only `recorded_with`, so the human-readable note naming
   the measuring oracle vanished on the very command a future pin bump would
   run.  That is a self-invalidating ratchet.  Fixed by item 18.
3. **The backlog overstates open work.**  MB-01's `-fno-plt` no-op,
   GP-23's missing gate, §7-7's CE provenance and GP-22's coverage rows are
   all closed on this tree.  Reimplementing them would be duplicated work;
   the honest action is a ratchet plus a measured triage, which is what the
   table above does.
4. **Local `-m32` capability is a provisioning gap, not a compiler defect.**
   `gcc-16 -m32` failed with `bits/libc-header-start.h: No such file` because
   the restored cache leaves the bookworm multilib bundle as `.deb` files.
   `sudo dpkg --unpack` on the 20 pinned packages makes it exit 0 with a
   4-byte pointer, unblocking the `zlib-compare256-correctness` gate and the
   195 previously-unmeasured i686 aggregate-ABI configurations.  This is
   recorded as an environment action, not claimed as a code fix.
5. **A second silent-ratchet defect in the DWARF DIE census.**  GDB 18.1
   showed LCCC reporting every parameter as `<optimized out>` at every
   optimisation level, while GCC 16.2 and Clang 23.1.5 resolve `a=5, b=7`.
   Tracing that into `readelf --debug-dump=info` exposed a worse problem than
   the gap itself: `check_dwarf_die_census.py` pinned only **3 of the 12 tags**
   LCCC actually emits and pre-declared **15 tags and 5 attributes** as
   "deferred" that the compiler already implements. Both halves of the gate
   ignored those names, so losing the entire C type graph, every variable DIE
   or every `DW_TAG_formal_parameter` would have produced a green check.
   Measured union (across `-gdwarf-2..5`, readelf 2.47): LCCC 12 tags /
   20 attributes, GCC 14 / 27, Clang 14 / 21; real gap = 3 tags
   (`enumeration_type`, `enumerator`, `lexical_block`) and 10 attributes.
   Fixed by pinning the full measured baseline, reducing the deferral list to
   the measured difference, adding an import-time disjointness assertion, and
   adding 7 corpus tests (including an end-to-end negative control that runs a
   gate copy with `DW_TAG_variable` removed and requires a non-zero exit).
6. **The gap itself is honest, so it is documented rather than faked.**
   Emitting a bare `DW_OP_regN` as a parameter location would claim the value
   stays in that register for the whole function, which is false as soon as
   the register allocator reuses it. GCC/Clang use entry-value expressions and
   location lists. A wrong location is worse than `<optimized out>`. The
   `parameter_location_is_never_wrong` test is a forward ratchet: LCCC may
   omit locations (measured, documented) or emit one GDB resolves to the true
   value; any half-correct implementation fails. The census docstring now
   records the GDB observation explicitly.

   The deferral was re-measured rather than assumed on the final tree:
   `lccc -g -O0 -S` emits 18 `.cfi_*` directives, the object carries
   `.eh_frame` with relocations, and the subprogram already publishes
   `DW_AT_frame_base = DW_OP_call_frame_cfa`, so a `DW_OP_fbreg` location is
   *almost* expressible. At `-O0` the first two parameters live at
   `-8(%rbp)` and `-16(%rbp)`; with CFA = `%rbp + 16` those are
   `DW_OP_fbreg(-24)` and `DW_OP_fbreg(-32)`. But those locations are only
   valid **after** the prologue stores them: a single static expression would
   make GDB read garbage when the user stops at `*func` before the store.
   A correct fix needs a location list split at the existing
   `.loc ... prologue_end` marker (register at entry, frame slot afterwards),
   and `writer_base.rs` has no `.debug_loc` / `.debug_loclists` section writer
   -- its `debug_loc()` is the `.loc` *line-table* directive emitter. Adding
   that infrastructure (plus its relocations, DWARF 2/3/4 vs 5 forms, four
   target ABIs and a census-baseline update) is a separate feature. Shipping a
   plausible-looking `DW_OP_regN` here would trade an honest
   `<optimized out>` for a silently wrong value, which is strictly worse for
   runtime debugging. The forward ratchet keeps this visible and testable.
7. **The deferral list was still only statically checked.**  An import-time
   assertion compares two hard-coded tables; a new LCCC capability added
   without updating `BASELINE_TAGS` would still be masked by an old deferral.
   The gate now also requires the *measured* `ours_tags`/`ours_attributes` to
   be disjoint from the deferral lists, and a negative control removes a
   baseline entry, re-adds the stale deferral and requires the runtime
   diagnostic. Both layers are now ratcheted.
8. **The executable ledger only ever ran against one architecture.**  The
   driver option table is architecture-independent, but a target-specific
   `cli.rs` arm could diverge while the fast lane stayed green. All four
   fastbuild drivers were measured: `lccc`, `lccc-i686`, `lccc-arm` and
   `lccc-riscv` each report 17 rows and zero problems. The same probes now
   exercise four back ends in a pinned corpus sweep, so a fifth target cannot
   be added without deliberately extending the coverage.
9. **A line-by-line audit of the new gate found three edge-case defects in
   the gate itself.**  (a) A wrong `--lccc` path raised an uncaught
   `FileNotFoundError`, so CI saw a Python traceback instead of a named
   verdict. (b) `opt.s` was reused for all 17 rows; a compile that failed
   without writing it left the previous row's bytes, which were then hashed
   and could be reported as a codegen change. (c) A relative `--lccc` path
   only worked from the repository root. Fixed with `resolve_binary` (named
   problems, `PATH` lookup, repo-root fallback), `dest.unlink(missing_ok=True)`
   before every compile, and exact-duplicate suppression in the problem list.
   Six new `GateRobustness` test methods cover all three plus a
   non-executable file and both halves of the stale-output contract (the
   failed-compile hash and a positive control proving a fresh compile is
   still hashed). An earlier description of this delta said "seven tests";
   there are six methods exercising seven distinct behaviours — the count
   here is the measured `unittest` result (18 to 24).
10. **`-Ofast` was a macro-only lie: a real missed optimization and GCC
    incompatibility.**  Tracing the reclassified rows into `cli.rs` showed
    `-ffast-math` sets six driver fields (`fp_trapping_math`,
    `fp_rounding_math`, `fast_math`, `fp_reassoc`, `math_errno`,
    `fp_contract`) while `-Ofast` set only `fast_math` — enough to define
    `__FAST_MATH__`, not enough to enable the fast-math pipeline. Measured on
    a packed FP reduction (x86-64): LCCC `-O3` and `-Ofast` were both an
    11-instruction scalar loop, while `-O3 -ffast-math` emitted a 63-instruction
    vectorized loop with 8 packed adds. GCC 16.2.0 and Clang 23.1.5 both emit
    byte-identical assembly for `-Ofast` and `-O3 -ffast-math`, distinct from
    `-O3`. The pre-existing GP-22 composition gate asserted that equality but
    on a scalar probe with no reduction, where all three variants are identical
    — a vacuous check that stayed green for the whole defect.
    Fix: one `Driver::set_fast_math(enabled)` helper used by `-ffast-math`,
    `-fno-fast-math` and `-Ofast`, so the two spellings cannot drift again.
    A Rust unit test pins all six fields, `-O3 -ffast-math` equivalence,
    `-fno-fast-math` last-wins, and explicit `-ffp-contract` precedence in both
    orders. The regression gate now checks a reduction fixture with explicit
    non-vacuity controls for LCCC and GCC; a corpus module checks LCCC, GCC and
    Clang plus a wrapper compiler that reproduces the old behaviour (which the
    check rejects with two distinct diagnostics). This is the only change in the
    PR that alters compiler output, and it removes a genuine "cycles left
    behind" defect rather than adding an unmeasured optimization.
11. **`-Ofast`'s fast-math is a property of the `-O` spelling, not of the
    flag list.**  Tracing the fix in finding 10 into the option-order space
    exposed a second GCC/Clang incompatibility: both oracles withdraw
    `-Ofast`'s implicit fast-math when a later plain `-O*` option appears,
    but keep an *explicit* `-ffast-math` sticky. On the packed reduction,
    LCCC used to emit the vectorized loop for all of `-Ofast -O0`,
    `-Ofast -O1`, `-Ofast -O2`, `-Ofast -O3`, `-Ofast -Os`, `-Ofast -Oz`
    and `-Ofast -Og`. Measured 20-vector matrix (macros and assembly hashes
    on LCCC / GCC 16.2.0 / Clang 23.1.5) is encoded in
    `tests/corpus/test_ofast_composition.py` and in the CI-executed GP-22
    gate. Design: one `Option<bool>` origin field (`explicit_fast_math`)
    plus one `withdraw_implicit_fast_math()` helper; no per-arm bookkeeping.
    The two oracles disagree on exactly one vector, `-fno-fast-math -Ofast`
    (GCC off, Clang on). LCCC follows GCC, because an explicit user opt-out
    of unsafe FP math must never be silently revoked; the divergence is
    pinned with measured values rather than hidden. 17 matrix vectors are
    executed per CI run with macro state, codegen equality and GCC oracle
    checks, plus an order-blind wrapper negative control.
12. **Numeric `-O<n>` silently produced unoptimized code.**  GCC 16.2 and
    Clang 23.1 clamp `-O4`..`-O2147483647` to their own `-O3`. In LCCC those
    spellings fell through to the generic tolerated-unknown handler and
    emitted the *unoptimized default* with only a warning — measured on the
    reduction: `8ffb18df` at `-O4` versus `189a4c8373a1` at `-O3`. Real
    build systems still pass `-O4`, so this was a live "cycles left behind"
    defect on the host CPU, and `-Ofast -O4` also leaked fast-math. Fix: a
    numeric `-O` arm that parses ASCII digits with `u64::from_str` (checked
    multiply/add, so overflow returns a named error rather than UB), rejects
    values above `i32::MAX` instead of reproducing GCC's `2^32` wraparound
    to `-O0`, and clamps to level 3. All eight `-O` spellings now share one
    `set_optimization_level(level, size, debug)` helper — 48 duplicated
    assignments became 8 one-line calls, and the `-Ofast` withdrawal lives
    in exactly one place. A dedicated 8-test corpus module covers byte
    equality with `-O3`, `__OPTIMIZE__`, zero diagnostics, GCC/Clang oracle
    clamping, 8 malformed/out-of-range refusals with pinned oracle
    divergences, strict-options behaviour, `-Ofast -O4` withdrawal, a
    source-level pin and a wrapper that reproduces the old cliff. The pinned
    GDB 18.1 runtime suite now also executes `-O4`, comparing host-CPU
    results and unwind chains against GCC and Clang.
13. **A first host-CPU benchmark was itself wrong, and the correction is
    measured.**  A dot-product timing harness put `volatile double sink`
    *inside* the outer repetition loop. LCCC's reduction legality check
    rejects a loop body containing a volatile access, so the inner dot
    product stayed scalar; GCC/Clang vectorized it and appeared 2x/3.6x
    faster. Reporting that as a missed optimization would have been
    guesswork about a harness artifact. Red-teaming the harness moved the
    volatile store after the loop and added a data-dependent mutation that
    prevents hoisting. All three compilers then pack the dot product, and
    on this 2-vCPU x86-64 host (9 interleaved runs of 40 000 dots of 2048
    doubles, minimum seconds): LCCC `-O3` 0.0993, LCCC `-O4` 0.0991,
    LCCC `-Ofast` 0.0147, GCC 16.2.0 `-Ofast` 0.0496, Clang 23.1.5
    `-Ofast` 0.0263. LCCC's `-Ofast` is 3.4x faster than GCC and 1.8x
    faster than Clang **on this one workload on this one CPU**; that is a
    measurement, not a general throughput claim. All five checksums agree
    with the strict `-O0` reference to one ulp (relative 1.7e-16). The
    six-test corpus module pins codegen shape, byte identity, checksums,
    a loose interleaved-minimum ratio gate with a >4x margin, the
    three-way measurement and a macro-only wrapper negative control.
    ICX 2026.2 remains unavailable locally; hosted CI's authenticated lane
    is authoritative.
14. **A pre-existing runner race was exposed by the loaded host.**  The
    full corpus, run concurrently with strict Clippy, the Rust suite and the
    local CI contract, failed
    `test_worker_exception_publishes_fatal_and_stops_the_queue`: it expected
    one `compile_case` call after a fatal runner fault but observed five.
    Root cause proven deterministically by delaying
    `concurrent.futures.as_completed` by 50 ms: `scripts/x86_gcc_torture.py`
    submits every case up front and cancels the remaining futures from the
    *consumer* thread. With `--jobs=1` the single worker dequeues and
    finishes all queued (instant, mocked) cases before the consumer wakes
    up, so `future.cancel()` is a no-op and the "stop the queue" guarantee
    is false under load. Fix: a `threading.Event` set by the worker in the
    same thread that raised, before it can dequeue another case, plus a
    pre-invocation check that returns a distinct sentinel (never conflated
    with a real `Result`). Two new regression tests pin the delayed-consumer
    race at `--jobs=1` (exactly one call) and bound in-flight work at
    `--jobs=4` (< =4 calls); all 37 tests in the module pass. A second
    failure in the same run was our own redundant wall-clock assertion
    comparing two byte-identical binaries (-O4 vs -O3): it differed by 36%
    under load for identical code. Removed; structural byte identity is the
    proof, and the remaining timing gate (LCCC -Ofast >= 1.5x faster than
    -O3, observed 6.8x) passed with a >4x margin.
15. **A line-by-line audit of `set_fast_math` found a pre-existing fenv state
    leak.**  The helper's `enabled` branch clears `fp_trapping_math` and
    `fp_rounding_math`, but the withdrawal branch never restored them: an
    explicit `-ftrapping-math` followed by `-ffast-math -fno-fast-math`
    silently disappeared. This is observable, not theoretical: a `_Float16`
    expression changes emitted assembly when either fenv bit gates half-float
    folding (`ir/lowering/half_float.rs`), and the digest reverted to plain
    `-O2` after the withdrawal. GCC 16.2 reports `-ftrapping-math [enabled]`
    after the same sequence (`-Q --help=optimizers`), and preserves explicit
    `-fno-trapping-math` across `-Ofast -O2`. Fix: two `Option<bool>` explicit
    origin fields (the same design already used for `fp_contract_explicit`
    and `explicit_fast_math`), recorded by the four `-f{,no-}{trapping,
    rounding}-math` arms and restored on withdrawal; `None` falls back to
    LCCC's documented fenv-off default. Last-explicit-option precedence is
    preserved. One Rust unit test sweeps both bits, both withdrawal paths,
    explicit on/off and the default; four corpus tests pin the non-vacuous
    `_Float16` probe, the restore, the `-Ofast -O2` restore and the GCC oracle.
    Neither of the two pre-existing 17-vector order matrices changes, because
    none of their vectors sets an explicit fenv flag.
16. **The third comparison oracle is now measured, not deferred.**  ICX 2026.2
    is not installable in this sandbox (Intel PPA blocked), so
    `tests/regression/check_icx_o_level_parity.py` was added and wired into the
    authenticated `icx-oracle` lane with `--require-icx` plus a local
    `ci_local.sh` gate. It measures the numeric-`-O` / `-Ofast` decisions on
    all four compilers (LCCC, GCC 16.2.0, Clang 23.1.5, ICX 2026.2), prints a
    digest/macro/exit-status divergence table, and asserts only the universal
    invariants: numeric levels >=3 clamp to that compiler's own `-O3`, a later
    plain `-O*` withdraws `-Ofast`'s implicit fast-math, and the probe is
    non-vacuous. Parser-limit spellings near `INT_MAX` are reported, not
    asserted, because LLVM versions legitimately differ on them. The first
    version of this gate was **wrong** and the measurement caught it: it
    compared `-Ofast -O2` against `-O3`, but GCC 16.2's `-O2` and `-O3`
    assembly differ, so the gate reported a false GCC failure. The oracle was
    corrected to compare each withdrawal against its own final `-O` spelling;
    the gate is now 3-compiler PASS locally and runs with all four in hosted
    CI. Two more corpus tests close the last parser-path TODOs: numeric levels
    inside `@response` files (including a malformed one that must be a hard
    error) and repeated mixed `-O` spellings with sticky explicit
    `-ffast-math` / `-fno-fast-math`.
17. **The third oracle disagreed, and that settled a design question.**  The
    authenticated ICX 2026.2 lane ran the new four-compiler parity gate and
    failed with real data: `icx: probe is vacuous, -Ofast == -O3` and
    `icx: -Ofast -O4/-O9/-O2 still defines __FAST_MATH__`. ICX's numeric
    `-O<n>` clamping and its *assembly* withdrawal both matched GCC 16.2 /
    Clang 23.1.5 / LCCC; only the macro and this fixture's vectorization
    differ. ICX 2026.2 is built on an older LLVM base than Clang 23.1.5.
    Decision: LCCC keeps the GCC/Clang 23.1.5 policy, because a stale
    `__FAST_MATH__` macro after the compiler has withdrawn fast-math codegen
    is a correctness hazard for source that branches on the macro. The gate
    now asserts the two universal invariants (clamping, assembly withdrawal)
    for all four compilers, asserts the macro withdrawal and probe
    non-vacuity only for LCCC/GCC/Clang, and **reports and pins** the two ICX
    observations so a future ICX adopting the GCC/Clang behaviour cannot turn
    CI red. A negative control (a wrapper that drops every plain `-O*`) makes
    the ICX slot fail with three `did not withdraw` errors, proving the
    ICX-specific assertions are not vacuous. The measured divergence and the
    policy decision are also pinned in `test_ofast_composition.py`'s docstring.
18. **Host-CPU runtime behaviour was checked with a real debugger, not just
   assembly hashes.**  A four-frame `noinline` fixture compiled by LCCC,
   GCC 16.2 and Clang 23.1.5 at `-O2`, `-Ofast` and `-Og` produces identical
   results for inputs 5, -3, 0 and 12345 (`6n+16`). GDB's backtrace at `leaf`
   is `leaf -> middle -> top -> main` for all twelve compiler/level pairs, line
   breakpoints resolve for all three, and `-fno-omit-frame-pointer` produces a
   saved `rbp` chain where default `-O2` reports only `rip`. These are the
   two optimisation levels reclassified in item 1, so their "implemented"
   classification is now backed by runtime evidence, not only a macro delta.

## Alternatives weighed

| Option | Pros | Cons | Verdict |
|---|---|---|---|
| Implement 20 large compiler features (PIE, stack protector, `gcov`, IFUNC…) | maximal headline scope | no measurable acceptance test in one session; high miscompile risk; violates "prove your claims" | rejected |
| Re-run every existing gate and fix only failures | safe | misses defects that are green because the gate never executed (the ledger, the provenance note) | insufficient |
| **Probe each candidate, fix provable defects, ratchet already-closed ones, measure the open ones** | evidence for every claim; zero miscompile risk; regression coverage per item | requires a triage document | **chosen** |
| Add the 20 checks to `ci_local.sh` | direct gate | would require editing `ci.yml` + `ci-lane-plan.json` + parity registries, widening the CI-failure surface | rejected; `unittest discover -s tests/corpus` already runs in both `ci_local.sh:2477` and `ci.yml:3223`, so `tests/corpus/test_backlog_roi.py` executes locally **and** hosted with no wiring change |

## Function-level efficiency audit of the new code

Every new hot path is static analysis or a single compiler invocation; none is
in a per-compilation loop. Measured on the 2-vCPU host (minimum of 3 runs):

| Function / gate | Complexity | Measured | Why this shape |
|---|---|---|---|
| `option_effect_ledger.parse_rows` | O(source bytes), one regex pass | — | Block parser with one `finditer`; no backtracking-prone nested quantifiers. Comments are handled by field regexes rather than a pre-pass that would copy the file. |
| `option_effect_ledger.dynamic_problems` | O(rows × probes), 17 rows / 11 probes | 0.472 s total for `--lccc --self-test` | Two compiler invocations per row (`-S`, `-dM -E`) are the irreducible cost of an *executable* contract; results are hashed, never diffed, so memory is O(1) per row. |
| `check_backlog_roi.Context.load` | O(sum of 17 file sizes), read once | 0.081 s for all 20 checks + 21 negative controls | One read per input; checks share a frozen context instead of re-reading the tree 41 times. |
| `check_backlog_roi.run_all` | O(20) | included above | Each check returns named problems, so `--json` can attribute a failure to one item. |
| `check_dwarf_die_census.census` | O(DIE lines), streaming | 0.364 s for 4 dialects × 3 compilers | Line-by-line state machine; no DOM, no retained DIE tree, so memory is O(tags + attributes). |
| `Driver::set_fast_math` | O(1), six field writes | n/a (per-process option parse) | One shared helper replaced two duplicated assignment blocks; zero allocations, zero branches beyond the `enabled` flag and the sticky explicit-contract check. The removed defect cost an entire vectorized FP reduction at `-Ofast`. |
| `Driver::set_optimization_level` | O(1), five field writes + one call | n/a (per-process option parse) | Eight literal `-O` arms collapsed from 48 duplicated assignments to 8 calls. `optimize` and `omit_frame_pointer` are derived from `level > 0` instead of stored twice, so the two can no longer disagree. Zero allocations; inlined at fastbuild. |
| `Driver::withdraw_implicit_fast_math` | O(1), one `Option` tag test + one bool test, ≤6 writes | n/a (per-process option parse) | Called once, from the shared level helper, so no `-O` spelling can bypass it. Guarded on `fast_math` and `explicit_fast_math.is_none()`, so a plain `-O2` with no prior `-Ofast` writes nothing. |
| numeric `-O<n>` arm | O(digits), one `u64::from_str` with checked multiply/add, one `min(3)` | n/a (per-process option parse) | Overflow returns a named error instead of UB or a wraparound; `i32::MAX` bound is an explicit comparison, no cast that can truncate. Zero allocations. Measured effect: `-O4` now emits `189a4c8373a1` (`-O3`) instead of `8ffb18df` (unoptimized default). |
| `option_effect_ledger` × 4 arch drivers | O(4 × rows × probes) | 1.675 s for all 18 option-ledger tests | The same 17 rows are probed against four host executables that emit four targets' assembly; reuses parsed rows instead of re-reading the source four times. |
| `check_sse2_codegen_matrix` `--check` | O(122 rows × 3 compilers) | 6.230 s | Dominated by 366 real compilations; the checker itself is linear. |

No function allocates proportional to an untrusted input size beyond the file
it was written to read; no quadratic string concatenation; no shell
interpolation of paths in the Python gates (all `subprocess.run` with list
argv); no `eval`/`exec` on repository content in the production path. The
`test_dwarf_die_census_ratchet` negative control deliberately `exec`s a
mutated *copy* of the gate inside a temporary directory — that is test-only
and never reaches a compiler.

Design alternatives weighed for the ledger: (a) reuse the shell gate's
existing per-option `if` chain — rejected, it cannot enumerate the table and
would duplicate the source of truth; (b) parse `option_table.rs` with a Rust
build step — rejected, a Python gate can run in both the shell and corpus
lanes without a Cargo build; (c) emit a JSON contract from Rust and diff it —
rejected, that verifies *declarations*, not behaviour. The chosen design is
the only one that fails when the compiler's behaviour changes while the table
stays still.

## Verification

All results below are from the rebased tree (`origin/main` = `32a8f6a`, which
merged PR #1102 and later PR #1111's six backend/preprocessor commits),
rebuilt with the fastbuild preset and the pinned GCC 16.2.0 / Clang 23.1.5 /
GDB 18.1 / Rust 1.99.1 oracles. The rebase replayed all nine PR commits with
no conflicts; the full Rust suite, corpus and every gate below were re-run
against the rebuilt post-rebase binaries.

| Check | Result |
|---|---|
| `cargo build --profile fastbuild --locked`, `RUSTFLAGS=-D warnings` | **exit 0, zero warnings** (0 `warning` lines in the build log) |
| `cargo fmt --check` | **exit 0** |
| `scripts/check_clippy_policy.py` (CI's exact strict Clippy gate) | **exit 0**, 76 policy lint names probed, 0 unknown, zero warnings |
| `cargo test --profile fastbuild --locked` (full Rust suite, `RUSTFLAGS=-D warnings`) | **8078 passed, 0 failed, 10 ignored**, zero compiler warnings |
| `cargo test --profile fastbuild --locked driver_option_contract_table` | **1 passed, 0 failed**, 8087 filtered |
| `cargo test … ofast_* / numeric_o_levels_*` | **3 passed** (six umbrella fields, contract precedence, 17-vector option matrix, 11 accepted + 8 rejected numeric levels) |
| `option_effect_ledger.py --lccc … --self-test --json` | 17 rows, 11 probes, 7 priority options, **0 problems** |
| `check_driver_option_contract.sh --skip-trapv-matrix` | **exit 0**, including the executable ledger |
| `check_sse2_codegen_matrix.py --check / --self-test / --update-baseline` | **all PASS** (122 rows; re-derived against the live oracles) |
| `check_dwarf_die_census.py` (4 dialects × 3 compilers) | **PASS** with the tightened 12-tag / 20-attribute baseline |
| `check_backlog_roi.py --self-test --json` | **20/20 PASS, 0 negative-control failures** |
| `tests/regression/check_option_effect_ledger.py` (rebuilt binary, GCC 16.2.0) | **PASS**: non-vacuous `-Ofast` composition, 17 option-order vectors, 6 numeric levels + 6 malformed refusals, zero diagnostics; a macro-only wrapper exits 1 with two distinct diagnostics |
| `tests.corpus.test_ofast_composition` | **9 tests OK** (composition, GCC/Clang agreement, macro-only wrapper, shared-helper pin, 17-vector order matrix, order-blind wrapper) |
| `tests.corpus.test_dwarf_die_census_ratchet` | **8 tests OK**, incl. end-to-end lost-tag and stale-deferral negative controls |
| `tests.corpus.test_option_effect_ledger` | **24 tests OK**: 2 architecture-parity tests sweeping 4 drivers, 6 gate-robustness tests (7 behaviours), 16 parsing/static/dynamic tests |
| `tests.corpus.test_numeric_o_levels` | **10 tests OK** (byte equality with `-O3`, macros, zero diagnostics, GCC/Clang clamping, 8 refusals with pinned divergences, strict options, withdrawal, wrapper cliff, 4-arch sweep + pinned set) |
| `tests.corpus.test_host_cpu_dot_product` | **6 tests OK** (packed-codegen non-vacuity, `-O4 == -O3`, one-ulp checksums, loose interleaved timing gate, three-way measurement, wrapper negative control) |
| `tests.corpus.test_x86_torture_contracts` | **37 tests OK**, incl. 2 new delayed-consumer race regressions |
| new/changed corpus modules together | **87 tests OK** |
| `tests.corpus.test_gdb_option_runtime` | **7 tests OK** with pinned GDB 18.1, now also at `-O4` (12 compiler/level runtime + unwind pairs) |
| `unittest discover -s tests/corpus -p 'test_*.py'` | **1335 tests, 0 failures, 5 skips** |
| `check_doc_links.py`, `check_script_imports.py`, `check_no_conflict_markers.py`, `check_ci_workflow_shell.py` | **all PASS** |
| Hosted CI on `fd3d490` | **22 pass / 3 skipping / 0 failures**, including the authenticated ICX 2026.2 integration lane |

Previously-failing local gates were traced to a missing pinned environment:
with `env.sh` sourced, `oracle-toolchain`, `zlib-compare256-correctness`,
`trapv-boundary-matrix-m32`, `gcc17-builtin-semantics`,
`public-native-simd-abi` and `memory-value-contracts` all pass. Hosted CI is
the authoritative gate.

### §10 audit of the merged cache refresh

`engineering/FOLLOWUP-2026-11-05-clang-2315-gdb-181.md` §10 was traced
line-by-line against the tree.  Every claim still holds: the publisher derives
`CLANG_VERSION` from the enforcing pin, stages the digest-verified Clang prefix,
builds GDB from the shared installer, and refuses retired payload names;
`agent-cache-lock.json` names `harness-cache-8` with a GDB entry; the six
surviving release tags carry zero obsolete payloads.  One residual risk was
found and closed: nothing prevented a future `--refresh` from reintroducing an
obsolete filename into the reviewed lock, so the guard is now ratcheted by
item 20 (`CACHE-REDTEAM/1`) rather than resting on a one-time manual audit.

**Not claimed:** ICX 2026.2 cannot be installed in this sandbox (the Intel PPA
is outside the network allowlist), so no local three-way comparison was run.
Hosted CI's `icx-oracle` job (`ci.yml:3450`) provisions the authenticated ICX
binary and runs `check_enum_forward_abi.py --require-icx` plus
`check_kernel_codegen_contracts.py --require-icx` alongside GCC 16.2 and
Clang 23.1.5, and the ICX dispatch fixtures compare lccc against ICX output;
that lane, not a local claim, is authoritative for three-way agreement.  No PMU
data is claimed — instruction counts and assembly hashes are deterministic
compile-time observables.  No runtime-performance claim is made for any item.
