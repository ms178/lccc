# PR791 independent review — 2026-10-11

Base: `6dd8eeeaa98aa5314b7a06264ae5753734c493e7`, ms178/lccc main,
merge of PR791. This is a new cumulative patch against that merge, not the
already-merged a9987 salvage patch. Conclusions are bounded by the evidence
below; no numerical quality score is assigned.

## Review disposition

| Review point | Independent finding | Resolution |
|---|---|---|
| 1. Portable CLZ uses defined-zero intrinsic unnecessarily | Reproduced on explicit `-march=x86-64`: `x|1` followed by test/branch/zero fallback before BSR. The driver's host-native default would hide this. | Emit `ClzNonZero(x|1)`; call the matcher once; preserve the Select's I32/U32 type. |
| 2. Guarded CLZ retains OR1 | Reproduced in standalone guards and the existing `bitops` loop. | CVP bypasses only a same-width safety OR under a dominating nonzero fact. Select simplification returns the existing safe arm for fallback W-1, or creates **defined-zero** Clz for fallback W and equal result/operand widths. |
| 3. Missing per-node widths | Confirmed by a failing width-mutation test and an IR-level semantic witness: changing the first CLZ comparison to U16 gives 31 for `0x100000`; old recognition changes it to 11. This is not a claimed newly found C frontend reproducer. | Require each count/swap arithmetic, compare and Select node to operate in its proved ring. CTZ working values are 64-bit; counts are 32-bit. Copies remain aliases; arbitrary casts do not. |
| 4. In-loop expansion invalidates coordinates | Confirmed source indexing hazard and failing mixed-transform source-span regression. Original CTZ/narrow-rotate insertions precede later uses of the initial def_loc table. | One deferred expansion queue for CLZ, CTZ and narrow rotates; reverse application after paired mask destination validation; span replication. Boolean rewrites and their def/use accounting remain immediate. |
| 5. Missing count codegen contracts | Confirmed. Also found an unmentioned recognition hole: the existing C `ctz_full` control emitted 54 instructions and **no TZCNT**, because its Select conditions were direct masks with reversed arms, not Eq-zero compares. | Normalize both proved CTZ condition polarities; add explicit baseline/v3 width/branch/OR contracts and truncated near-misses, plus i686 runtime coverage. |

### Where the review needed qualification

- Not all findings are demonstrated C wrong-code. Distinguish the width-level
  IR witness, the observed source-span corruption/index hazard, and missing
  machine-code quality. The tests do not establish that every width mutation
  changes a source program's output.
- No blanket cast peeling: a nonzero 64-bit input can truncate to zero at
  32 bits; `clz_guard_wide(0x100000000)` must remain 31. The simplifier stays
  conservative on widening result carriers and on condition widenings; CVP
  can use existing zero-equivalence proofs for same-width/widening casts.
- A select's fallback W permits a **new defined-zero intrinsic**, not a copy
  of a bare speculated NonZero arm. Existing bare-NonZero rejection tests stay.
- Negative opcode assertions are current whole-network shape contracts, not
  a prohibition against a future separately proved partial/truncated fold.
  Runtime witnesses and per-node IR proofs remain the semantic authority.
- "All are non-regressions" is not established as a blanket historical claim.
  CLZ's safe-OR cost came from the merged correctness fix; the insertion and
  width issues existed in the reviewed base. No broader history audit is
  implied here.

### Additional constant-carrier proof guard

Operation widths alone are not sufficient. A literal `I16(-1)` in a 64-bit
AND means all 64 bits set, not the low-16 mask. The recognizer now normalizes
literal values in the operation's ring, consistent with the IR constant
folder, rather than treating their storage carrier as an unsigned mask.
A new test checks fully operation-width-correct CLZ/CTZ networks with narrow
negative literals: recognition must reject them and preserve their result 31
at the selected inputs. These are additional IR proof guards, not a claim of
newly reproduced C frontend miscompilations. The independent fixture evaluator
was also corrected to interpret signed literals before applying node widths.

## Additional design change and caught regression

The old CTZ expansion used comparison, safe-input Select, Ctz, result Cast,
and fallback Select. Instead, set bit 63 and issue `CtzNonZero`, then cast its
small result to the original count carrier:

```
ctz64(x | (1ULL << 63)) == min(ctz64_defined(x), 63)
clz32(x | 1)            == min(clz32_defined(x), 31)
```

For a nonzero x the extremal bit cannot change the relevant first set bit;
zero maps to the source tree's exact W-1 result. This also removes an
unreachable U64-output branch: the CTZ matcher accepts only 32-bit count
Selects.

The first candidate passed all 26 C configurations but regressed the Linux
find-bit extract: its new CTZ high-bit OR survived an existing nonzero guard,
adding instructions and 16 bytes to the ELF. This was caught by the before/
after workload comparison, not averaged away. CVP now handles **both**
extremal-bit safety ORs, with a guarded CTZ assembly contract and dominance/
truncation unit tests. The final Linux binary is byte-identical to the baseline; this regression is not accepted in the delivery.

## Validation and measured results

- **4290 Rust tests passed, 0 failed, 7 ignored** on the final production code.
- **28 executable configurations passed**, with GCC stdout agreement: five
  programs at O0/O1/O2/O3, both count programs on explicit x86-64 baseline at
  O2, and both on i686 at O1/O2/O3. Count fixtures include zero, every single
  bit, seeded values and truncation witnesses; safe-guard fixtures include
  high-bit-only inputs, signed/unsigned widening and truncating guards.
- Assembly contracts additionally compile x86-64-v3 without executing it:
  LZCNT32 / TZCNT64 positive controls, current truncated-network negatives,
  branch-free unguarded safe counts on baseline and v3, no redundant OR under
  the validated CLZ/CTZ guards.
- The existing strict workload runner checks gzip CRC32, zlib-ng Adler32,
  Expat XML scan, SQLite varints, Linux find-bit, glibc memcmp and bitops.
  **All seven match GCC; six final binaries are byte-identical to baseline.**
  Only bitops changes; its main function loses MOV+OR before LZCNT (82 to 80
  static instructions, including returns). Linux is restored exactly after
  the intermediate regression.

### Code-generation distribution, not runtime speedups

All rows below request `-O2 -march=x86-64-v3`; counts include returns and are
static function-body instruction counts. The external column excludes LCCC.
Compiler Explorer labels: GCC 16.2, Clang 23.1.0, ICC 2021.10.0, ICX latest.
The local executable reference is separately identified as GCC 14.2.

| Function | LCCC before | LCCC final | Fewest external instructions |
|---|---:|---:|---:|
| `clz_full` | 4 | 4 | 25 |
| `ctz_full` | 54 | 6 | 32 |
| `clz_trunc` | 42 | 42 | 5 |
| `ctz_trunc` | 54 | 54 | 30 |
| `clz_guard32` | 8 | 7 | 29 |
| `clz_guard31` | 8 | 7 | 29 |
| `clz_guard_live` | 9 | 8 | 30 |
| `clz_guard_wide` | 9 | 9 | 29 |
| `ctz_guard64` | 58 | 7 | 36 |

Explicit baseline `-march=x86-64` CLZ improves **9 to 5 instructions** and
has no zero-branch diamond. V3 CLZ was already branchless (4 to 4); that
baseline-specific benefit must not be advertised as a v3 instruction saving.
The tables retain the sizeable truncated-tree gaps rather than hiding them
behind the canonical CTZ improvement. Requested common flags and CE labels
are recorded; no unmeasured compiler diagnostics or fixed ICX build identity
are asserted.

## Publication gate and limits

The final cumulative patch is published only with a matching frozen-tree
`ci_local.sh --fast` stamp. Its actual receipt is stored outside the tree it
authenticates in `LCCC-DELIVERY.json`, the snapshot ledger and
`artifacts/pr791-review/ci-fast.log` / `ci_local.pass`. Earlier UNGATED
checkpoints and interrupted/failed exploratory CI runs are not delivery proofs.

Full baseline/final oracle manifests, before/after assembly, failure logs,
workload checks and the compact checked evidence are retained. The compact
upstream record is `engineering/evidence/pr791-review-2026-10-11.json`;
reproduction commands and prioritized gaps are in the companion follow-up.

Only fast validation was authorized. No runtime-speed, PMU, full-package
build, kernel-boot, whole-codebase completion, or exhaustive cross-target
correctness claim is made. The remaining CFG guards and conservative
same-width Select restriction are explicit technical debt, not "10/10" work.
