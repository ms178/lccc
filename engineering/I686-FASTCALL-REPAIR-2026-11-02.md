# i386 typed fastcall repair — validated cohort, not final readiness

## Scope and engineering decisions

The original standalone non-kernel volatile-pointer probe returned 1 at `-O0`
and crashed at `-O2`/`-Os`. Inspection found **two independent defects**:
fastcall was absent from function types, and the fastcall emitter reloaded and
removed a target spill that its inherited no-op hook had never created.

* Add `CallingConvention::I686Fastcall` to the existing canonical function-type
  mechanism. Preserve it through parser declarators, typedefs, compatible types,
  indirect calls, inherited definitions and nested definitions. Do not introduce
  a second parser name table as the source of truth.
* Capture an indirect target before argument staging. Implement this slot only
  in the fastcall path: overriding the generic i386 spill hook would also change
  cdecl/regparm layout. Include its four bytes in the ordinary outgoing alignment
  budget. With dynamic vector alignment, retain the separate saved-ESP link and
  restore the exact spill address before removing it.
* Fix the additional sret defect exposed by the broader test: a nonvariadic
  fastcall record-returning callee must pop its complete compacted argument area,
  not the cdecl hidden-pointer four-byte rule. Variadic record calls instead
  retain GNU cdecl's four-byte hidden-pointer cleanup.
* Preserve nested-function chains in dedicated incoming/outgoing frame homes.
  Capture before parameter materialization and reload after argument staging.
  GNU i386 fastcall uses **EAX** for its chain, leaving ECX/EDX for user arguments;
  cdecl uses ECX. Generate the corresponding trampoline MOV opcode from final IR
  function metadata. This follows GCC 16.2 `ix86_static_chain` in the previously
  verified `i386.cc` blob `4dfb90acf3f6a763f621f2eedb212241d276549c`.

### GCC / Clang are not interchangeable ABI oracles

Actual GCC 16.2 and Clang 23.1.4 assembly/runtime probes exposed two differences:

1. For fastcall record results, GCC puts the hidden pointer in ECX; Clang puts
   it on the stack. Both can emit `ret $12` for the four-int example, so the
   return instruction alone cannot validate the ABI.
2. With a fourth 32-byte vector and a following stack int, GCC pops 36 argument
   bytes and leaves the trailing alignment padding caller-owned. Clang pops 64.

Using one unqualified cross-vendor expectation crashed the new adversarial
probe. The repaired backend implements the **GNU i386 ABI**, not a blend of
these incompatible layouts. Cross-TU tests retain record-result coverage in
both GCC/LCCC directions. Clang/LCCC pairs exercise their common scalar,
wide-integer, floating and by-value-record argument ABI; the source explicitly
identifies the non-interoperable record-result case. The vector assembly oracle
uses GNU cleanup and independently verifies 32-byte entry alignment and
register/stack argument values. No warning suppression was added.

## Executed validation (a53 cohort before the subsequent main refresh)

All binaries were built with `fastbuild`, Rust 1.99.1, two jobs and active 4 GiB
swap. GCC/Clang compiled **only standalone non-kernel ABI fixtures**.

| Gate | Result |
|---|---|
| All library tests | 7,643 pass, 0 fail, 10 existing ignores; 15.28 s execution |
| All fastbuild binaries | PASS, 42.60 s |
| Kernel-codegen object/runtime contracts | **281 PASS**, formerly 251 |
| All-target Clippy, `-D warnings` | PASS, 2m46s |
| Expanded DWARF 2/3/4/5 / four targets / GCC 16.2 round-trip | PASS |
| Authentic saved-command usercopy and strict configured objtool | PASS |

The added 30 contracts comprise twelve ordinary/nested i386 executions, six
AVX2 dynamic-realignment executions and twelve cross-compiler TU pairs. The
optimization matrix is `-O0/-O2/-Os`; local i386 fixtures also request both frame
pointer modes. Assembly callees independently verify ECX/EDX/stack placement and
entry alignment. Coverage includes zero/one/four arguments, volatile globals,
local pointers, returned pointers, conditionals, member and array callbacks,
inline-attributed callback parameters, inherited declarations, wide/floating
results, record results, variadics, captured nested callbacks, sibling/deeper
chains and repeated calls. This is bounded evidence, not exhaustive proof.

Binary SHA256 identities:

* lccc: `0c258b6b025b4c40512bba68adeebf095bf5978efa17802a10f124c49a22a8a2`
* lccc-i686: `52e6adc3ff8302d035c33586bf9af14007885ac74fe5361b30d3146540b5032b`
* lccc-ld: `9e25a7bdb64c9295634a971fae61b8fae41cc59f6b98d16717c5f91b0bf6b4b0`

Detailed local logs: `/home/user/fastcall-{all-units,all-bins,contracts,clippy,dwarf,usercopy}.log`.

## Hosted kernel evidence: earlier pushed source, not this repair

At source `86b0b1a57bf0a435c5eb128fe383611342919a3a`, all 26 listed PR checks
passed, including authenticated ICX 2026.2. Kernel run **41499635926**, job
**125464215626**, completed successfully at **2026-11-02T19:30:53Z**:

* boot-first check: `_end=31040`, **1728 bytes** below the 32768-byte limit;
* full defconfig bzImage/link step: 18:32:34–19:30:31Z;
* full-image QEMU step: 19:30:31–19:30:39Z, success.

The job and boot annotation API receipts are retained in
`engineering/measurements/2026-11-02-hosted-kernel-86b0b1a57-{job,annotations}.json`.
The artifact download redirected to an unavailable Azure host; its detailed
contents and warning count were **not inspected**. This is not evidence that
all kernel diagnostics are zero, nor a boot of the newer merged-main source or
this local repair.

## Open findings — do not label the PR ready

* The supplementary x86-64 nested-capture witness discovered here was
  subsequently reproduced with SysV too and repaired by modeling trampoline
  scratch clobbers. The follow-up also repairs Microsoft leaf XMM-save frame
  allocation and gates the original witness on all four targets. See
  `NESTED-TRAMPOLINE-AND-MS-FRAME-REPAIR-2026-11-02.md` for the before/after
  evidence, historical reproducer identity, 332-contract cohort and exact
  validation revision boundaries. This is no longer listed as an open defect.
* Nested regparm-3 requires GCC's different ESI/stack-chain entry mechanism and
  is not established by this repair. Typed regparm pointer propagation remains
  a separate audit.
* CS-prefix/all extended-register transfers, mitigation option forwarding and
  register-only paths remain open.
* All inventoried warning/option families, zero-diagnostic full kernel,
  current-source local full link/objtool/QEMU, and current-head hosted CI are
  still required. Existing bounded code-size/runtime measurements do not become
  kernel-wide performance guarantees through this ABI repair.
