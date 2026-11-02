# Follow-up — PR #640…#629 semantic red-team audit

**Session date:** 2026-09-27 (UTC)

**Repository:** `ms178/lccc`

**Audited tree:** `e9f572eb36f5cd98c9ea1f077bc2922bd001177e` (`origin/main` at
session start), plus the corrective commit
`a5600dd203721054f8c5837a1e4ad1005f53d837` and the durable follow-up
commits `370767c3abcf41b067792d06c7cc8cc9b0c25ade` and
`a14be90d0a736065b344998251cfde6b1460a483` and
`71903f1c38c53d048b67919cc000c14b613373ec`.

**Deliverable:** `/home/user/ms178-1.patch`

**Snapshot continuity:** the authoritative snapshot tag, base/head, SHA-256
hashes, CI gate, and `APPLIES-CLEAN` result are recorded in
`/home/user/artifacts/SNAPSHOT_LEDGER.md`; the current canonical patch is
`/home/user/ms178-1.patch`. The full 87-gate run was green immediately before
the final wording-only refresh, and every exact-tree refresh after it passed
the fast mirror.

---

## 1. Executive result

The ten most recent merged PRs on `main` were audited as a connected change
set, not as ten isolated diffs. The audit covered the frontend/IR/backend
boundaries affected by the PRs, x86-64 and i686 code generation, the in-tree
linkers, CFI and `.eh_frame`, assembler encoding, liveness/register-cache
contracts, and the regression/oracle wiring that claims to validate them.

One real semantic defect was found and fixed:

> The shared `.eh_frame` CIE parser treated a DWARF v4 CIE as a v1/v3 CIE.
> DWARF v4 inserts `address_size` and `segment_selector_size` between the
> augmentation string and the alignment factors. The old parser therefore
> read the wrong bytes as the alignment and return-address fields and could
> reject a valid CIE or assign its FDEs the wrong pointer encoding.

The same audit found two related fail-closed defects in the EH-frame path and
fixed them in the same commit:

* unsigned absolute pointers were forced through `i64` (bad for 32-bit
  addresses with bit 31 set and for high-half 64-bit `udata8` values);
* unsupported text/data-relative and indirect encodings were treated as if
  their base were zero, and fixed-width header displacements were silently
  truncated to `i32`.

The corrective implementation now rejects unrepresentable/unsupported values
instead of manufacturing a plausible but wrong unwind-table entry.

No other production defect was demonstrated by the available evidence. The
remaining optimization gap is recorded rather than hidden: the new two-lane
I64 map path is correct and wins the local runtime experiment against GCC
14.2, but static Godbolt instruction counts still show a clear opportunity for
unroll/epilogue work against GCC 16.2 and ICX.

---

## 2. Exact PRs audited

| PR | merge commit | topic commit | primary area | verdict |
|---:|---|---|---|---|
| #640 | `e9f572e` | `9b02dfd` | I64/U64 map vectorization; shared `.eh_frame` reader hardening | **fixed: DWARF v4, pointer and range defects** |
| #639 | `8624a5d` | `7b7bb8a` | x86-family `%rax` epoch scope; sink coverage; x86 D32 moves | no residual defect demonstrated |
| #638 | `a89e685` | `af57893` | CFI from final code; real `.eh_frame`; ARX/reassociation | no residual defect demonstrated |
| #637 | `57ce949` | `b026ddd` | accumulator invalidation; SSE staging; thunk-reference scan | no residual defect demonstrated |
| #635 | `9bbf681` | `d39611c` | indexed GEP order; hidden-read liveness; coalescing; return pointers | no residual defect demonstrated |
| #633 | `0cc06e8` | `e11ec99` | revert of PR #632 | revert correctly neutralizes #632 |
| #632 | `5a71853` | `cac4a28` | folded-GEP/hidden-read/retpoline experiment | audited as reverted; no active code remains |
| #631 | `43c3cd4` | `8b1b99e` | SQLite miscompile fixes; assembler hardening; CI parity | no residual defect demonstrated |
| #630 | `a066f0c` | `737e98a` | x86/i686 VEX selection and EVEX operand validation | no residual defect demonstrated |
| #629 | `9c806fe` | `4f94abf` | GAS 2.47 EVEX/VEX parity and strict reciprocal stress | no residual defect demonstrated |

The full parent-to-topic diffs were retained during the audit under the
session's ephemeral `/home/user/audit/` directory; only the useful corrective
source and this durable report are in the deliverable.

---

## 3. Corrective change details

### 3.1 DWARF v4 CIE layout

`parse_cie_fde_encoding` now:

1. accepts only CIE versions 1 through 4;
2. for version 4, consumes and validates `address_size` and
   `segment_selector_size` before reading the alignment factors;
3. requires the CIE address size to agree with the output ELF class;
4. rejects non-zero segment selector sizes because this decoder does not have
   a segmented-address model.

This is deliberately fail-closed. Returning an FDE with a wrong encoding is
worse than omitting one entry: the unwinder's binary search can select a
semantically unrelated frame description.

### 3.2 Pointer decoding

`decode_eh_pointer` now keeps `(raw_value, signedness)` instead of coercing all
encodings through `i64`:

* `absptr`, `uleb128`, `udata2/4/8` remain unsigned;
* `sleb128`, `sdata2/4/8` remain signed;
* signed PC-relative values use checked signed addition;
* unsigned PC-relative values use checked unsigned addition;
* indirect, text-relative, and data-relative encodings return `None`, because
  the section-only decoder has no relocated image/text/data base to dereference
  or apply;
* all byte-range checks use subtraction/checked bounds and cannot wrap an
  offset before indexing.

### 3.3 Header field range law

`.eh_frame_hdr` explicitly uses `sdata4` for its pointer/table entries and
`udata4` for its count. Header construction now returns an empty header when a
count, allocation size, or displacement cannot be represented, rather than
casting with `as i32` and emitting a wrapped search table.

The FDE virtual-address calculation is checked as well.

### 3.4 Tests added

The linker-common unit suite now includes:

* a valid DWARF v4 `zR` CIE/FDE fixture;
* truncated and unknown-version v4 rejection;
* 32-bit high-bit absolute pointer decoding;
* high-half 64-bit `udata8` decoding;
* high-canonical-address PC-relative arithmetic;
* rejection of unsupported data-relative and indirect encodings;
* rejection of an unrepresentable `.eh_frame_hdr` displacement.

The existing malformed-record, extended-length, bounded-LEB, CIE-cache,
compaction, pruning, and arbitrary-record tests continue to pass.

---

## 4. Red-team evidence

### 4.1 Build and lint policy

* 8 GiB `/swapfile` is active (`/proc/swaps`), with the repository's
  `ensure_swap.sh` policy in place.
* Rust 1.98.1 / Edition 2024.
* LCCC built with the repository fastbuild preset, `-O1`, and `-j2`.
* `-D warnings` was used for targeted builds and the CI build gate.
* `cargo fmt --all -- --check`: **PASS**.
* `cargo clippy` through `ci_local.sh`: **PASS**, zero warnings.

The first fast-CI attempt was not treated as a code result: the disposable VM
lacked 32-bit libc headers, so the i686 portions of the new map and no-CFI
gates failed during preprocessing. `gcc-multilib` and `libc6-dev-i386` were
installed, the two gates were rerun directly, and both passed. The clean
post-fix fast run below is the authoritative result.

### 4.2 CI mirrors

`./scripts/ci_local.sh --fast` on the corrective source tree passed first:

```text
84 passed, 0 failed, 3 skipped
ALL GATES GREEN
```

The three skips are the script's intentional `--fast` skips:

* regression corpus with SSA validation;
* benchmark-output oracle;
* peephole whitespace invariance.

The final documented tree was then run through the complete
`./scripts/ci_local.sh` (no `--fast`):

```text
87 passed, 0 failed, 0 skipped
ALL GATES GREEN
pass stamp: target/ci_local.pass (full)
```

The slow gates supplied the missing evidence: **815/815** regression-corpus
cases passed with SSA validation, the benchmark-output oracle reported
**204 PASS / 0 FAIL / 0 SKIP**, and the whitespace-invariance gate completed
its in-tree, operand-matrix, and generated-assembly phases successfully.
All other gates passed as well, including cargo tests, differential
correctness, fuzz smoke, CFI/eh-frame unwind, assembler/GAS 2.47 parity, all
BB-SLP red-team gates, vectorization gates, cross-backend atomics, codegen
quality, rustfmt, and clippy.

Targeted post-fix results:

```text
linker_common::eh_frame::tests: 16 passed, 0 failed
check_eh_frame_unwind.sh: PASS
check_cfi_invariants.sh: PASS (8538 functions, 9139 returns)
check_map_i64_two_lane.sh: PASS
check_nocfi_peephole_parity.sh: PASS
```

### 4.3 I64/U64 map stress

A separate deterministic stress fixture exercised signed and unsigned 64-bit
maps, add/sub/mul/bitwise/shift/div/modulo, invariant broadcasts, conditional
maps, three-stream maps, tails from zero through 72 elements, larger trip
counts, unaligned destinations, signed `int64_t`, and legal non-`restrict`
overlap. It passed at:

```text
-O0/-O1/-O2/-O3 × x86-64 and x86-64-v3
```

The overlap test was deliberately corrected to remove `restrict`; the initial
version was testing undefined behaviour, and GCC -O3 failed it too. The final
fixture and the compiler's scalar-kill-switch comparison both passed.

### 4.4 Godbolt oracle

`scripts/godbolt.py audit` passed and resolved the requested aliases:

```text
gcc16.2    -> x86-64 gcc 16.2
clang23.1  -> x86-64 clang 23.1.0
icc        -> x86-64 icc 2021.10.0
icx        -> x86-64 icx (latest)
```

The standalone map reproducer was compared with all four remote oracles. The
recorded instruction counts are static counts, not a runtime claim:

| flags | LCCC | GCC 16.2 | Clang 23.1 | ICC | ICX |
|---|---:|---:|---:|---:|---:|
| `-O2 -march=x86-64` | 47 | 11 | 28 | 74 | 55 |
| `-O2 -march=x86-64-v3` | 46 | 33 | 50 | 74 | 25 |

This is an honest remaining code-quality gap: LCCC's two-lane path is not yet
unrolled as aggressively as the best remote implementations. The local
runtime harness (`scripts/bench_kernels.py`, best-of 5, inner 200, unpinned
VM) measured `map64_sub` at **1.486x faster than local GCC 14.2**. That result
is useful evidence for this VM only; it is not presented as a Raptor Lake or
PMU result.

Oracle artifacts are in `/home/user/artifacts/godbolt-map64/` and include the
source hash, flags, resolved compiler IDs, and manifest.

---

## 5. Per-PR adjudication

### PR #640 — fixed finding

The map gate and shared EH-frame parser were read together because the PR
changes both target-gated optimization and linker metadata. The map gate
covers target reset (x86-64 then i686), SSE2/AVX2 width selection, tails,
scalar kill-switch, alias fallback, and runtime equality. The EH-frame audit
found the v4/pointer/range defects described above and fixed them with unit
fixtures.

### PR #639 — accepted after audit

The `%rax` epoch scanner is debug-only and opt-in only for x86-64/i686. The
review checked the implicit writer table (`div`, `idiv`, one-operand `imul`,
`mul`, calls, string ops, compare-exchange, system instructions), generic
AT&T destination rules, xchg/xadd/mulx operand classes, embedded-newline
scanning, truncation/re-emission, and the final full-buffer assertion. Existing
classifier and sink-coverage tests plus the full fast mirror passed.

### PR #638 — accepted after audit

The final-code CFI derivation, x86/i686 CFI emission, `.eh_frame` synthesis,
ARX frame selection, and reassociation paths were checked against the CFI
invariant and unwind gates. The critical invariant is derived from final
machine code, not a pre-peephole estimate; the tested output had 8,538
functions and 9,139 returns with no invariant violation.

### PR #637 — accepted after audit

The audit followed the accumulator-cache dataflow through call staging,
casts, comparisons, floating-point paths, memory paths, return paths, GVN,
and both x86-family backends. Width-exact SSE staging and code-only
retpoline/thunk scans are covered by their dedicated regression gates. No
uncovered write or stale cache use was demonstrated.

### PR #635 — accepted after audit

Indexed-GEP emission order was checked for def-before-use, hidden reads were
checked in liveness and peephole replay, commuted-RHS coalescing was checked
against type/width constraints, and slot-independent accumulator sourcing was
checked against the debug epoch contract. The vector-copy and replay/phi
contracts passed.

### PRs #632 and #633 — accepted as a revert pair

#632's folded-GEP/hidden-read/retpoline experiment was immediately reverted by
#633. Both diffs were audited so the review did not accidentally reason about
#632 as production code. The effective main-tree behavior is the #633 revert;
no #632-only optimization is claimed in this report.

### PR #631 — accepted after audit

The audit covered the SQLite reproducer family, function-pointer/type-scope
regressions, IV-widen exit bounds, displacement composition, callee-saved
relay across calls, tentative incomplete arrays, CI gate parity, and the
assembler modes inherited from the preceding PR. Differential correctness,
SSA validation where enabled by the gate, and the cross-PR red-team gate
passed.

### PR #630 — accepted after audit

VEX direction/hint handling and EVEX operand validation were compared against
GAS 2.47 for x86-64 and i686. The merged PR629 follow-up casefiles passed:
90/90 x86-64 cases and 568/568 i686 cases in the fast mirror.

### PR #629 — accepted after audit

The audit covered EVEX promotion, disp8-N scaling, tuple/operand validation,
VEX direction, strict computed reciprocal packing, and the added LZ4 benchmark
fixture. GAS 2.47 parity and the strict reciprocal codegen gate passed. No
unsupported instruction was accepted by the tested encoder paths.

---

## 6. Known limits and follow-up queue

1. **Keep the full 87-gate result reproducible.** The final documented tree
   is currently stamped `ci_local-full-PASS`; any source or documentation edit
   must be followed by a fresh exact-tree pass before publishing another
   delivery snapshot.
2. **Close the I64 map instruction-count gap.** Prototype a two- or four-iteration
   unroll only when the cost model predicts enough trip count to amortize the
   extra epilogue/branch work; validate against the same scalar fallback and
   Godbolt set. Do not optimize the static count at the expense of small loops.
3. **Run on the requested i7-14700KF.** This VM has an Intel Xeon virtual CPU,
   no `perf`/PMU interface, and no P/E-core topology. The current evidence is
   deterministic correctness plus wall-clock best-of sampling, not hardware
   counter evidence.
4. **Keep ICX/ICC manifests fresh.** The oracle audit currently resolves all
   aliases; every future claim must preserve the resolved IDs in its artifact
   manifest.
5. **Consider a structured result type for EH-frame header construction.** The
   current public API historically uses an empty vector for “cannot build”; a
   future linker API could distinguish “empty valid section” from “rejected
   malformed input” without changing the current fail-closed behavior.
6. **Retain the multilib preflight in session restore.** The missing i386 libc
   headers were an environment failure, not a compiler failure, but without
   the preflight the regression gates report a misleading red result.

No performance win is accepted merely because an IR shape or instruction
count looks better. The next map optimization must carry a reproducer,
assembly, correctness differential, runtime distribution, and (on real
hardware) counters.
