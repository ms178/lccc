# glibc 2.44+r25 LCCC bring-up — 2026-10-18

## Reproducible configuration

- LCCC base actually used for the final validated series:
  `59f061d21dc73155e2bc0af814c8942dac8cc5f9`; the five commits
  `532669b7..20078a5f` are merged by `d8f79a6b` (PR #868)
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
and disables the direct Local-Exec memory fold.  This is a compilation-wide
(per-object) choice, not a per-declaration implementation: every TLS reference
in an object compiled with the option uses Initial-Exec.  Local-Dynamic and
Global-Dynamic are rejected explicitly rather than silently approximated.

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

## Completed frontier and runtime fixes

The `e_acosh.c` failure was reduced to a local x86 peephole folding a stack
reload into an address although the reloaded value was XMM-homed. The fold now
requires a GPR-capable address value; the assembler remains fail-closed for an
XMM base. The exact glibc command subsequently compiled.

With explicit Initial-Exec TLS for this startup-loaded libc, the complete glibc
build and staged install succeeded. Direct `write`, malloc/free and overlapping
memmove probes passed through the custom loader. A broader smoke then exposed a
second independent defect in `vfprintf-internal.c`: lowering emitted a pointer
`Load`, but call conversion labelled its SSA result I32 and truncated a stack
address. CType pointer results are now authoritative, and a call argument whose
defining instruction is `Ptr` can only be strengthened to `Ptr` (never blindly
retyped: small aggregate ABI carriers are intentionally integers). The latter
restriction was added after red-team testing caught four SysV aggregate crashes
in a 186-shape cross-compiler ABI grid. All 186 shapes now pass in every tested
caller/callee direction.

The pointer-width correction was also measured across that grid rather than
assigned a speculative spill-size benefit.  With seed `20261017`, 160 random
plus 26 curated top-level shapes, compile both generated TUs at `-O2 -w -S`
using `532669b7` (before the pointer-width correction) and `95bbb4cd` (after):

```sh
python3 tests/regression/check_sysv_aggregate_abi.py \
  --emit-only --keep /tmp/sysvabi
for tu in prov call; do
  BEFORE/target/fastbuild/lccc -O2 -w -S /tmp/sysvabi/$tu.c -o /tmp/$tu.before.s
  AFTER/target/fastbuild/lccc  -O2 -w -S /tmp/sysvabi/$tu.c -o /tmp/$tu.after.s
  cmp /tmp/$tu.before.s /tmp/$tu.after.s
done
```

The paired assembly files were byte-identical: **0 generated assembly bytes and
0 spill bytes changed** in this corpus.  The `call` assembly SHA-256 was
`422d8eb6267dcee018b5731e1fefa71604d87d87ae95e236ff808ee6bf4bd4bf` and
`prov.c` assembly was
`8fcd0d3a11b0f8d85eb0b35a377eac8f985a3be388d5bfe1239c058a1fc7d3e9`.
This grid validates the ABI boundary but does not exercise glibc's
pointer-return/load path, so no performance improvement is claimed from this
zero delta.

Strict-FP glibc exposed two self-recursive builtin implementations as well:
`__fma` called its public `fma` alias, and `__ieee754_sqrt` called `sqrt`.
Hardware FMA and sqrt obey MXCSR rounding and exception state; GCC 16.2 also
emits them under `-frounding-math -ftrapping-math`. They are therefore included
in the strict-environment-safe intrinsic set. The final staged-loader smoke
prints:

```text
glibc=2.44 fma=10.0 acosh=1.31695790
```

## Oracle and performance evidence

The cross-vendor Compiler Explorer semantics sweep passed all 9 programs with
GCC 16.2, Clang 23.1, ICC 2021.10 and current ICX (36 vendor executions, zero
divergences). Aggregate LCCC instruction count was 1487 versus 1852 GCC, 2274
Clang, 3714 ICC and 2441 ICX. This is code-size evidence, not a cycle claim.

The glibc-derived Callgrind A/B screen against GCC 14 found:

- `libm_round_family`: Ir ratio 0.97505 (LCCC/GCC), but native paired median
  1.0472 and a large D1-miss disadvantage;
- `glibc_memcmp`: Ir ratio 1.05136 and native paired median 1.0101.

The round-family assembly oracle explains the native loss: Clang emits a
four-lane packed rounding loop, while LCCC emits a two-way scalar unroll. This
is recorded as a real optimization backlog item, not patched speculatively:
adding packed round nodes touches the vectorizer's parsing, legality, reduction,
remainder and backend tables and requires exception/rounding-mode tests. The
current patch deliberately keeps the correct scalar implementation rather than
claiming a size-only or unvalidated speed win.

## Follow-up audit verification (2026-10-19)

The repository census is a Python ratchet, not a Cargo binary.  The commands
actually run were:

```sh
python3 scripts/audit_census.py --selftest
python3 scripts/audit_census.py --check
```

Both passed against `scripts/audit_census_baseline.json`; no metric grew, so the
baseline was correctly left unchanged rather than regenerated gratuitously.
Patch applicability is checked with `git apply --check`.  No unsupported
`git am --fuzz` command is claimed (`--fuzz` belongs to `patch`, as used by the
glibc workload runner, not to `git am`).

## Red-team conclusions

- **Accepted:** preserving linker rejection of TPOFF32 in shared objects. IE is
  selected explicitly only for startup-loaded glibc; silently weakening the
  linker would be unsound for late `dlopen`.
- **Accepted after narrowing:** definition-driven call typing. The first broad
  implementation broke small aggregate carriers. Restricting the override to
  an actual `Ptr` producer fixes vfprintf without bypassing aggregate ABI logic.
- **Accepted:** strict FMA/sqrt inline lowering. It preserves operation-level FP
  semantics and prevents glibc alias recursion; focused runtime and assembly
  gates pin both properties.
- **Rejected:** backend register/rematerialization and final-peephole theories
  for the printf crash. No rematerialization event occurred and the malformed
  I32-to-Ptr cast was already present in optimized IR.
- **Rejected for now:** packed-round vectorization based only on oracle assembly.
  Native and Callgrind evidence identify the opportunity, but correctness under
  strict FP and reduction ordering needs a dedicated implementation and corpus.
- **CI hygiene fixed:** externally visible leaf definitions may now inline
  (matching GCC 16.2), so the libcall gate tests semantics rather than requiring
  a stale call shape. Newly added standalone regressions are mirrored in local
  and hosted gate inventories.
