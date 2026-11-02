# Follow-up: AArch64 address contracts and GPR-only population count

## Scope and publication

This round implements **A14-14 and A14-19**, preserving the earlier A14-29/30
repairs. Upstream was fetched and rebased again after checkpoint S20; main
remained `e047aa1eb2ae59b3943a64ab7ec5e5dddb89a3ff`. S18, S19 and S20 save the
validated implementation stages, source archive, cumulative patch and history.
They were explicitly UNGATED checkpoints, not final certification.
The final unchanged-tree fast-CI receipt and exact delivered identity belong in
`LCCC-DELIVERY.json` and the snapshot ledger, rather than a self-referential
hash in this tracked report. No full/slow local CI is requested or represented.

This is a bounded correctness/ISA repair, not compiler-wide perfection, a proof
of optimal code, or a claim of superiority over other compilers. A newly
reproduced, pre-existing shared-symbol visibility problem is **A14-35**.
The A14 queue now has **35 entries, 30 open**.

## A14-14: address encoding, relocation and final placement

* A shared signed-21 field encoder rejects overflow before masking. ADR accepts
  byte offsets -1048576 through 1048575, including unaligned byte offsets.
* ADR/ADRP require exactly two operands and a 64-bit general/zero destination;
  W registers, SP, FP registers and surplus operands are rejected.
* Static, dynamic-executable and shared-library relocation paths propagate
  errors. Generated PLT ADRPs use the same checked field contract.
* Address relocation sites and symbol indexes are checked against their input
  sections before computing output addresses. This prevents huge offsets from
  wrapping into headers and short fields from overwriting neighboring input
  sections. The output patcher separately checks its four-byte destination and
  leaves bytes unchanged on failure. NONE relocations need no site resolution.
* Same-section local ADRP remains an ELF relocation: the final section page
  offset is not known to the assembler. Its section-symbol relocation retains
  the target offset **and addend**. The old fold both assumed page placement and
  discarded the addend. A linked fixture deliberately straddles a final page.
* Direct shared ADR rejects preemptible, unresolved/dynamic and absolute
  symbols. Hidden/protected declarations in other translation units constrain
  binding regardless of object order; same-named local symbols do not enter
  that global-name set. Unknown shared relocations now fail, rather than warn
  and silently continue.

### Evidence and alternatives

The original compiler accepted the invalid operand/range controls; its failing
regression receipt is preserved. The permanent contract test covers every
register, boundary residues, invalid widths/classes/arity, i64 extremes, three
ELF modes, relocation addend/range mutation, final-page placement, visibility
and malformed sites. Rust tests additionally exhaust all **2,097,152** valid
field values and check opcode/register preservation and error atomicity.
Those Rust tests are part of the final fast gate, not assumed executed merely
because the targeted Python test passed.

The first fast-gate attempt caught a missing GOT/TLS variant set in the new
input-site check. The implementation was expanded to all six supported
ADR/ADRP/GOT/TLS address-relocation kinds; the test was retained unchanged.
That failed attempt is preserved separately and is not a delivery certificate.

**544 actual ADR machine words match Clang 23.1.0 exactly.** This used CE's
native AArch64 channel `armv8-cclang2310`, binary compilation/linking, and returned
opcode words—not an `-S` assembler listing masquerading as an encoding oracle.
The request, response, input assembly and equal byte hashes are preserved in
`engineering/evidence/2026-10-14-address-repair/oracle/`.
CE hides the reserved `_start` function from its disassembly; the successful
oracle uses the ordinary `adr_probe` name and an explicit entry point.

Truncation is not a fallback. Globally disabling local resolution would add
unnecessary relocations; only page-dependent ADRP folding was removed. A
field-only fix was insufficient: it would miss linker overflow, malformed
input sites, same-section page placement and shared-symbol preemption.
The wrapping S+A/P arithmetic deliberately represents 64-bit address algebra;
range validation is performed on the resulting signed displacement before any
field truncation. This is not a blanket hardening claim for every ELF relocation.

## A14-19: legal, fixed-work popcount

With `-mgeneral-regs-only`, 32-/64-bit popcount uses a **12-instruction SWAR
core**, no helper call, no data-dependent branch and no FP/SIMD registers:
2-bit counts, 4-bit counts, masked byte counts, multiply by repeated 0x01, then
extract the high byte. Per-byte counts are at most eight, and the total at most
64, so no carry contaminates that final byte.

Only x0 (the existing unary accumulator) and x1 (reserved emitter scratch,
excluded from the allocated SSA register pools) are clobbered. W operations
exclude unspecified upper argument bits. The normal four-instruction NEON
path is retained: restricting an ISA is a correctness requirement, not evidence
that software popcount is faster than the unrestricted implementation.
A helper call adds ABI/call overhead and a runtime dependency; a bit loop adds
data-dependent work. Neither is needed for these fixed-width operations.

The final ISA check examines compiler-generated instruction operand contexts,
not arbitrary register-shaped text. It distinguishes branch/address/literal
symbols, relocation immediates, directives and comments from registers. Explicit
inline assembly is exempt, consistently with the existing x86 guard: this is a
compiler code-generation contract, **not a sandbox or general assembler parser**.
Other compiler-generated FP operations under the flag receive a hard diagnostic.

### Runtime and oracle coverage

`tests/regression/check_arm_popcount_contract.py --run` requires QEMU and tests
O0/O1/O2/O3/Os, both default and restricted modes (ten executions). The default
fast test automatically executes QEMU when available; hosted CI installs
qemu-user. It still performs compile/link/structural checks without a cross
runtime and reports the actual runtime count, never a fabricated execution.

Each execution checks zero/all-ones, all 64 single bits and complements,
all 65,536 low-16-bit inputs and their high-half placements, and 4,096 deterministic
random values against a nibble-table reference in a separate translation unit.
Controls include signed 32/64-bit inputs, signed/unsigned char/short promotions,
a ten-argument live-pressure function, and an assembly tail-call that deliberately
leaves garbage in the upper 32 argument bits. The old compiler fails the ISA
regression. Native and local GAS **2.44** assembled popcount .text matched for
both paths; that is explicitly supplementary context, not a GAS 2.47 result.

Requested CE comparisons were performed: **GCC 16.2**, **Clang 23.1**, and the
latest available **ICX alias** compiled and executed independent portable
popcount/SWAR controls with `-mno-popcnt`. The alias's effective version is
**2024.0.0**, not a claim about Intel's newest released compiler. Clang 23.1
AArch64 GPR-only SWAR also has a 12-instruction core. Raw responses, sources,
assembly and hashes are under `engineering/evidence/2026-10-14-popcount-repair/`.
No unavailable ARM GCC 16.2, GAS/bfd 2.47, lld 23.1 version check or mold 2.42.1
result is silently substituted. This round did not build mold.

## Native regression/performance screen

Valgrind **3.24.0**, three paired Callgrind rounds, six bounded workload-derived
kernels: gzip CRC32, zlib-ng Adler32, Expat SipHash, SQLite varint, Linux find-bit,
and glibc memcmp. Equal-length executable paths, identical flags, native and
instrumented output checks, raw counters, commands and hashes are retained.
The measured compiler pair is explicitly recorded: saved f837 versus S19.
Later changes harden AArch64 linking/guards, not these x86-64 code-generation paths.

Every paired executable was **byte-identical** and every after/before simulated
Ir ratio was **1.000000** in all three rounds. Arithmetic/geometric means,
best and worst are all **1.000000**: **0 wins, 0 regressions, 6 ties**. There were
no >1%, >2% or >5% regressions in this screen. All six candidate kernels also
completed Memcheck with **zero reported errors**. The archive retains 36 raw
Callgrind profiles and their manifests; per-kernel memory-check logs are separate.

These are native x86-64 regression controls and simulated counters—not AArch64
cycle measurements, wall-clock speedups, or physical i7-14700KF PMU evidence.
QEMU **10.0.13** provides AArch64 execution correctness, not physical-target
performance. No fresh full Linux boot/glibc build or full SQLite workload is
claimed. Earlier full gzip/zlib-ng/Expat evidence remains historical and retained.

## Outstanding work

1. **A14-35:** AArch64 `.hidden` definitions are still emitted as GLOBAL DEFAULT
   in .dynsym. The ADR binding guard now consults input visibility, but complete
   merged visibility, export filtering, protected binding, GOT/PLT and dynamic
   symbol serialization need a coherent follow-up. A minimal reproducer and
   readelf output are retained; this is not hidden behind a green gate.
2. **A14-17/15/13:** RISC-V shared unknown relocations/expression algebra and
   frontend vector-size type semantics remain open. No RISC-V source fix is
   claimed in this round.
3. **A14-20:** PGO lock/stdio-flush ordering is unchanged. A14-34's CI environment
   leak is also unchanged; explicit two-job/debug/incremental settings and active
   6 GiB swap are the resource workaround, not a source repair.
4. Continue the remaining evidence/architecture/performance queue with actual
   reproducers, unchanged-tree fast CI, and physical-target measurements before
   making target-runtime claims.
