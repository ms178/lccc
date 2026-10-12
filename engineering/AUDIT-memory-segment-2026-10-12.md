# Segment diagnostics and vector-store effects audit

## Evidence status

This document records the preceding correction, now merged upstream. Its
red/green and runtime statements are recorded observations of that earlier
session, not independently rerun results of the current quality follow-up.
The retained segment-store verification archive contains logs, source deltas,
profiles and hashes, but not the producing compiler executables. Hash integrity
and inspection of those records are not compiler reproduction. The older
review-diagnostics payload has the further provenance limitations stated below.
No compiler, compile-based tests or performance measurements are run locally
for the current follow-up; new Rust tests await hosted validation.

## Scope and upstream

This correction is based on main `52064a7ad563744e2653faca4b50ec697e304e5a`,
after PRs #803 and #804. Its source tree already contains the preceding delivery;
none of that merged work is reverted or replayed as a duplicate patch.
This is a focused follow-up, not certification of the whole compiler or all
architecture backends. Full hosted acceptance remains necessary.

## Hosted failure reproduced

Both merges failed the named-space conversion-diagnostics gate. Public job
metadata identifies the step; full hosted job logs were not accessible.
The original gate was reproduced locally with GCC 13 and GCC 14: GCC 13 rejects
`pos_member_of_seg_obj.c` and `pos_member_plus_seg.c` as nonconstant initializers,
where GCC 14 and LCCC accept both. The workflow already installs gcc-14, but
the old gate invokes unversioned gcc. Do not change these valid expectations.

The gate now selects GCC >= 14 (CI pins gcc-14), reports its identity, requires
a semantic positive control from both compilers, checks the rejection reason
on negative fixtures, and refuses abnormal compiler exits. An explicitly
unsupported or missing oracle is an error, not a skip or fallback.
Python gate tests cover selection, positive-control failures, wrong-reason
rejections, accidental acceptance, crashes and exit 77.

## Unsupported-target diagnostics

ARM/RISC-V load and store uses of named spaces reached backend panics; unused
named declarations and syntax-only uses could be silently accepted. This is a
front-end diagnostic gap, not evidence that cross-target segment loads emitted
wrong code. Parsing now rejects preprocessed named-space tokens on non-x86
machines before lowering. No token scan is added to the x86 hot path. Real-driver
and thread-local parser tests cover FS/GS loads, stores, declaration-only use,
x86/i686 positives and generic/inactive-branch controls. The backend's defensive
panic remains in place for malformed internal IR.

## PR #803 destructive-path findings

Three additional regressions reproduce against the original scripts without
network, real sudo, real mounts or deletion of user data:

- A symlink in an ancestor bypasses the chroot's final-component symlink check.
- A symlinked KDIR ancestor hides a mount from the literal mountinfo comparison.
- A failed debootstrap can leave a mount behind; the old retry loop checks only
  before the first attempt and issues more deletion commands afterwards.

Chroot paths now reject symlink ancestors and compare physical HOME/repository
identities. Package and kernel mount queries use canonical absolute identities without
rewriting relative package-root overrides. A stubbed clone control catches
incorrect sparse prefixes; the relative-alias mount negative remains enforced.
Every bootstrap attempt rechecks mounts immediately before deleting. Tests use
non-executing sudo stubs, mount-table fixtures and forced preflight failure.
These are administrative safeguards, not protection against a hostile concurrent
process changing symlinks/mounts between check and use.

## CI checker correction

The pipeline/SIGPIPE checker mistook a logical `|| grep -q pattern file` for a
pipe in the new target gate. The expression was not a pipeline. The checker now
distinguishes `||` from `|`, also recognizes stderr pipes (`|&`), and retains
real-pipe detection after an OR list. Three new self-test expectations fail
before this correction; all 17 pass afterwards. No test is suppressed.

## Review findings: accepted, qualified, rejected

**F1 — accepted root defect.** VecStoreI32x4Pair was classified pure. Baseline
IR tests reproduce incorrect pure/const inference, sinking a load across a
store, stale temporary forwarding, and refusal to fold a valid store address.
The classification test also fails. Reintroducing only this opcode into the
producer predicate reproduces the five failures; restoring the fix passes.

**DCE qualification.** Direct destination-less stores already survive DCE:
`is_dce_root` independently retains instructions with no result. Thus direct
store deletion is not a demonstrated current DCE bug. Keep explicit survival
coverage, including address-fold -> dest_ptr=None -> DCE. Callee call deletion
remains a hazard of incorrect interprocedural purity; do not confuse the two.

All 13 vector-store families have classification, folding/idempotence, DCE,
purity, forwarding and sinking coverage. The constant-array callee proof already
checks args-writing effects independently; the new negative control preserves
that defense. Loop unrolling already refuses args-writing/vector/read families;
no speculative relaxation is justified.

**F3 — accepted missing coverage.** The formerly absent
`tests/regression/bb_slp_pure_store_fn.c` is now a real, self-checking runtime
regression, registered in the memory-value gate. Pair and wide stores check
both values and neighbour sentinels across repeated calls. It passes on the
baseline too: this is a new coverage gate, not a claimed end-to-end red miscompile.
Controls disable SLP, IPCP/purity together, and vector-address folding separately.

**F4 — split semantics, not instruction shape.** `produces_vector_value` no
longer lists stores. `vector_result_width` excludes stores; `vector_home_width`
retains backend SIMD-home metadata, including historical store source homes.
A pair store's source occupies 16 bytes but writes only 8. Backend metadata
consumers retain their existing homing behavior; optimization predicates no
longer use home shape as a purity exemption. `reads_pointer_arg` explicitly
excludes args-writing stores. Opcode predicates cannot test `dest.is_some()`;
instruction consumers must do so separately.

The suggested four-reader invariant is incomplete: 23 memory readers remain
classified, including 14 modern pure vector-load families. Pure means no
observable side effects, not read-none. Existing interprocedural reader tests
retain this distinction. No unsafe widening of alias proofs is introduced.

**F2/F7 — retention is not reproduction.** `review-diagnostics.json` is kept
byte-for-byte and added to the independently checked retention manifest.
`review-diagnostics-provenance.json` anchors the retained upstream commit/blob
and labels its historical scope. Its recorded producer head is not available
in fetched main history; its recorded base is. Embedded old compiler hashes
are not newly verified compiler identities. New validation records belong to
the external corrective evidence archive, with source/tool identities.

**F5 — retain narrow alias-oracle policy.** The declared-alias self-check and
execution remain mandatory; the narrow GCC comparison exception is not a
license to assume unrelated declarations always alias or never alias.

**F6 — no cosmetic history splitting.** The corrective work stays cumulative.
Misleading store/result comments and the aggregate-copy counter description
are corrected. Historical copy observations are explicitly not current proof.

## Performance and acceptance boundaries

Pair-store address folding is now reachable and verified in IR. A backend can
already fold an equivalent address: fewer IR operations alone do not establish
fewer machine instructions or a runtime speedup. Measure emitted code, not the
predicate change. Intel SDM 325383-093US and AMD APM 24594 revision 3.38 describe
MOVQ's memory width; register home width must not be substituted for that width.

The earlier local acceptance recipe was fastbuild O1, no LTO, incremental, jobs=2,
active swap; `ci_local.sh --fast` includes formatting, Clippy, Rust/Python tests
and runtime gates. The five slow gates remain hosted-only. Native performance
claims require controlled measurements; Callgrind Ir is not cycles. Neither
all-14 structural wins, a 14700KF result, independent engineering approval,
10/10 correctness nor final-head hosted green follows from these fixes.
