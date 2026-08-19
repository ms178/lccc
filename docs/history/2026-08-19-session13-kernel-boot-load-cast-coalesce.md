# 2026-08-19 (session 13) — kernel boot: load→widen-cast coalescing + acc-cache preservation

**Base:** `origin/main` @ `a8844c03` (PR #127 — session 11 landed upstream)
**Snapshot:** `/home/user/ms178-1.patch`

## Re-base

Session 11 landed as PR #127 (`5651e8e9`). Session 12 (cast coalescing) was
still pending, so it was recovered from `/home/user/artifacts/lccc.bundle`
and replayed onto `a8844c03` (conflict-free: PR #127's squash tree is
byte-identical to session 11's final tree). Kernel + archpkgbuilds re-extracted
and re-patched (both wiped again).

## Two size wins (both i686-only, x86-64 output byte-identical)

### 1. Coalesce sign-extending sub-word loads with their widening casts

The `strchr` shape (`while (*s && *s != c) s++`) loads `*s` (I8), sign-extends
it (`movsbl`), and compares it twice. The load and the widening cast
`(I32)(I8)*s` are a bit-preserving pair — the load already extended into its
register — yet the cast materialized a relay through the accumulator and a
second register:

```
movsbl (%ebx),%esi ; movl %esi,%eax ; movl %eax,%edi ; cmpl %ebp,%edi
```

Now they coalesce (32-bit gated; the load must have ≥ 2 uses so a
would-be-spilled single-use load cannot drag the cast's register down with
it — skip_atoi taught that lesson, measured +15 before the gate), and the
coalesced cast emits nothing:

```
movsbl (%ebx),%esi ; cmpl %ebp,%esi
```

Frees %edi (strchr drops a push/pop pair; 22→19 insns). Same-value cast edges
may OVERLAP (the load is also read by the `*s==0` test), so the strict
non-overlap acceptance is relaxed exactly for them (`same_value_edges`).

### 2. Keep the accumulator cache valid across register stores

`store_eax_to()` materializes %eax into a value's register home and then
invalidated the acc cache, forcing the next use of the value to reload it
(`movl %reg,%eax`). But `movl %eax,%reg` leaves %eax unchanged, so the value
is still in the accumulator. The register path now `set_acc` (mirroring the
slot path), so the next `operand_to_eax`/cast/compare hits the cache and
skips the reload. The cache is invalidated by any real %eax clobber anyway.

## Numbers (linux-6.18.44 arch/x86/boot, -m16 -Os)

| metric | session 12 end | now |
|---|---:|---:|
| setup.elf `.text` | 31214 | **31044** (−170) |
| total object text | 33700 | **33526** (−174) |

Per-object, none regressed: string −93, video-mode −36, video-vga −16,
cmdline −12, cpucheck −11, early_serial −6, cpu −6, main −3, video −1.

Remaining instruction mix is dominated by `movl` slot/relay traffic — the
`%eax`-reserved accumulator tax, now isolated to the structural rework
(%eax allocatability / caller-saved-first Phase 0).

## Investigated and reverted (documented)

- **Direct register-to-register int cast** (`movsbl %slo,%dest`): correct and
  fuzz-clean but a net −3 with a +14 video-vga regression — it bypassed the
  accumulator cache, materializing call results into their register home
  before the extension. The acc-cache fix (#2 above) is the strictly-better
  root fix. Reverted.

## Correctness

- 918 unit tests (was 910 at session start; +1 relay codegen check
  `check_load_widen_cast_no_relay.sh`)
- 50/50 correctness
- 341 + 6 diagnosed GCC-oracle mismatches (unchanged set)
- 900-case i686 differential fuzz (seeds 0:300 × O0/O2/Os), 0 mismatches
