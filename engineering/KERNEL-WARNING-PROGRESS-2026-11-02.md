# Kernel compiler work — intermediate evidence, NOT readiness

This work continues PR #1065. The requested acceptance criteria remain all 13
inventoried warning families/options, full native kernel link and real objtool
validation, a kernel QEMU boot, green hosted CI, and measured correctness-backed
runtime/code-size gains. Those criteria are **not yet met**.

## Executed checks (before latest-main integration)

* Rebuilt compiler/native linker with the pinned Rust 1.99.1 fastbuild toolchain.
* Layout/unused differential suite: **2,940** non-kernel GCC 16.2 comparisons,
  covering code-generation and syntax-only behavior, headers, numeric levels,
  severity precedence, GNU statement-expression value uses and pragma scopes.
* Enum-conversion suite: **500** non-kernel GCC comparisons, including implicit
  arguments/returns/assignments, aggregate initialization, explicit casts,
  promoted arithmetic, statement expressions and ordinary-namespace shadowing.
  Enumerator diagnostic provenance is separate from its language-mandated C type
  and follows the existing scoped constant undo log. Completed-enum provenance
  uses shared ownership, not one copy of all variants per enumerator.
* Verbose assembly: **12** target/optimization combinations, positive/negative
  option ordering, function commentary and native ELF byte equality. Comments
  are added after optimization/validation, not used to alter code generation.
* Integrated existing warning gate: passes its original precedence suite,
  **60** pragma checks, **697** include-path checks, **636** date/time checks,
  plus the newly integrated suites above.
* Integrated linker gate: ELF32/64 RWX/fatal/suppression contracts, AArch64 and
  RISC-V shared plumbing and QEMU-user freestanding execution, and **16** strict
  discard-none ELF32/64 comparisons with GNU ld. Local absolute and FILE symbols
  are retained; stripping still wins regardless of discard-none ordering.
* Census: **262**, baseline **262**; no metric grew.
* Actual CachyMod Linux **6.18.58**, package commit
  `df9c917272d816361e0da61bb951b5732cb1c662`, stable source commit
  `2926d88bb7fc4608fd942453d929c39e4e5ec006`, **29** package patches:
  boot-only `_end=31088`, limit `32768`, headroom **1680** bytes; GNU-linker
  byte parity and all **24** boot ISA scans passed. Setup image SHA-256:
  `93bc0dc7e1ca24f59905028e74d21a64008d9d2a50cfd72b2dbc4db9b88caeff`.
  Kernel target translation units used LCCC; only host tools used GCC.
  Tag signature authentication was unavailable (`gpg-verified=no`), explicitly
  recorded in the preparation stamp. No signature-verification claim is made.

These results are fixture/gate evidence, not universal diagnostic parity. A
follow-up malformed negative numeric-option rejection is included; the final
binary rebuild and complete rerun must validate that follow-up too.

## Host environment repair (outside the repository)

Installed reviewed wheel-backed Bison/Flex/pkgconf, host bc 7.0.3, m4 and zstd
1.5.7. m4's unchanged suite has **109 passes, 21 legitimate platform skips, zero
failures** after fixing its glibc portability assumptions and escaping literal
braces in its old Perl copyright helper. Patches and source/hash receipts live
under `/home/user/kernel-host`. Matched elfutils 0.188 and OpenSSL 3.0.20 public
headers to the installed runtimes; a warning-clean host probe verified both and
checked the OpenSSL version number at runtime. zstd compression roundtrip passed.
`/home/user/toolchains/current-env.sh` supplies this host-only prefix separately
from the reviewed compiler cache. No fake libraries or validator shims were used.

## Open gates and scope

Still outstanding: designated-init, frame-larger-than, vla-larger-than,
implicit-fallthrough, format-overflow, format-truncation, missing-format-attribute,
restrict and stringop-truncation; the remaining inventoried compiler options;
latest-main integration and post-merge revalidation; complete unit/ABI gates;
full kernel link/objtool, kernel QEMU boot, refreshed hosted CI and measured gains.
Kernel preparation continues to report the unimplemented options; those warnings
have not been suppressed. The boot size above is not a performance improvement
claim or a replacement for the full default-config kernel acceptance gate.
