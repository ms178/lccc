# 2026-08-19 (session 12) — kernel boot: no-op-cast coalescing + dead-value registers + sign-ext pruning

**Base:** `origin/main` @ `1c127e47` (unchanged — the session-11 merge had not
landed on GitHub at session start; this work is already based on it)
**Snapshot:** `/home/user/ms178-1.patch`

## Setup

Re-extracted linux-6.18.44 + re-cloned archpkgbuilds (both wiped), re-applied
the 26 patches, regenerated the boot stubs. Environment (swap/rust/apt)
survived this time.

## Three size wins (all i686-only; x86-64 output byte-identical)

### 1. No-op cast coalescing — the big one

`build_coalesce_groups` only merged `Copy` edges, so a same-width U32↔I32
cast (a **bit-preserving no-op** on 32-bit targets) materialized as a relay
copy. Every `cpu_vendor[0] == 'Genu'` (u32 global compared as I32) emitted:

```
movl sym,%edx ; movl %edx,%ebx ; cmpl $imm,%ebx
```

Now the cast coalesces like a Copy (32-bit gated):

```
movl sym,%edx ; cmpl $imm,%edx
```

`is_intel`: 30 → 19 instructions. This pattern dominated every global-load
compare site in the boot corpus. Gated on `target_is_32bit()`: on x86-64 the
upper 32 bits of the register have different zero/sign semantics, so the cast
is NOT folded there.

### 2. Dead-value register exclusion

`build_folded_value_set` (generation.rs) collects the values the codegen loop
folds away — loads folded into `cmp{b,w,l} $imm,(mem)` and fused
compare-and-branch booleans — and the i686 prologue adds them to its
`never_materialized` set. They emit no code, so they must not occupy a
register or slot. Previously the allocator parked the dead byte/boolean in
`%edx`, stealing it from the live loop pointer (the root cause of strlen's
extra push/pop pair).

### 3. Redundant sign-extension pruning

`movl $imm` with `imm ∈ [-128,127]` and `andl $imm` with `imm ∈ [0,127]`
leave sign-extended values, so the trailing `movsbl %al,%eax` is a no-op.
Extended `eliminate_redundant_sign_ext_i686` with the range check (the existing
zero-ext pass already did this for byte-ness).

## Numbers (linux-6.18.44 arch/x86/boot, -m16 -Os)

| metric | session 11 end | now |
|---|---:|---:|
| setup.elf `.text` | 31271 | **31214** (−57) |
| total object text | 33755 | **33700** (−55) |

Per-object, none regressed: string −7, video −8, video-vesa −8, video-bios −8,
cpucheck −8, printf −4, main −4, tty −10.

The 32 KiB gate needs `.text ≤ ~27449` (`pecompat` 4K alignment); we are
**~3.7 KB over**, down from ~3.8 KB.

## Correctness (the hard constraint)

The fuzz caught a bug I almost shipped. First attempt: a cast-hazard
relaxation (`Cast` marked `%edx`-clean) that let strlen's loop pointer take
`%edx`. It looked sound (the cast's `store_eax_to` writes the dest's own
register, overlap-protected) — but `m32_differential_fuzz` seeds 0/80–100 at
O2/Os diverged. Root cause: a loop-carried value in `%edx` got clobbered
across a `URem`'s `xorl %edx,%edx; divl` — the relaxation let it span a cast
whose surrounding sequence reuses `%edx`. **Reverted.** Correctness first.

The shipped changes are validated:
- 918 unit tests (4 new sign-ext, 1 new relay codegen check)
- 50/50 correctness
- 340 regression + 6 diagnosed GCC-oracle mismatches (unchanged set)
- 600-case i686 differential fuzz (seeds 0:200 × O0/O2/Os), 0 mismatches
- `tests/regression/check_u32_i32_cast_no_relay.sh` proves the relay is gone
  (and fails on the pre-fix binary)

## Remaining levers (unchanged, documented)

1. `%eax` accumulator reservation — the fundamental 4-push/pop tax.
2. Caller-saved-first (Phase 0) allocation order — the greedy start-ordered
   linear scan wastes freed registers on low-priority values; a loop-weight
   first pass would make the dead-value exclusion and cast coalescing pay off
   much more.
3. `%ecx` scratch conflicts (compare LHS / store value in `%ecx` degrade to
   accumulator round-trips).
