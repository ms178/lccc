# Session 59 — AArch64 GCC torture, GAS 2.47, and oracle expansion

Date: 2026-08-22

Upstream base: `78209d17` (latest `ms178/lccc` after the prior patch was merged).
All compiler builds in this session used only `scripts/build_lccc_fast.sh`.

## Toolchain and method

* GCC torture source revision: `gcc-mirror/gcc@ad82e41f1b997dedd2cf227cb410afd8763d89c8`.
* Corpus: `gcc/testsuite/gcc.c-torture/execute`.
* Assembler: self-built `GNU assembler 2.47.20260726`, configured only for
  `aarch64-linux-gnu`. The new harness refuses any assembler whose version is
  not 2.47.
* Execution oracle: AArch64 GCC 14.2 binaries and LCCC binaries under QEMU
  10.0.11, comparing exit status, stdout, and stderr.
* Code-generation oracles: Compiler Explorer ARM64 GCC 16.1, ARM64 GCC trunk,
  and Clang 22.1 with an automatic `--target=aarch64-linux-gnu` flag.

`aarch64_execute_suite.py` parses unconditional DejaGNU `dg-options` and
`dg-additional-options`, uses the source directory as an include directory,
keeps generated data off the 1 GiB `/tmp` tmpfs, emits atomic JSON, and records
all tool versions. A first implementation exposed and then fixed a race in
`codegen_oracle.py`: concurrent function requests used the same temporary
assembly filename and deleted one another's output. Local assembly now uses a
unique `mkstemp` file per request.

## Correctness defects found and fixed

### 1. Narrow unsigned compare constants mis-selected as CMN

GCC torture `20000528-1.c` exposed this sequence:

```c
unsigned long source = (unsigned long)-2;
unsigned short narrowed = source;
if (narrowed != (unsigned short)-2) abort();
```

LCCC treated the `I16(-2)` storage variant as full-width `-2` and emitted:

```asm
cmn wN, #2
```

That compares against `0xfffffffe`, while the U16 value is `0x0000fffe`.
The fix normalizes every integer constant according to the comparison type:
truncate to the semantic width, then sign- or zero-extend to the W/X machine
width. CMP/CMN immediate selection is performed only after normalization, and
the register fallback materializes the normalized bit pattern too.

Focused U8/U16/I16 edge tests and two Rust unit tests cover both valid and
invalid CMN cases. The GCC reproducer now passes.

### 2. Tail recursion illegally reused a frame whose address escaped

GCC torture `20000412-2.c` passes `&local` to the recursive tail call. Turning
that call into a backedge changes distinct recursive automatic objects into one
reused object; the result changed from 1 to 0 (and register allocation could
turn it into a null dereference).

TCE now computes stack-derived pointer provenance through Alloca, DynAlloca,
GEP, Copy, Cast, Phi, Select, and pointer BinOp forms. A self-tail-call argument
that can point into the current frame rejects TCE. Scalar locals do not disable
TCE. A focused IR test covers a GEP-derived stack pointer, and a runtime
regression verifies frame identity at recursion depth 100.

### 3. Store-only aggregate forwarding deleted copies after pointer promotion

GCC torture `20001026-1.c` constructs a three-long struct via a marching pointer
and copies it through an sret temporary into an outer struct. Loop pointer
promotion intentionally made the store pointer unprovable to
`aggregate_copy_forward`; the pass then interpreted the temporary as having no
writes and deleted both memcpys while leaving stores in the old temporary.

A store-only forwarding candidate must now contain at least one write that
`pointer_paths` can attribute to the source root. Unknown/marching-pointer
writes fail closed. A focused IR test uses pointer arithmetic invisible to the
path engine, and an independent runtime struct-copy regression covers the full
sret/copy chain.

### 4. Unassigned 32-bit Select emitted mixed-width CSEL

GCC torture `20020615-1.c` produced:

```asm
csel x0, w23, w28, ne
```

which GAS 2.47 correctly rejects. Both ordinary and fused Select fallback paths
hardcoded `x0` even when the arms were W registers. They now choose `w0` for
32-bit selects and `x0` for 64-bit selects. The testcase assembles and executes
correctly with GAS 2.47.

## GCC torture results

Initial 100-test screening before fixes:

```text
93 PASS, 5 FAIL, 2 SKIP
```

After the first three fixes and DejaGNU option handling:

```text
first 100: 96 PASS, 2 nested-function frontend FAIL, 2 oracle-option cases resolved
```

Expanded first-500 run after all fixes:

```text
454 PASS, 46 FAIL
  30 execute mismatches
  11 frontend compile failures
   5 link failures
   0 GAS 2.47 failures
```

This is not presented as full GCC-suite conformance. The remaining failures are
a prioritized defect corpus. Importantly, all generated assembly in the tested
set that reached assembly was accepted by GAS 2.47 after the CSEL fix.

## GCC/Clang code quality

The architecture-aware oracle now compares the maintained benchmark pattern
against GCC 16.1, GCC trunk, and Clang 22.1. All four compilers choose the same
core instructions for conditional increment and narrow high-constant compare.
However, complete LCCC leaf functions remain much larger because parameters and
results are moved into callee-saved homes:

| Function | LCCC | Best GCC/Clang | Gap |
|---|---:|---:|---:|
| `conditional_increment` | 13 | 3 | 4.33x instructions |
| `narrow_high_constant` | 14 | 4 | 3.50x instructions |
| `select_pressure` | 31 | 6 | 5.17x instructions |

Therefore the defensible claim is instruction-selection parity for the central
idioms, **not** overall superiority. ABI-aware AArch64 leaf register allocation
is the largest remaining code-quality gap; unsafe register-pool swapping is not
an acceptable substitute for scratch-aware fixed-register allocation.

## Performance and linker evidence

The new `aarch64_select_patterns` benchmark was added to the maintained corpus.
On the pinned VM's x86 host build, output matched GCC and the paired median was:

```text
LCCC/GCC = 1.3939 [1.3563, 1.4153] (9 randomized rounds)
```

This is an honest regression signal, not an ARM hardware claim. Earlier in the
same controlled run, existing workload kernels all matched GCC; ratios were
1.071 for gzip CRC, 1.604 for zlib-ng Adler-32, and 2.060 for Expat XML scan.
These gaps remain optimization targets.

The fastbuild LCCC driver and its integrated `lccc-ld` passed the complete
linker suite:

```text
160 PASS, 0 FAIL, 0 WARN, 0 SKIP
```

A relative `--lccc` path bug in the linker harness was also fixed: tests change
to temporary working directories, so the driver path is now canonicalized once
at startup.

## Final validation

```text
Rust library tests : 1089 pass, 6 ignored, 0 fail
Correctness suite  : 50/50 pass
x86 regression     : 436 pass, 0 fail
Progressive suite  : 22 pass, 0 fail, SQLite corpus absent
ARM regression     : 268 pass, 51 fail, 68 skip
Linker suite       : 160 pass, 0 fail
GCC torture (500)  : 454 pass, 46 fail
```

The ARM total improved from the prior session's 257/56/67 baseline while adding
new regression coverage. Remaining failures are explicitly retained rather
than hidden behind an aggregate score.
