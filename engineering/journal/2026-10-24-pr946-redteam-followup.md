# PR #946 integrated red-team follow-up — 2026-10-24

## Scope and evidence boundary

This journal covers follow-up review on the integrated tree, initially merge
`5bee77aed` (PR #946 plus current main). It does **not** convert PR #946's
isolated-head results into combined-tree evidence, and it does not claim a
whole-SDM review. The official SDM version 093 volume-2 transport exposed only
front matter/TOC in this sandbox; instruction conclusions below are bounded to
the named families and executable oracle/model tests.

No LCCC binary or Rust toolchain is installed locally. GCC 12.2.0, GNU
binutils 2.40, and native x86-64 execution are local oracle/model evidence only.
The authoritative LCCC pass-after, warning-free fastbuild, pinned compiler, and
differential results must come from the hosted swap-enabled lanes.

## 1. Declared `_Float128` alignment is not carrier alignment

The `_Float128`/`Decimal128` lowering used
`struct_align: Some(16.max(declared))`. That conflated the local parameter
object's requested alignment with the SysV carrier's ABI alignment after the
register files were exhausted.

A GCC 12.2.0 oracle with an `aligned(64)` `_Float128` parameter, exhausted GP
and SSE argument registers, and one preceding stack integer showed:

* incoming carrier offset: **16**;
* following stack scalar offset: **32**;
* local treatment is optimization/context dependent: GCC's standalone `-O2`
  path realigns to 64, but an `-O0` fixture path addressed the carrier in place
  and was not always 64-byte aligned.

An initial fixture assertion incorrectly required 64-byte local alignment and
failed GCC-against-GCC at both optimization levels in the integrated caller
context. That assertion was removed; it was not a stable ABI contract. The fix
keeps `struct_align` at the type's natural ABI alignment and retains stronger
declared alignment only in `param_align`. The decisive `aligned_f128` interop
check expects **83**, so mixed callers/callees detect the old offset-64 carrier
misclassification without relying on local realignment. Complete callee/driver
builds and all four `-O0`/`-O2` caller/callee combinations passed locally with
GCC; warning-enabled builds used `-Wall -Wextra -Werror`, with only
pre-existing GCC ABI-change notes emitted.

## 2. Caller-owned incoming homes do not impose a local-alloca tail lifetime

Generic sibling-call analysis rejected every used alloca. A checked incoming
MEMORY home is different: it belongs to the caller's argument area and remains
alive after the wrapper's epilogue. Ordinary and dynamic local allocas still
reject sibling calls.

The exception names only `incoming_byval_slots`, whose mapping already checks
carrier size/alignment/offset and parameter-alloca identity. The x86 sibling
call lowering independently refuses every call with stack-passed arguments,
so this change does not enable outgoing/incoming stack-area overlap. The new
`tail_big` fixture passes `&incoming_big` to a register-only external consumer.
Runtime coverage crosses LCCC/GCC callee and driver objects, optimization and
frame-pointer modes, and enabled/disabled incoming mapping. The assembly gate
requires a sibling jump only in the enabled path and an ordinary call in the
private-home control. Existing `CCC_NO_SIBCALL` and
`CCC_NO_INCOMING_BYVAL_HOME` remain the A/B controls; no new knob was added.

## 3. ZMM uniform shifts; word forms disproved by execution

The existing dword/qword proof also covers unmasked ZMM0–15 forms: `VMOVD`
zeroes every count bit above bit 31, `VMOVQ` supplies the whole qword count,
and the uniform instruction reads the low 64 count bits. The pass's existing
register-bank, alias, barrier, and deleted-write liveness checks remain in
force. Registers 16–31 and all decorated/masked forms remain outside the
matcher.

Native original-vs-folded assembly testing on the local Intel Xeon VM passed:

* **163,840** exact AVX2 dword/qword comparisons;
* **245,760** exact comparisons in the extended run (the same baseline plus
  ZMM dword/qword cases).

Each operation used all boundary counts and 16,384 deterministic samples,
including counts above the lane width and with nonzero upper count bits. GNU
assembler/objdump 2.40 accepted and decoded every original and folded ZMM
encoding. These are semantic model results, not LCCC pass-after or performance
results.

A proposed AVX-512BW word extension was **rejected**, not shipped. Per-lane
`VPSRAVW` consumes 16 count bits while uniform `VPSRAW` consumes 64. Native
execution found count `0x85012179f8de0000` (low 16 bits zero) where the
original returned input lane `48128`, the candidate fold returned `65535`, and
the integer reference required `48128`. A negative matcher test now records
this family boundary. This is why native execution, rather than a superficial
broadcast-count argument, is required for each lane width.

## Remaining validation and performance boundary

The integrated tree still requires warning-free swap-enabled fastbuild,
rustfmt/clippy, LCCC execution of both fixtures, GCC 16.2 / Clang 23.1.3 / ICX
2026.2 comparisons, and the differential lanes. No representative ZMM timing
has yet been measured, so no runtime-speedup claim is made for the new ZMM
matcher coverage. Hosted CPU identity and any future timing must be reported
with the measurement; these local semantics do not establish whole-program or
cross-microarchitecture superiority.
