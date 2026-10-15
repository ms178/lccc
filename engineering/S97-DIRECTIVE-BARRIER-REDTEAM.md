# S97: directive transparency and unmodeled block exits

## Reproductions, not assumed C-source reachability

Two independently GCC16.2-linked native fixtures establish wrong results:

* The actual isolated identical-block pass redirects a branch across different
  executable alignment fills (CLC 0xf8 versus STC 0xf9). Original exits0;
  pre-fix isolated transformation exits13. The full pipeline passed this
  fixture BEFORE the fix; this is not a demonstrated full C compiler failure.
* The full production peephole pipeline crosses `.cfi_remember_state; .byte
  0x66,0x0f,0xef,0xc0` (PXOR XMM0,XMM0), retaining an invalid XMM2=XMM0
  relation. It replaces the later XMM2 read with XMM0. Original exits0;
  transformed assembly exits1. This proves a full peephole-pipeline defect,
  not an established route through C frontend inline-assembly quarantine.

The first prefix-classifier-only repair was INSUFFICIENT. It stopped scanning
at the effectful directive, but identical prefixes still merged: both absent
fall-through successors were represented by None, incorrectly conflating an
unknown exit with a proved terminal exit. Its failing native run is preserved.

## Alternatives and surgical choice

1. Do not repair the already-correct shared directive parser, or add another
   ad-hoc semicolon scanner. Reuse its exact-token, quote/comment-aware inert
   directive proof in each private predicate.
2. Do not broaden vector-copy transparency to every shared inert directive.
   Keep its existing narrower family allowlist, guarded by the shared proof.
3. Do not special-case the witnessed fill bytes or assume a missing successor
   is unreachable. Require a known explicit exit or modeled fall-through
   before merging a block prefix. This also covers truncation at ordinary
   CFI metadata or `.byte`, not only alignment directives.
4. Do not disable the optimization globally. Existing default-fill alignment,
   CFI-transparent vector-copy, and legitimate merging controls remain active.

Five new tests cover the two native witnesses, positive/negative directive
classifiers, and unknown-exit prefixes ending at alignment, `.byte`, or CFI.
Native tests follow the existing Linux x86-64 GCC harness and use leased
TempFiles. PATH selected GCC16.2.0-3 here; they do not require a version-specific
executable name on every CI runner. Both native tests run in the normal suite.

## Executed evidence

* RED: original helper tests, isolated wrong-result assembly, full-pipeline
  vector wrong-result assembly, and insufficient prefix-only repair.
* GREEN: fastbuild/-O1/-j2 Rust library suite **4841 passed,0 failed,9 ignored**;
  all five new tests included, no added ignores. Rustfmt/diff check pass.
* Fresh Compiler Explorer: **12/12 expected controls** for original and saved
  bad assembly under GCC16.2.0, Clang23.1.0 and the `cicxlatest` channel.
  The bad assemblies intentionally return1/13; those are negative controls,
  not compiler defects in the reference vendors. Labels are namespaced only
  in embedded-assembly CE inputs to avoid compiler debug-label collisions.
* IMPORTANT: actual `cicxlatest` reports **2024.0.0 (2024.0.0.20231017)**,
  `__INTEL_LLVM_COMPILER=20240000`. A moving alias is NOT proof of Intel's
  newest available release. The request for genuinely latest ICX is not yet
  satisfied by that channel. First harness attempt's rejected -no-pie flag
  and label collisions are preserved separately; no warnings were suppressed.

Raw evidence: `engineering/evidence/redteam97/`. This checkpoint is interim:
final current-tree FAST/strict Clippy and fresh runtime-performance acceptance
remain separate gates. No speedup, zero-defect guarantee, or external approval
is claimed. Existing historical 45-case/zstd timings do not transfer here.

## Follow-up validation and freshness

Eight fresh Valgrind3.27.1 Memcheck/Callgrind runs cover both original and saved
bad native assemblies. Expected exits all match; Memcheck finds zero errors
in BOTH correct and wrong-result programs. Memory cleanliness does not prove
semantics. These tiny controls are not a performance corpus or speedup claim.

Fresh `git fetch origin main` still identifies84a62201. The refreshed package
repo main is2e62d34d5970fea9dc20ea1f940497ab058ca42d; zstd remains1.5.7/pkgrel16
with unchanged source/patch hashes. The recipe now explicitly requests LLVM24
and multi-stage LTO/PGO/BOLT; comparing plain compiler builds is not claiming
reproduction of that complete packaging pipeline. Recipe and hashes retained.
CE's C registry lists no ICX release newer than2024.0.0; the requirement for a
newer actual Intel release cannot be silently met by relabeling that binary.

Raw Callgrind streams are stored as .callgrind.gz; decompress before annotation.
