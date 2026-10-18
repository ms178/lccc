# glibc 2.44+r25 LCCC bring-up — 2026-10-18

## Reproducible configuration

- LCCC base: `aec2a2751a70a59a7cca2b5714c5ad0371f1aa4d` (then session commits)
- glibc source: `afd131806b25715a7617ab757db43f0bb610acbf`
  (`2.44+r25+gafd131806b`)
- source archive SHA-256:
  `764ea4a5f2aebf9d9b61f07a50a1f54854055a913dd66a0f05e1d0a9b7f08bb2`
- archpkgbuilds revision: `4587e7caa08ada404dd603be8ae066ae8745220e`
- `ms178-glibc.patch` SHA-256:
  `52c9953f8e3ec710d73308464bd98bbe752895d83c6b17201235edcb1c974aeb`
- binutils: pinned 2.47.20260726 build; each invoked program was checked by
  `tools/linker/oracle_versions.sh`
- compiler build: `scripts/build_lccc_fast.sh` (Rust `-O1`, `-j2`)
- 4 GiB swap was active throughout.

The workload runner now consumes the recipe's actual post-release commit.  The
old runner incorrectly extracted the 2.44 release tarball, against which the
post-release patch cannot apply with `--fuzz=0`.

## Validated fixes

### Ordinary-identifier shadowing

`fnmatch.c` declares local `struct scratch_buffer wstring` while a file-scope
enumerator named `wstring` is visible. Sema unconditionally preferred its enum
map and diagnosed the local object as `int`. Resolution now compares lexical
depths in the two internal maps, as required by C11 6.2.3.

Evidence:

- focused regression passes at `-O0/-O1/-O2/-O3`;
- the exact 52k-line preprocessed glibc `fnmatch.c` witness compiles;
- a resumed glibc build passed the former `posix/fnmatch.o` stop.

### `-ftls-model=initial-exec`

The driver previously warned and ignored this option. Local TLS in `-fPIC`
objects therefore used `R_X86_64_TPOFF32`; `lccc-ld` correctly refused those
relocations in `libc.so`. The option now reaches x86 codegen, selects GOTTPOFF,
and disables the direct Local-Exec memory fold.

Evidence:

- focused relocation gate observes `R_X86_64_GOTTPOFF` and no TPOFF32 for IE;
- the local-exec control still observes TPOFF32;
- after rebuilding affected objects, LCCC linked `libc.so.6` successfully.

### binary128 integer-zero argument

The glibc `cabsf128` template calls the binary128 path with an implicitly
converted integer zero. IR canonically carried `I64(0)`, but the SysV binary128
argument materializer accepted only `Zero` and `I128`, causing an internal
compiler panic. All integer-zero IR variants now produce 128 zero bits while
floating negative zero remains sign-preserving.

Evidence: focused regression and the exact glibc `math/cabsf128.c` command pass.

## Current build frontier (not a success claim)

The build reached **4410 object files**, linked `elf/ld.so`, linked `libc.so.6`,
and proceeded into libm/support. It is not yet a complete build/install. The
next deterministic stop is:

```text
sysdeps/ieee754/dbl-64/e_acosh.c
ccc: error: `1(%xmm11)' is not a valid base/index expression
```

This is an x86 address-selection defect: a scalar FP/XMM home escaped into an
integer base-address position. Preserve the exact compile command from the
`math/e_acosh.o` build log, reduce it before modifying allocation or memory
folding, and mutation-test the reproducer. Do not weaken the assembler's valid
rejection of an XMM base.

## Highest-priority follow-up

1. Reduce and fix the `e_acosh.c` XMM-as-address defect; rerun its exact command.
2. Resume `make -j2`, recording each new first failure. A clean build must then
   be rerun through `tests/workloads/glibc-2.44/build_lccc.sh` so its install and
   structural ABI gates produce the schema-2 manifest.
3. Run staged-loader smoke only after install; never execute a partially staged
   libc through the host loader.
4. Add a safe explicit interrupted-build resume mode. Current
   `GLIBC_REUSE_BUILD=1` intentionally accepts only a completed, identity-matched
   manifest; manual resume was used for diagnosis only.
5. The runner still needs a self-contained pin acquisition path for the
   archpkgbuilds patch, and its capability comment about IFUNC must be reconciled
   against current linker evidence before any multi-arch gate is lifted.
6. Only after correctness/build completion: run glibc tests, triage with
   `scripts/glibc_check_triage.py`, then collect deterministic Callgrind/icount
   evidence in this PMU-less VM. Hardware claims remain blocked pending the
   i7-14700KF environment.
