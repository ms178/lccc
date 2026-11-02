# Assembly symbols and unknown callers — focused validation, not final acceptance

## Confirmed defects and repairs

* DCE treated a standalone x86 `stac`/`clac` instruction as a reference to the
  same-named C helper. This retained unused outlined helpers and their
  unbalanced UACCESS alternatives. The shared scanner now distinguishes bare
  fixed-zero-operand instructions using the same mnemonic inventory as the
  assembler validator. Operands, labels, directives, quoted text and uncertain
  forms retain whole-identifier references. The proof applies only to x86 ELF
  machines 3/62. Macro/include/escape context is collected across ALL blobs,
  not separately per function. This is not a general GAS macro interpreter.
* Initial conservative escape handling still retained the real helpers because
  EXPORT_SYMBOL uses quoted `\0` data. A new regression distinguishes quoted
  data escapes from unquoted assembler escapes and malformed quoting. The real
  kernel check, not just synthetic scanner tests, caught this missed case.
* Callback propagation omitted top-level assembly from its caller inventory.
  A valid standalone asm entry supplies a different callback than the C caller;
  the old compiler returns the wrong result at O0/O2/Os. GCC 16.2.0 and Clang
  23.1.4 pass all three. Top-level assembly now participates in shared exposure
  facts; declared symbol operands are normalized consistently with DCE.
* IPCP separately overlooked assembly, aliases and retention/export metadata.
  Changing the asm caller's scalar argument exposed another wrong result even
  after callback propagation was repaired. Shared module facts now exclude
  those bodies from in-place parameter specialization. They include global
  address/initializer references, explicit asm operands, aliases, ifunc
  resolvers, constructors/destructors, used/retain/export/asm-label/symver
  metadata. A positive control still specializes a genuinely closed local
  caller. Call-site constant evaluation remains available independently.

## Alternatives and limits

Deleting all assembly mentions would drop real address references. A global
text substitution of opcode names cannot distinguish `.quad stac` from an
instruction. Suppressing objtool findings would preserve the defective output.
None of these alternatives was used. Unknown syntax remains conservative;
explicit operands and retention annotations remain necessary for symbols
constructed opaquely by assembler machinery. No universal parser or optimality
claim is made. The shared exposure facts replace divergent analyses rather
than adding another independent address-taken definition.

## Executed evidence

* Full fastbuild lib unit suite: **7635 PASS, 10 existing ignores, 0 FAIL**,
  50.08 seconds. Includes scanner, DCE, callback/inliner, IPCP and all backend
  units. Unit harness uses fastbuild opt-level/debug 0; actual compiler bins
  use normal optimized fastbuild. Four GiB swap active; builds serialized.
* Normal optimized fastbuild bins: PASS, 57.23 seconds.
* Expanded contracts: **251 PASS**, covering x86-64, i686, AArch64 and RV64,
  O0/O2/Os, native GCC 16.2.0 and Clang 23.1.4. The new asm callback fixture
  also varies scalar arguments and exercises a public alias of a static body.
* All-target Clippy with `-D warnings`: PASS, 33.99 seconds.
* Authentic saved Linux 6.18.58 `lib/usercopy.o` compiler invocation, LCCC only:
  strict UACCESS **before FAIL (2 findings), after PASS**. Configured checks
  including IBT/ORC/retpoline/rethunk/static-call/UACCESS and prefix checks
  **PASS** with `--Werror --dry-run --link`. An initial harness omitted the
  required IBT `--link` flag and failed option validation; the corrected full
  invocation is retained in the measurement receipt. No validator was disabled.
* `.text` **1183 -> 1151 bytes** (-32); reported section sum **1547 -> 1481**
  (-66), including dead alternative metadata. `check_zeroed_user` stays 591
  bytes. This is object code-size evidence, not another runtime speedup claim.
* Authenticated ICX 2026.2 job **125466069491** on pushed **86b0b1a57** passed
  the explicit cc1 return-thunk forwarding and all remaining vendor steps.
  It does NOT yet validate this newer working-tree assembly/IPCP cohort.

Raw object hashes, compiler hash, source hashes, exact argv, selected validator
flags and diagnostics are in
`measurements/2026-11-02-asm-symbol-usercopy.json`.

## Still open

Current main advanced to 27ae9f4c6 and must be integrated and retested. The
new cohort needs current-head hosted CI. The known i686/non-x86 mitigation
option defects, CS-prefix/register-only forwarding, remaining warning families
and optimization options, full strict kernel build and current QEMU acceptance
remain open. Kernel compilation still emits inventoried unsupported-option
warnings; a single-object validator pass is not zero-warning kernel acceptance.

## Subsequent main integration

Main 27ae9f4c6 was integrated as 0582ec6f8 with a three-way merge, preserving
60 branch-only commits and their existing validation/checkpoint identities
rather than rewriting that history. The integrated tree passes **7643 lib
units / 10 existing ignores**, normal optimized fastbuild, **251 contracts**,
the expanded DWARF 2/3/4/5 all-target/dialect/GCC-16.2 round-trip gate,
all-target Clippy with warnings denied, and both strict usercopy validators.
The kernel source invocation still emits the already inventoried unsupported
option warnings. Those are not counted as fixed by these validator passes.

A further standalone, non-kernel i686 probe has confirmed another open ABI
failure: a volatile fastcall function pointer called with four integer
arguments returns a wrong result at O0 and segfaults at O2/Os. GCC 16.2 and
Clang 23.1.4 i386 executables pass. Assembly inspection shows that fastcall
is lost from the pointer's function type (the caller emits cdecl arguments).
The fastcall-specific indirect-call emitter also invokes the inherited no-op
pointer-spill hook while its later reload/cleanup assumes a four-byte spill;
that path requires a separate regression after typed ABI propagation is fixed.
Neither defect is claimed fixed. The current checkpoint is not a production
acceptance delivery and does not claim every callable ABI path is correct.
