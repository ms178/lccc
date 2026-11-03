# PR1065: main 662b771c0 integration and diagnostic phase audit

This is an integration checkpoint, **not an all-13/zero-kernel-warning or
production-readiness certificate**. The main tip checked through GitHub on
2026-11-03 was 662b771c0ad1ff11249a05cd79da065febaebfda.

## Repairs and engineering decisions

* Retain both branches' CI gates and the authenticated ICX steps outside the
  lane planner. Restore the local FAST mirrors rather than deleting hosted
  coverage or relaxing the parity validator. The resulting inventory contains
  524 commands and 27 registered invocation contracts; workflow shell validation
  covers 13 workflow files and two rules. The focused parity suite passed 84
  tests, including mutations removing the restored mirrors.
* Keep consumer width/signedness in VLA range proofs: Copy is bit-preserving,
  not a typed conversion. Cast inversion and affine proofs must stay inside
  both source and consumer domains. An extracted implementation mutation test
  with minimal IR adapters gave old=8 pass/3 fail, fixed=11 pass. This is not
  represented as a complete internal-IR integration harness; the real crate
  separately includes the three regression tests and passes its unit suite.
* Distinguish libcpp diagnostics from C-frontend message diagnostics. GCC 16.2
  preserves expanded message directives under C `-E`, without issuing the
  frontend note or malformed-message warning until parsing. Assembly expands
  operands (including counter builtins) once, discards the message directives,
  and never issues those frontend diagnostics. Assembly preprocessing still
  reports ordinary/libcpp warnings and respects promotion and suppression.
* Preserve message whitespace and UTF-8 inside literal tokens. Retained
  directives must be skipped by the lexer at whole-keyword boundaries.
  Malformed-message warning snapshots use the scoped WarningConfig rather
  than the final global state.
* Macro aliases of `_Pragma` have a second operator-rescan path. The broader
  frontend-preprocessor gate caught protected keyword bytes escaping that
  rescan after the first marker-normalization pass. Normalize newly emitted
  metadata on the rescan path, without re-expanding operands or deleting raw
  control-byte literal payloads. The ordinary no-operator fast path does not
  gain another scan. Add a direct unit test and extend the existing 25
  producer/consumer valid-message roundtrips with the alias path.

The pragma matrix has 414 four-target/mode checks against GCC 16.2, including
25 valid-message and 25 self-reference cross-compiler producer/consumer
roundtrips and 16 assembly checks. The VLA matrix has 7,488 four-target GCC
comparisons. These are non-kernel oracle fixtures: GCC does not compile kernel
target code. Existing ignores are not newly disabled tests.

## Validation receipts

The first complete merged cohort passed the fastbuild binaries, 414 pragma
checks, 7,488 VLA checks, 7,800 library unit tests (zero failures, ten existing
ignores), and all-target Clippy with warnings denied. Audit census and
workflow/parity checks also passed without increasing a census baseline.

The broader frontend-preprocessor gate then exposed the alias-rescan bug
above. After repair, the rebuilt binaries, frontend-preprocessor gate and
extended 414-check matrix pass. The final follow-up passed 7,801 library tests (zero failures, ten existing
ignores), all-target Clippy with warnings denied, 696 designated-init and
1,747 frame checks, Microsoft scalar/wide/record/variadic interoperability,
and ten cross-target narrow-count executions (checksum 4177401467814966825).
The LCCC-only Linux 6.18.58 boot-only build passed at 31,024 bytes with 1,744
bytes headroom, 24 ISA-clean objects, and a byte-identical GNU-linker oracle.
Its setup.bin SHA256 is cb29bdd28392582f7ef76dc10f075c63cd9725647885cf872a67e31b0a06d98f;
this is not a full bzImage or a system-QEMU boot and is not a size gain. Logs are under `/home/user/checkpoints/rescan-*` and
`current-*`; no generated compiler, kernel objects or datasets are added here.

## Explicit outstanding acceptance criteria

The currently pushed hosted run is not all green: its FAST shards report the
missing-mirror failures repaired locally, and its full defconfig kernel job
has not yet completed. New source requires a new hosted run.

A separate measured diagnostic presentation discrepancy remains: for
`#warning first`, `#pragma GCC warning "second"`, `#warning third`, GCC prints
first/second/third, while the driver's separate ordinary-warning and pragma
loops print first/third/second. The composition tests sort their comparisons,
so their passing result establishes severity/location/composition, **not
source-order equivalence**. This is a confirmed open defect, not a claimed fix.
A robust repair must preserve execution order through includes, #line and
queued macro effects; sorting by filename/line is not sufficient.

The six remaining warning families, outstanding option semantics, zero
kernel diagnostics, current full link/strict objtool/system-QEMU boot and
current measured runtime/code-size improvements remain required. Earlier
boot-only size or userspace checks are not evidence of those results.

## Boot host-generator prerequisite repair

The first repeat boot-only attempt failed because mkcpustr includes capflags.c
but that generated table was absent. The boot probe now uses Linux's own
mkcapflags.sh and real feature headers, refreshes stale tables/string data,
and compiles only the host mkcpustr tool with HOSTCC. Target code remains
LCCC-only. A shared helper is covered by the existing kernel provenance gate:
missing inputs, unchanged no-op, each of four isolated freshness dependencies,
and generator/compiler/executable failure paths. Failed generation cannot
publish a partial header; temporary headers live beside their destination so
publication uses a same-filesystem rename even if OUT is elsewhere. These
mocked shell tests complement, rather than replace, the actual boot build.

## Ordered warning/pragma stream follow-up

The ordering defect described above is now repaired. A compact execution-order
index stream refers to existing warning/pragma payloads and warning snapshots;
it does not duplicate messages or sort filenames/locations. Message operand
effects are drained before recording that message, preserving date-time and
nested-pragma order. All five driver paths use the same dispatcher and retain
C-frontend phase filtering. This addresses warnings/pragmas, not the separate
hard preprocessing-error paths.

Validation: 594 four-target mode checks against GCC16.2, frontend-preprocessor
rules, 7,803 library tests (zero failures, ten existing ignores), all-target
warnings-denied Clippy and unchanged audit census. The old executable fails
the new order regression first/third/second versus GCC first/second/third.
The 180 new checks preserve ordering under promotion/suppression, notes,
repeated includes and #line, date-time operands and nested _Pragma operands.
The nested GCC note can have a `cc1:` prefix without a location; the comparison
includes it rather than silently dropping that record.

Hosted 7d78b07a7 passed all listed checks except the kernel check. Its real
failure is now confirmed: assembly hash comments such as `#Setup` were
incorrectly reported as unknown CPP directives and promoted by Kbuild's
-Werror after assembly warning reporting was enabled. That classification
needs its own repair; the unsupported warning/options diagnostics also remain.

## Kernel assembler hash-comment regression repair

Assembler-with-cpp now expands and preserves unknown hash-prefixed lines as
assembler comments, matching measured GCC16.2 behavior (including a macro in
the first word). It does not categorize them as C unknown-directive warnings.
Actual #warning/#error processing remains active, including unconditional
#error under -w. C's existing unknown-directive policy is unchanged.

Validation passed: 602 four-target/mode GCC checks, frontend-preprocessor
rules, 7,804 library tests/zero failures/ten existing ignores, all-target
warnings-denied Clippy and unchanged audit census. The pre-fix executable
fails the non-kernel hash-comment witness under -Werror. The real Linux
6.18.58 canonical `arch/x86/realmode/` target now compiles, links and emits
realmode.bin/relocations using LCCC/lccc-ld for target code; only host tools
use GCC16.2. The boot-only gate remains 31,024 bytes with 24 ISA-clean objects
and GNU-linker byte equality. No current whole-kernel boot or size gain is
inferred from that result.

A first targeted invocation was invalid: verify_cpu.S is an include fragment,
not a standalone TU, and directly requesting a realmode child object bypassed
the parent build context. The corrected canonical directory target, not those
invalid invocations, is the integration receipt.

System-QEMU prerequisites are now available from the authenticated hosted
Debian bookworm bundle, transported as six Git blobs. All part hashes, archive
hash and package hashes verified before extraction without installation hooks.
QEMU 7.2.22 (Debian security build 1:7.2+dfsg-7+deb12u18+b3) passed actual
PC/TCG firmware initialization, QMP version query and clean shutdown. This is
host-VM readiness, not a kernel boot. The separate custom GLib attempt built
successfully but had 300 passing/four environmental test failures (missing MIME
database/update-desktop-database); it is not claimed green and is not used by
this packaged QEMU. Source compiler readiness still requires the remaining
warning/options work and a fresh complete hosted kernel/QEMU result.
