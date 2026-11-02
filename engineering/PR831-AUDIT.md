# PR831 follow-up audit — 2026-10-15

## Scope and honesty boundary

Reviewed hosted head: `9881506686a270497d635917dde75297ed805a5f`.
Integration base: `7f9eb225def23bda9370d3349304b9b785554fea`.
Dedicated follow-up branch: `audit/pr831-redteam`.

The previous S41 local FAST pass was not a hosted/full pass. The PR831 source
also removed ten historical plaintext evidence logs; its compiler source was
otherwise the delivered revision. This follow-up preserves those logs losslessly
as gzip with raw-content hashes in `engineering/evidence/pr828/RAW-LOGS.json`.
It does not remove their tests or silently rewrite historical results.

No external expert approval, universal bug-freedom, exhaustive review of every
repository function, or optimality claim is made. The review lenses are ISA/ABI
semantics, dataflow proof soundness, arithmetic domains, reproducibility, and
performance controls; they are not fictitious independent human reviewers.

## Hosted failures: evidence before hypotheses

The completed PR831 run reports two failed execution lanes and a correctly
failed aggregate. Its build, Clippy, MSRV, assertion-enabled units/corpus, SSA
corpus, and codegen/runtime lanes report success. These are results of the OLD
hosted revision, not acceptance of this follow-up.

1. **Assembler/linker:** downloaded `linker-results.json` contains 302 PASS,
   zero FAIL/WARN, and one SKIP: `plain_gotpcrel_relaxation`, because the fixture's
   assembler rejects MOVRS. Strict mode correctly refuses this skip.
2. **Frontend/ABI:** installation of GCC16/multilib exits 1 before the tests.
   Public job metadata identifies the failing step but not the package-manager
   diagnostic. Authenticated log downloads were unavailable; the user selected
   continuing without supplying that log. **The precise cause remains open.**
   The independently found C++-version check gap below is NOT claimed to explain
   or repair that hosted installation failure.

## Attributable repairs and alternatives

### A. Select the pinned assembler for the assembly fixture

`_plain_gotpcrel_test` used `CC -Wa,...` while the sibling MOVRS fixture invoked
`as` directly. Putting GAS2.47 on PATH does not override a compiler configured
with a private/absolute assembler. Both `pg.s` and `sc.s` now invoke
`as --64 -mrelax-relocations=no` directly; C compilation and linker shims remain
unchanged. The existing exact relocation, byte, runtime, PIE/static/shared and
negative script-link assertions remain intact.

Rejected: weakening `--strict`, deleting MOVRS, or applying a global compiler
`-B` that can also change CRT/library/linker selection. No compiler production
change is needed for this harness defect.

Evidence: new controller tests fail before the fix, pass afterwards, and are
wired into hosted and local FAST. A real GNU as2.40 private-driver control
reproduces the reported diagnostic before repair and passes the complete
four-mode fixture after repair. System GAS2.44 passes BOTH versions: the
original local environment was insufficient to expose the selection defect.
The old-GAS control is a reproduction of the mechanism, not an Ubuntu VM replay.

### B. Exact byte-range endpoints beyond signed displacement limits

The inherited shared `ranges_overlap` helper added i32 displacements and widths
in i32. A valid memory range can extend past i32::MAX even though its start is
an encodable displacement. With overflow checks this panics; without them it
can wrongly say two overlapping regions do not overlap. Empty ranges also
incorrectly overlapped in some cases.

The helper now excludes empty/nonpositive ranges and adds endpoints in i64.
Two i32 inputs cannot overflow that domain. This preserves exact overlap rather
than saturating/wrapping or treating every boundary case as an opaque alias.
The changes affect compiler-side reasoning, not inserted target instructions.
All callers were located: dead-code/store forwarding, FP slot liveness and
frame compaction. An FP consumer test checks a live read spanning the signed
endpoint and a genuinely disjoint negative-displacement positive control.

Evidence: the original helper fails two extracted-source tests with overflow
checking both enabled and disabled. The repaired helper passes an independent
i128 interval oracle over 8,100 combinations, plus the direct boundary case.
The integrated consumer and full library pass. This is a proven proof-helper
failure; no fabricated end-to-end C miscompile is attributed to it.

### C. EVEX decorator whitespace must not change write obligations

Whole-write deletion proofs stripped a mask but did not trim the preceding
whitespace. A GAS-valid `%xmm2 {%k1}` destination therefore lost its register
identity and could evade the upper-zero write obligation. The regression was
red in the actual Rust suite (one failing, one passing neighboring test).

Both whole-write and upper-effect projections now trim the undecorated operand.
Decorated destinations conservatively retain partial-write treatment, including
spaces INSIDE the braces. This also avoids introducing a merge-mask bug while
fixing register recognition. Unmasked full definitions retain their kill proofs;
no optimization family is disabled and no global assembly reformatting is needed.

Tests cover four spellings, observable XMM upper-zero writes, and YMM merge-mask
inputs. A native AVX512F/VL architectural witness on this Xeon additionally
checks all four spellings and a deliberately deleted-producer mutant: four
correct upper-zero results, while deletion preserves the nonzero high 48 bytes.
This witness is NOT a full C optimizer reproducer and is NOT 14700KF evidence;
that processor does not expose this AVX-512 ISA.

### D. Validate both installed GCC toolchain drivers

Package status plus a binary named `g++-16` did not establish its version.
A controlled PATH with GCC16.2 and G++15 was incorrectly certified. The helper
now checks `g++-16 -dumpfullversion` against the validated C driver's version,
including the already-installed fast path. Exact-version overrides and the
existing rolling-major policy remain intact. Four mismatched/malformed-version
subcases were red, then green; the complete eight-test provisioning suite passes.

### E. Preserve evidence without patch whitespace warnings

Ten historical raw logs were compressed, decompressed and SHA256 checked.
`git diff --check origin/main` is clean; this is not a whitespace-ignore switch.
Historical log bytes are preserved, not edited to hide failed runs.

## Wider review and executed controls

The cumulative patch changes twenty Rust files including embedded tests. The
review revisited IVSR definition indexing/dominance and symbol-base reuse,
Copy-only identity, signed-width reduction/remainder bounds, pointer-ring versus
byte-count representability, transactional packed integer DAG construction,
LICM canonical write/ordering barriers, and vector low/upper/slot consumers.
Known CFG/directive, pinned, volatile and FP restrictions were retained.
This is not a claim that each repository function was exhaustively verified.

Full library after B/C: **4,747 pass / 0 fail / 8 ignored**. This includes module
coverage for x86-64, i686, ARM/AArch64 and RISC-V, not native execution of all
architectures. The fresh assertion compiler passes the 22 targeted reported
cases with IR/SSA validation. CI parity: **226 commands / 22 invocation contracts**;
71 CI-gate-parity tests and the eight GCC-provisioning tests pass.

All compiler/linker builds use fastbuild, opt-level1, exactly `-j 2`; 4 GiB swap
is active. Normal candidate, independent immutable main, and assertion compiler
have distinct recorded identities. No release/LTO iteration was used.

## Runtime-codegen controls after the repairs

Three alternating pairs for each of five workloads, GCC16.2 output oracle,
equal-length executable paths, fixed inputs/flags/cache geometry, Callgrind:

| Workload | Candidate/main whole-process simulated Ir |
|---|---:|
| nbody | 1.0 |
| fannkuch | 0.9523269469412591 |
| histogram | 1.0 |
| libm_round_family | 1.0 |
| lz4_match_extend | 1.0 |

All three ratios per workload agree. Fannkuch: **1,230,175 / 1,291,757 Ir**,
a reduction of **4.7673%**, not cycles or native speedup. N6/N8/N9 three-pair
controls also improve and agree with GCC16 and the assertion compiler.
The ten candidate/main measured ELFs are byte-identical to their S41 counterparts;
these repairs are NOT claimed as an additional workload-codegen improvement.
Fresh whole-process counters are retained separately from historical counters.

All fifteen current candidate/main/GCC executables were separately recompiled:
byte-identical ELF, empty compiler stderr, exact output digests. Disassembly,
raw stdout/stderr, and verified compressed raw Callgrind counters are retained.
Optional Godbolt GCC16.2, Clang23.1 and latest ICX accept the same bounded
fannkuch ISA/optimization flags, have empty compiler diagnostics and execute
with the expected output. This is not a native performance ranking of vendors.
No live Compiler Explorer merge gate was added.

## Final acceptance boundary

This tracked document is frozen BEFORE the final immutable FAST run. Its result
and exact before/after identities will be external receipts under
`artifacts/pr831/`, so publishing the result does not mutate the accepted tree.

Still required: resolve/replay the hosted GCC installation failure, rerun the
hosted lanes on the new revision, and obtain native 14700KF/PMU evidence before
claiming that target's performance. Broad effect-descriptor consolidation and
universal missed-optimization claims are not declared completed. No push and no
production-ready/zero-issues certificate are issued by this report.
