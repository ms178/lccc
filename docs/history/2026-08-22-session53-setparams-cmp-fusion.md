# 2026-08-22 session 53 — zlib-ng `example` / `zng_deflateSetParams` miscompile

Base: `a61e417a41697ccf60fbc723d9124f5a52869992` (PR #193). Continues
session 52 (IV-across-call, GEP+0/mem2reg, fnptr-param sema).

## Agent A / Agent B (unchanged verdict)

- **A:** keep the *invariant* (call-spanning IV must not ride a
  caller-saved home). Reject function-wide `live_across_any_call`.
  Window-local `phi_window_clobbers_caller_saved` stays.
- **B:** take IR hygiene (GEP+0, Load/Store GEP0, post-fold2 mem2reg,
  i686 fold any dead GPR except ESP/EBP). Reject `.base_ref`,
  `MAX_SMALL_INLINE_BLOCKS` 3→12, split-call on every function, `testl`
  operand swap.

## New miscompile (this session)

Clean rebuild of zlib-ng 2.3.3 + `ms178-1.patch` (recipe from
`ms178/archpkgbuilds`) failed **1/69**: `example`.

```
Expected compression level Z_NO_COMPRESSION, got 1
```

`zng_deflateSetParams` after `deflateInit(Z_BEST_SPEED)` then
`SetParams(Z_NO_COMPRESSION)` left `s->level == 1`. Isolated:

```
set err=-5 status=0,-5   # Z_BUF_ERROR on the STRATEGY param
```

`param->size` was 4, `min_size` was 4. The compare
`int32_t buf_error = param->size < min_size` was turned into
`buf_error = size` (4), so `if (param_buf_error)` fired.

### Root cause

x86 peephole `fuse_compare_and_branch` accepted any
`movzbl %al, %<reg>` as “the setcc result”, then fused the preceding
`cmp`/`setb` with the next `testq %rax, %rax` / `je`.

In `zng_deflateSetParams` the STRATEGY arm is:

1. `cmp $4, %r8` / `setb %al` / `movzbl %al, %r12d`  (`size < 4`)
2. load slotted `new_strategy` into `%rax`
3. `testq %rax, %rax` / `je` else   (`*out != NULL`)

The peephole fused (1) with (3) and rewrote the branch to `jae`
(inverted `setb`). A valid 4-byte buffer took the `*out != NULL` body
and returned `Z_BUF_ERROR`. `CCC_NO_COALESCE=1` hid it by changing
homes so the boolean landed in `%eax`.

### Fix

`compare_branch.rs`: skip only `movzbl %al, %eax` / `movzbq %al, %rax`.
A `movzbl` into any other register means a later `%rax` test is a
**different** value.

Defense in depth (`prologue.rs`): Cmp dests are never put in the
x86-64 immediately-consumed nohome set. Flag fusion
(`fused_cmp_dests`) remains the only legal way to skip setcc.

## Validation

| Gate | Result |
|---|---|
| zlib-ng 2.3.3 + ms178-1.patch, Ninja `-O2`, AVX512 off | **69/69** CTest (was 68/69) + minigzip roundtrip 1/2/6/9 |
| Isolated `zng_deflateSetParams` | `set err=0` / `get2 level=0` |
| gzip 1.14 `-O3 -march=x86-64-v3` | **30/30** |
| `cargo test --lib --profile fastbuild` | **1079 passed**, 0 failed, 6 ignored |
| regressions `deflate_setparams_cmp_cast`, `loop_iv_across_call`, `fnptr_param_assign`, `addr_of_inline_mem2reg` | pass |
| unit `test_compare_branch_does_not_fuse_setcc_in_other_reg_with_rax_test` | pass |

Workloads re-fetched; hashes match provenance:

```
gzip-1.14.tar.xz       01a7b881bd220bfdf615f97b8718f80bdfd3f6add385b993dcf6efd14e8c0ac6
sqlite-src-3530400.zip d18fa15aec74d8c17e1463f861095adc01b5ad190256acb4f91d22f0368d232b
glibc-2.44.tar.xz      37f600f2bef3c5e8300147059568b2a2e40a7ad6ccc65ce942556d49429cc667
ms178-glibc.patch      52c9953f8e3ec710d73308464bd98bbe752895d83c6b17201235edcb1c974aeb
zlib-ng ms178-1.patch  40a317e1ac64e458bab6133ef259288edeab1a9054249dece3c7048fade3c2cc
```

glibc 2.44 official tarball + `ms178-glibc.patch` (no sourceware git).
Full configure/make still exceeds 2 GiB RAM.

## Not a miscompile (left as RA/ISel quality)

- gzip `longest_match` stack-mem vs GCC (RA-01/RA-02).
- Adler DO8 `sum2`/`n` on stack (RA-03/RA-06).
- sqlite `yy_shift` cmp-replay slot-only (IS-09, conservative).
