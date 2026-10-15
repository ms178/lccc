# PR828 repair — work in progress, NOT delivery-ready

Baseline: d19565ad39bb37aa53dc1d179fe0fcbc4246dbdc, main 3ff36e85.
The debug-assertion build reproduced the reported StrictRecip LICM panic.
The new args-writer unit test also failed on VecStoreI32x4Pair before repair.
After canonical-write fallbacks in BOTH loop memory models, all 12 LICM tests
pass (fastbuild/O1, exactly two Cargo jobs). Atomic loads/fences are treated as
ordering barriers; dest_ptr writes record derived-global/unknown alias effects.
These focused tests are not a whole-patch acceptance result.

The parallel CI rewrite and GCC fixture contracts in this checkpoint remain
DRAFT, not fully validated. The original accepted S24 is preserved separately.
Still required: repaired debug C reproductions, CI contract/mutation tests,
local bounded parallel scheduling, pinned oracles, Callgrind comparisons,
immutable complete FAST, formatting/clippy, final upstream refresh and gated
publication. No native 14700KF performance or hosted parallel speedup is claimed.

Focused debug C checks now PASS for both previously crashing programs, with
CCC_VERIFY_IR=abort and CCC_VALIDATE_SSA=1. GCC 16.2 and repaired LCCC both
report `strict reciprocal pack: 140 cases OK` and `0 0 0 1 2`, respectively.
CI parity: 225 commands / 22 contracts; all 71 mutation tests pass after
repairing the stale hard-coded `test`-job mutation. Build/unit RUSTFLAGS are
now shared, avoiding an accidental normal-library rebuild on hosted CI.

## Additional independent PR824 handoff (newly reproduced, not historical CI)

The independent handoff's two nbody false passes reproduce on this candidate:
15 Python tests, 2 failures before repair, all 15 pass after repair. VMOVDDUP
uses disjoint eight-byte read intervals per 128-bit lane (not a fabricated
contiguous width); unary IMUL invalidates its implicit full-register address
facts while retaining its read-only source. Two/three-operand forms retain
positive controls. Masked census inputs now require a separate proof.
The test and production repairs are separate commits. Rust review reproductions
are test-only additions at this checkpoint; their baseline run is in progress.
No new FAST acceptance or performance conclusion is claimed.

## Architectural-effect repairs

Four new Rust red controls failed before repair (upper zeros, unknown CFG
implicit readers, masked overwrite coverage, masked copy admission). After
repair, the complete targeted peephole suite passes: 1,128 passed, 0 failed,
3 ignored. The memory boundary matrix covers 15/16, 31/32, 47/48/63/64 with
XMM/YMM/ZMM reads and masked/unmasked writes. Unknown state-image extents are
opaque, never assigned a small guessed size. Masked copies are not admitted
as unconditional relations. Read extent and guaranteed overwrite extent are
separate functions; covering-store endpoint arithmetic uses i64.

All 12 external low-XMM deadness calls were migrated to the owned-window
whole-write-region proof; the existing dead_code move consumer is the 13th
reviewed destructive call site. Its first conservative draft overestimated writes
of OTHER registers and failed three existing positive folding controls. The
refinement derives the possible upper write for the actual queried register;
all three positive controls now pass without weakening their assertions.
Unknown CFG returns no proof. The low-only deletion helper is private.

Consumer obligations (production callers, not unit-test queries):
- dead_code / reg_move_dead_after: delete one admitted move, including zeros.
- vector_copy / dead-copy: whole move; FMA bracket and result retargeting:
  union of removed/redirected writes in owned window, queried at its end.
- local_patterns (four callers): load/producer and consumed temporary window.
- memory_fold (three callers): deleted memory feeder and rewritten consumer.
- store_forwarding (two callers): deleted load and rewritten consumer window.
All still require their original alias, pin, CFG, opcode and operand controls.
These are scoped proofs, not a claim that every optimization is maximal.
Native whole-pipeline byte observation, current FAST and performance remain
pending at this checkpoint. No historical FAST result is relabeled.

Native full-pipeline upper-zero witness PASS: four legacy overwrites, each
executed before and after the complete peephole pipeline (eight executables).
The test observes stored upper bytes, not a literal optimizer opcode. Fixture
assembly/linking uses GCC without C optimization; this is a semantic execution
witness on this VM, not a GCC performance comparison or Raptor Lake timing.

F8 fixture selection was still present: all four literal g++ calls now use
CXX. The adversarial PATH test failed both EH fixtures before repair and passes
afterward, with four actual selected-driver subprocess invocations. Runtime
and ELF observations in that policy test are mocked explicitly; the production
exit-code/output/relocation assertions are untouched. Real targeted EH and the
strict linker suite remain required; this mock result does not substitute for
them. This does not select a performance-comparison compiler.

## CI architecture and resource policy

Normal compiler/linker executables are built once and commit/SHA256 checked by
four independent consumer lanes (corpus, frontend, codegen, backend). A separate
assertion-enabled unit/corpus lane does not overwrite the normal artifact.
Clippy/MSRV remain independent; the stable `Test Suite` aggregator requires
all eight lanes to succeed, including rejecting skipped/cancelled lanes.
Only the two explicitly unwanted research jobs were removed. The offline
research publisher/tests remain available; no live CE merge gate was added.
The normal build/unit flags match; debug unit/corpus settings match.

Local parallelism is deliberately bounded to two *audited isolated tooling*
checks. Unknown gates, every Cargo command, installer and compiler-driven gate
are exclusive barriers. This avoids unsafe shared temporary outputs and nested
-j2 oversubscription on the 2-vCPU VM. Four scheduler tests prove actual overlap
by synchronization, bounded queues, serial reference mode and exact failure
aggregation. Five DAG tests exercise real wiring and missing-dependency/hash
mutants. Focused CI Python set: 16 passed; parity: 225 commands / 22 contracts;
workflow shell validation: four workflow files passed. No measured hosted wall
clock speedup is claimed. Historical script comments remain historical; the
new local Cargo policy is exactly two jobs, with normal cache settings shared.

## Reported corpus cases: oracle contracts repaired, not suppressed

Targeted run of the 22 reported cases: 22 pass, no skip-compare/run. This is
21 real GCC16.2 differentials plus the existing LCCC-only casts policy (whose
misnamed .c.env had previously been ignored). An added, separate alias-policy
fixture also passes. Scope is targeted, not the prohibited full/slow VM corpus.

Repairs: missing libm inputs; code16 non-PIE link contract; explicit -O2 in the
two v3 cross-PR fixtures (rather than comparing differing compiler defaults);
GCC-admitted canonical CPU names plus separately preserved LCCC alias checks;
C23-legal VA_OPT token pasting; two formerly duplicate number() definitions
renamed and BOTH exercised (aligned and unaligned); nested static x87 helpers
moved to file scope; complete array type before sizeof, retaining its earlier
incomplete tentative declaration. The cast fixture deliberately asserts an
LCCC x87 conversion policy, including non-portable infinity conversion: it is
not advertised as defined-C differential evidence. GCC16.2 exits 4 on that
policy matrix; LCCC exits 0. No numerical tolerance or output checks were relaxed.
A new test rejects silently ignored .c.env/.c.flags sidecar names.

Real EH linker checks: 2 pass / 0 fail / 0 warn / 0 skip. Actual nbody shape
check passes with two proved mass-reading operands. Full strict linker/FAST
remain pending. The standalone tooling suite run completed 478 tests OK;
subsequent sidecar/fixture edits are not covered by that earlier run.

## Performance and remaining acceptance blocker

Three alternating pairs per workload, independently built current main,
matched inputs/flags/header provider and fixed simulator geometry:

| Workload | candidate/main simulated Ir |
|---|---:|
| nbody | 0.9984867491 |
| fannkuch | 1.0427579198 |
| histogram | 0.9247984220 |
| libm_round_family | 0.9940012138 |
| lz4_match_extend | 0.9882261054 |

All pair outputs and the GCC16.2 outputs match. Within each workload all three
ratios were identical. Fannkuch adds 52,924 Ir, 8,660 data reads and 5,040 data
writes in the first pair. Skip/trace controls identify the changed indexed-IV
policy and register/spill shape, not a runtime speedup. Two narrower SIB-rule
exemptions were built and measured: narrow symbol indices +8.234%, multiple
symbol streams +6.615% versus main; BOTH REJECTED AND REVERTED. Do not promote
an intuitive profitability rule which actually makes the measured case worse.
The remaining +4.276% fannkuch regression is NOT resolved or user-approved.
It remains a delivery blocker under the supplied acceptance plan.

GCC16.2 (cg162), Clang23.1.0 (cclang2310) and latest ICX (cicxlatest) all
accepted the same bounded fannkuch v3 flags with empty diagnostic arrays,
compiled/executed successfully via the existing Godbolt script, and returned
`228 / Pfannkuchen(7) = 16`. Optional research only, not a live CI gate or
native performance evidence. Raw responses and Callgrind counters retained.

Further LICM audit: derived global writes through inline-asm outputs were
independently red, then fixed by resolving EVERY recorded write pointer, not
only the ordinary-store arm. A real Pattern E consumer test keeps the store
observed at byte 48 by a ZMM read, with a YMM non-overlap deletion control.
Whole library after these changes: 4,729 pass / 0 fail / 8 ignored, warnings
fatal, fastbuild O1/j2. This is still not immutable FAST/hosted acceptance.

Current assertion-enabled compiler (after all correctness repairs) also passes
all 22 reported cases with CCC_VERIFY_IR=abort and CCC_VALIDATE_SSA=1.
The same 21 GCC differentials / one explicit x87-policy distinction applies.
Latest upstream fetched again: main remains 3ff36e85; rebase is up to date.
Next step is immutable FAST; no full/slow local run has been authorized or run.


FAST attempt 1 was stopped after concrete prerequisite failures, not declared
complete: the freshly restored isolated GCC16 lacked its 32-bit libgcc archive.
Installed lib32gcc-16-dev and lib32stdc++-16-dev from the same 16.2.0-3 package
family. The real GCC16 -m32 executable and the previously failing complete
memory-value-contracts gate now pass. No source/controller was edited while
that attempt ran. Its aborted log is retained, not overwritten or called green.


## Follow-up after S36: fannkuch performance repair (2026-10-15)

S36's immutable FAST finished **251/0/6**, including **4729/0/8 ignored**
library tests, strict linker **303/0/0 warnings/0 skips**, fmt, MSRV and
strict Clippy. Exact tree `58da848a48e7ece3c32397f1885a67f22b85525d`;
before/after identities differ only in timestamp. This is historical proof
for S36, NOT acceptance of the following changed tree.

The user explicitly declined a fannkuch trade-off and requested further work.
Matched main/candidate ablations now isolate the difference: disabling IVSR
produces **byte-identical ELF files and identical counters** in both arms;
disabling SLP or vectorization alone does not eliminate the regression.
The global SIB-rule bypass is rejected: it worsened fannkuch to +5.089% Ir.

Assembly exposed duplicate live addresses for the same symbol after fresh
GlobalAddr materialization; these increased register pressure and spill
traffic. The repair reuses an existing GlobalAddr only when its definition
**dominates the preheader insertion point**, using the already-valid CFG.
No CFG edges change. Otherwise the existing fresh-materialization fallback
remains. The x86 SIB exception is restricted to narrow symbol indices needing
widening and possessing such an available base. Pointer-base stencil cases,
native-width indices, unavailable symbol bases and non-x86 materialization
policy remain unchanged. No optimization family or correctness guard is removed.

Two test-only controls were executed red on `cf44d45`: expected recurrence
missing (0 versus 1), and duplicate bases (2 versus 1). Four adjacent controls
passed there. After production commit `9113c2d`, the **entire library passes
4735/0/8 ignored**, with fatal warnings, fastbuild O1 and -j2. Controls include
a non-dominating diamond definition, fresh-base fallback, native-width SIB,
ordinary pointer-base SIB and unchanged non-x86 materialization policy.

Three alternating candidate/current-main Callgrind pairs per workload:

| Workload | Candidate/main simulated Ir (all three pairs) |
|---|---:|
| nbody | 0.9984867378772616 |
| fannkuch | **0.9930050720902538** |
| histogram | 0.9247983141061648 |
| libm_round_family | 0.994001213762196 |
| lz4_match_extend | 0.9882261053655662 |

Thus the bounded fannkuch case is **-0.6995% versus main**, rather than
+4.2758%. All outputs matched GCC16. The measured immutable candidate is
`987a2a7` (full identity in `symbol-reuse-performance.json`); production
integration adds tests and clarifies comments. A fresh production-compiler
ELF-equivalence check, updated debug corpus and new immutable FAST are still
required. Raw counters, negative screens, source patches and experimental
history are retained separately; rejected bypasses are NOT in this patch.

No native i7-14700KF/PMU result and no hosted full/slow result is claimed.
The larger effect-descriptor follow-up remains separate. This checkpoint is
UNGATED until the changed tree completes its own acceptance.


## New upstream integration: PR830 / main 7f9eb225 (2026-10-15)

After S37 was safely published, the required fetch found new main
`7f9eb225def23bda9370d3349304b9b785554fea`. Its optimizer parent `cdae9c6`
has tree `0b31d5b26dcb36f4288566250aef1c313e7995e7`, EXACTLY the protected
PR814 validated source tree; `d508ea1` additionally makes benchmark-reference
failures and empty case filters fail closed. The latter controller is kept
byte-identical to new main.

The attempted linear replay conflicted and was aborted without losing S37.
The cumulative follow-up was then squash-rebased in dedicated branch
`audit/pr828-main-7f9`, retaining `audit/pr828`, its safety ref, saved patch,
S37 session bundle and experimental-history bundle. This avoids replaying
rejected experiments into the production history. No conflicting file was
blanket-selected without reviewing main-only functions and assertions.

Main-only improvements retained explicitly:
- Copy-only min/max element identity, including Cast/cycle/foreign-load
  adversarial controls, integrated into the widened-counter lowering;
- pointer-ring address equivalence SEPARATE from representable byte-counter
  and remainder bounds; narrow-intermediate refusals and the new U64 quotient
  guard are both retained;
- opaque assembler directives fail closed, pinned SIB scan boundaries remain
  barriers, and upstream debug-window/RIP-relative/raw-byte tests are kept;
- the old coarse upper-register approximation is superseded by the reviewed
  three-region model. Its upstream test now queries the WHOLE write footprint
  rather than conflating low-XMM liveness with architectural upper lanes.

Integration initially duplicated the IVSR gate registration. The existing
parity checker correctly rejected it; the duplicate was removed (one original
registration and rationale remain). CI/local parity is now green with
**225 commands and 22 invocation contracts**. This is not full compiler acceptance.

The independently built immutable new-main compiler has SHA-256
`d8a51efe042c10df0cac8cb02a15778d9e1e77c52a7e18b0249de2b601c340ab`.
ALL earlier S36/S37 FAST, library and performance results remain attached to
their original trees and OLD main basis. The merged tree requires fresh
library/debug/FAST tests and fresh alternating comparisons to 7f9eb225.
Hosted full/slow and native target-hardware measurements remain pending.

S38 merged-tree library validation has now completed: **4742 passed, 0 failed,
8 ignored** (tree `f0b384a2fbca32c103a4f6e7c56bb5ac72717ca8`, exact project
fatal-warning flags, fastbuild O1, -j2). This includes seven retained upstream
controls in addition to the S37 suite. Fresh compiler/debug/performance and
complete FAST acceptance on the new basis are still pending.


S39 fresh normal/debug compilers are now built. The new debug compiler passes
all **22 reported cases** with `CCC_VERIFY_IR=abort CCC_VALIDATE_SSA=1`
(21 GCC16 differential cases, one explicit x87 policy case).

Fresh three-pair alternating comparisons to **7f9eb225** now supersede the
old-base table for CURRENT-main claims:

| Workload | Candidate/new-main simulated Ir (all three pairs) |
|---|---:|
| nbody | 1.0 |
| fannkuch | **0.9523241050414963** |
| histogram | 1.0 |
| libm_round_family | 1.0 |
| lz4_match_extend | 1.0 |

The bounded fannkuch regression is therefore repaired: **-4.7676% Ir against
new main**, without the declined trade-off. The other improvements are now
UPSTREAM and are not attributed again to this follow-up. Three additional
alternating input controls also match GCC16 and the new IR/SSA-checking debug
compiler: N=6 ratio **0.9821267948769306**, N=8 **0.9651220606414428**,
N=9 **0.8596563555049819**, each identical across three pairs. No inference
about unmeasured native i7-14700KF cycles or larger inputs follows.

Raw counters, outputs, adverse counters and exact compiler identities are
retained. `main-7f9-performance.json` is a compact tracked receipt; the raw
current-main evidence is external. The source remained unchanged through
these builds and measurements. The changed tree still needs its OWN full
immutable FAST/fmt/MSRV/strict-Clippy/linker acceptance; S36's green FAST
must not be relabeled. Hosted full/slow remains pending.
