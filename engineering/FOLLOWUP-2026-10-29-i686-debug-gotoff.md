# Continuation: i686 DWARF GOTOFF and completed fast-CI attempt

## Evidence and scope

The S11 tree (`9192e41`, base `ed928575365e1c56a03e040d53ab5dc5af83a860`)
completed `ci_local.sh --fast`: **487 passed, 4 failed, 8 skipped**. The log is
`/home/user/ci-local-fast-final.log`. This is not a green run and does not
validate the later tree. `cargo-test`, rustfmt, MSRV and strict clippy passed.

* `pr824-boundaries`: qemu-aarch64 was missing at gate time; after installing
  QEMU the same targeted test passed 369/369, 0 skips.
* `gnu-retain-sections`: mold was missing at gate time; after installing the
  digest-verified upstream mold 2.42.1 binary, the targeted gate passed.
* `regression-corpus-link`: 1270/1271 passed. The GCC 16.2 i386 libgcc's
  `bid_binarydecimal.o` contains 1198 `R_386_GOTOFF` relocations in
  `.debug_loclists`; the LCCC i686 linker rejected them. This was reproduced
  with a standalone i386 assembler fixture. The targeted float128 compile/link
  passed after the change below.
* `linker-suite`: 324 passed, 1 failed. The private binutils 2.47 bfd was
  built without Debian's x86_64 multiarch default search directory and its
  bare `ld -lc` failed. This was an oracle environment failure. With actual
  Debian GNU ld 2.44 on PATH, both isolated default-search-path tests passed;
  this is not evidence of passing the full suite with that oracle, nor a claim
  that the tool is 2.47. GCC 16.2 and the tested system 32-bit executable
  remained functional with this environment.

## Fix and safety boundary

Pass the already selected output GOT base to i686 debug relocation handling.
Resolve `R_386_GOTOFF` using wrapping `S + A - GOT` only if a retained
allocated section contains a `R_386_GOTPC`: that is the tested proof that the
output needs a GOT anchor, and covers the actual GCC libgcc failure. Both
executable and shared/PIE output paths are covered. Other GOTOFF cases remain
explicit errors, not guessed values. In particular, GNU ld can accept a
GOTOFF present only in a debug section without an exported GOT symbol; its
implicit anchor behavior needs further investigation. GOT use induced only by
PLT, other GOT relocation types, linker scripts, discarded sections and
unusual symbol bindings need their own oracle fixtures before broadening this
precondition.

`tests/linker/i686_debug_gotoff.py` uses GNU ld and LCCC on the identical
assembler object. It checks each output against its own ELF symbol addresses,
so differing section layouts are not compared as identical bytes. It covers
local/global symbols, positive/negative addends, static, PIE and DSO, and
checks that debug-only GOTOFF remains explicitly rejected by LCCC instead of
silently producing a possibly wrong value. The test is registered in the
existing linker-suite CI gate. Before the fix the focused fixture failed as
expected; after the fix all supported fixtures and the original float128
case passed. The LCCC build used fastbuild, Rust `-O1`, `-j2`, with active swap.

## Still required

Run the entire focused regression-corpus-link and linker-suite gates and then a
fresh full `ci_local.sh --fast` on this exact final tree. Record the actual GNU
ld version and search paths: a deliberate toolchain change is not a CI pass
under the previous toolchain. Resolve any further failures, and do not issue a
full-green claim until a clean completed run is tied to the committed tree.
Run real Arch makepkg and real i7-14700KF P/E PMU tests; neither was available
in this VM. Continue the documented `#include_next` provenance repair. The
existing 63/63 workload-derived corpus run is correctness-only, not measured
hardware performance.

### Focused gate continuation

The complete `regression-corpus-link` gate passed on S12: 1271/1271, no skips.
The first full linker-suite attempt exposed a test-registration signature error
(the registry supplies both `args` and `oracles`); the new regression itself
had previously passed standalone. The signature was corrected and the test
then passed through the registry with `--strict`. The full suite must be rerun
on this corrected tree. This is not a full fast-CI pass.
