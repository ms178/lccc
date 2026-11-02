# Follow-up — A14-30 backend root cause and A14-29 evidence repair

## Scope and identity

Session base: `e047aa1eb2ae59b3943a64ab7ec5e5dddb89a3ff` (ms178/lccc main).
The workspace initially had no previous patch or restored worktree. This patch
is cumulative against that main, not a reconstruction of an unavailable patch.
Changes, evidence and this handoff are in the source archive and this
cumulative follow-up commit. The exact final base/head and fast-CI gate status
are in the snapshot receipt and ledger; an intermediate UNGATED checkpoint is
not CI proof.

This is a bounded correctness iteration, **not** completion of the whole backlog,
a line-by-line audit of every subsystem, or proof that every compiler function
is optimal. No allocator, loop theorem, ISA default, linker algorithm, FP model
or profitability heuristic was changed.

## A14-30: cause, alternatives, implementation

The licensed two-TU zlib-ng extraction was replayed before editing. The expected
complete-table SHA-256 is
`022736ff3362ef3591d1f8fc3b7fede5927c0d25ab0813d7ef8cd95efd6ea242`;
the old -O2 output was
`531118a2c07a32090b063ce1fb96e713f0716273c7924f114c49c78e8619cd32`.
The first distance-table byte was 144 rather than zero; the literal-code table
also differed. The existing no-IV-widen/LICM/GVN controls identify enabling
transformations, not the defect's ownership.

New isolations:

* `LCCC_NO_PEEPHOLE=1` still miscompiled, excluding the final text peepholes as
  a necessary cause.
* `CCC_NO_MACHINST=1` produced the exact expected table.
* IR after widening/LICM retained the local counter address. In `tr_static_init`,
  `v145 = GEP(v5, 16)` is defined in a dominating preheader that appears after
  the loop in the block vector. `v5` is the local 32-byte `bl_count` allocation.
* The shared generator marks a folded GEP when **emitting its producer**.
  MachInst accepted earlier-emitted loads/stores before that marker existed,
  reading a never-materialized pointer register. In the saved main assembly,
  `%r12` still addressed `dist_code`, so the counter's 144 increments corrupted
  that global and left the intended `bl_count[8]` wrong.

The existing guard already consulted the function-wide **indexed** address map.
The fix extends that exact contract to the **constant-offset** map, routing
map-managed Load/Store accesses through the map-aware classic generator. It
folds or rematerializes their addresses rather than reading an unwritten home.
No instruction producer is speculatively marked, no register is invented, and
no IR optimization is disabled.

Alternatives rejected:

1. Disable IV widening/LICM/GVN: hides the symptom and needlessly discards safe
   optimization across the corpus; does not repair the backend contract.
2. Sort all blocks into dominance order: changes layout globally and still
   makes correctness depend on ordering rather than address availability.
3. Materialize every folded GEP: discards addressing-mode work and adds pointer
   lifetimes/register pressure everywhere.
4. Teach MachInst both address maps immediately: possible future architecture,
   but duplicates mature fallback/segment/alignment contracts for this fix.
   A narrow dispatch repair is easier to validate and uses an existing invariant.

The new guard adds at most one extra hash-map membership query per candidate
memory access. That is a compile-time cost, not a generated-code instruction.
No compile-time speedup or exact overhead is claimed.

### Validation

* Full zlib-ng 2.3.3 with the existing SHA-pinned archpkgbuilds patch: **69/69
  CTests**, including maketrees; independent round trips at levels 1/2/6/9 pass.
* `check_zlib_maketrees.sh`: all bytes compared at -O0/-O1/-O2/-O3/-Os/-Oz, plus
  -O2 no-IV-widen/no-MachInst/no-peephole controls. Executable failures and
  timeouts cannot become a passing checksum. Wired into local and hosted CI.
* Mutation control: the new gate fails at -O2 with the retained pre-fix binary;
  -O0/-O1 controls pass. No weakening of the golden or optimization flags.
* Two Rust dispatch tests pass. Constant offsets -16/0/16/i32::MAX, loads,
  stores, volatile-store classification, producer/non-access negative control;
  the prior indexed-address/foreign-pointer test remains intact.
* A 58-file, six-level before/after assembly census: **348 identical outputs**,
  zero compile failures. This is a codegen-change screen, not runtime evidence
  or a license to waive future measured layout regressions.
* The new predicate is in shared generation, but MachInst is the x86 path;
  other targets' default hook declines. No new target-specific encoding is
  introduced. Full target runtime certification is not claimed.

## A14-29: JUnit execution state is not outcome

Replayed the saved real CTest reports: both executions succeeded, but the old
checker threw on `status="run"`. Merely adding `run -> pass` would leave its
other early-return bug: a prior status or child could hide a conflicting or
unknown later attribute.

The replacement collects all outcome channels, treats CTest `status=run` as an
execution marker, rejects contradictions/unknowns, and validates optional suite
counters against the cases in each suite's subtree. Missing case records cannot
erase an aggregate failure. Disabled counts may only describe a subset of
skipped cases. The existing identity multiset and configuration comparisons
remain enforced; equal failures or an omitted test do not count as comparable.

**44 self-tests pass**, including run+failure/error, conflicting attributes and
children, unknown secondary status, nested suite summaries, count mismatch,
malformed/empty reports and existing missing/duplicate/configuration cases.
The retained real GCC/LCCC XML now compares successfully. The full fresh Expat
runner completes: **4,932 upstream checks per arm** and **8/8 XML cases**
(canonical output and invalid-input rejection), with a complete passing summary.
Self-test fixtures use one scoped TemporaryDirectory instead of leaking
directories on each run. The six extracted-workload filters (gzip, zlib-ng,
Expat, SQLite, glibc, Linux) additionally pass **40/40** optimization-level cases
with zero failures/skips against the local execution reference. Full gzip 1.14
passes **30/30 upstream tests per arm** and all six compressed-stream identities
agree. Its concurrent single-round timing screen is deliberately excluded from
retained performance evidence; see `gzip-correctness.json`.

## Oracle and VM evidence policy

The existing `codegen_oracle.py` was used actively on gzip CRC32, zlib-ng Adler32
and Expat SipHash at -O2 x86-64-v3. GCC 16.2, Clang 23.1 and CE ICX compiled.
ICC correctly failed closed because it does not honor `-march=x86-64-v3`.
A **separate**, explicitly labelled ICC `-march=core-avx2` control compiled all
three. It is not silently merged into an equal-ISA competition result. CE's ICX
alias is not proof of the latest Intel release. A separate `-v` identity probe
records GCC 16.2.0, Clang 23.1.0, ICC 2021.10.0 and **ICX 2024.0.0 (20231017)**;
see `oracle-identities.json`. Do not call that ICX installation a 2026 release.
Static whole-TU instruction counts still show substantial gaps; they are not
hot-loop throughput ratios and do not establish which compiler runs fastest.

Local distro GCC is used for execution/sanitizer correctness controls, clearly
identified in environment.json; it is **not** relabelled GCC 16.2. Target runtime
superiority requires actual 14700KF measurements. This KVM Xeon guest has two
vCPUs and about 2 GiB RAM, not a Raptor Lake P/E-core topology. No PMU counters,
cycle ratios, statistical confidence intervals or bare-metal wins are claimed.
Concurrent build/test wall times are explicitly excluded from performance
interpretation. Assembly equality is evidence about code identity only.

Build policy: stable Rust, `scripts/build_lccc_fast.sh`, fastbuild/-O1/-j2,
`-D warnings`, initially 2 GiB swap on the largest filesystem, extended to
4 GiB total during the full all-targets test build. Actual swap use was observed. Toolchains and build caches are under excluded `.cache`; the persisted
source tarball, session bundle, checkpoints and recovery script are outside it.
The original snapshot attempt hit the publication budget because Rust was in
the visible workspace; it was retried successfully after relocating caches.

The linker provisioner already pins bfd/GAS 2.47, lld 23.1 and mold 2.42.1.
Upstream mold v2.42.1 CMakeLists.txt was checked: the exact target variable is
`-DMOLD_TARGETS='X86_64;I386'`; X86_64 remains first. No redundant provisioner
rewrite or claim that unbuilt linkers were benchmarked is justified.
Package selectors were checked against archpkgbuilds main
`b57eabef31f3812362f52c68bec43f88d3f6a0a4`: zlib-ng 2.3.3 / Expat 2.8.5 match the
existing pinned runners. The zlib patch digest remains
`40a317e1ac64e458bab6133ef259288edeab1a9054249dece3c7048fade3c2cc`.

## Remaining work, in priority order

1. **P0 A14-17/19/14/15/13/20:** unknown RISC-V shared relocations, forbidden
   AArch64 SIMD, ADR range bounds, unsupported data expressions, target vector
   sizing and PGO flush/locking. The existing contract probes remain the entry
   point. This patch does not close them.
2. **P0 evidence A14-01/02/03:** census lexer, every measured sample's checksum,
   and invalid normalized-hash noise waivers. Do not trust an aggregate speed
   verdict until those specific contracts are repaired.
3. Audit other consumers of emission-order-dependent `folded_gep_values`,
   particularly aliases and operand materialization. Do not generalize the new
   Load/Store guard to unsupported instruction classes without verifying the
   map builders' use filters and source lifetimes.
4. Measure the fix on real 14700KF workloads only after correctness. Separate
   P/E cores, affinity, SMT, thermal/frequency state, randomized paired trials,
   per-sample output checks and PMU data. The 348 unchanged assemblies provide
   a useful control set, not an automatic noise waiver.
5. A14-18: explicit per-vendor effective-feature contracts in the oracle;
   preserve the demonstrated ICC mismatch rather than suppress its warning.
6. Full Linux/glibc/SQLite builds, native AArch64/RISC-V runs, long fuzz campaigns,
   ASan/UBSan breadth, and all other open backlog items remain uncompleted.
7. gzip's runner uses an unbounded `urlretrieve`; the direct GNU endpoint stalled
   here. A SHA-verified mirror archive supplied through its existing `--archive`
   option avoids the stall. Add bounded/retryable fetching as a separate tested
   harness change, without weakening the digest pin.

## Reproduction and recovery

```sh
scripts/ensure_swap.sh
scripts/build_lccc_fast.sh
bash tests/regression/check_zlib_maketrees.sh
python3 tests/workloads/expat-2.8.5/junit_check.py --self-test
# Dedicated disposable directories only (existing zlib runner owns WORK_DIR):
KEEP_WORK=1 WORK_DIR="$HOME/.cache/zlib-next" \
  ARTIFACT_DIR="$HOME/.cache/zlib-next-evidence" \
  bash tests/workloads/zlib-ng-2.3.3/run.sh
WORK_ROOT="$HOME/.cache" ARTIFACT_DIR="$HOME/.cache/expat-next-evidence" \
  bash tests/workloads/expat-2.8.5/run.sh
scripts/ci_local.sh --fast
```

An independent offline restore of S06 with `LCCC-RECOVER.py --offline
--require-exact-history` reconstructed the original base, session commits and
HEAD tree successfully (`mode=offline-reconstructed-base`, `exact_history=true`).
This checks actual recovery, not merely tar readability.

Read the snapshot ledger before restoring. Verify the source archive and patch
hashes with the supplied recovery script, fetch main, rebase session commits and
re-anchor `LCCC_BASE_REF`. Snapshots S01 onward are cumulative named checkpoints;
retain previous generations across rebases. Do not rely on `.cache` surviving.
