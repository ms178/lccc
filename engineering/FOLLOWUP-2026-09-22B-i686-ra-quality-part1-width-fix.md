# Follow-up: i686 RA quality, Part 1 — the `I64` constant-spelling trap is found and fixed

Date: 2026-09-22 (session 60)
Base: `e08565c0` (`ms178/lccc` main; re-based per standing instruction).

Session 59 closed the "what is the gap" question (+515 slot stores, +288 slot
loads, 70/70 functions with a frame). This session closed the "why do the
short-lived values get slots" question for the *const-seeded accumulator*
defect class, and landed the principled fix: **width resolution moves from
constant *spelling* to *context evidence***, in the value-type map that is
already the single source of truth for materialisation width.

---

## 1. Root cause (measured, three independent reads agree)

`check_cpuflags` (`arch/x86/boot/cpucheck.c:89`) has a u32 loop accumulator:

```c
static int check_cpuflags(void) {
    u32 err = 0;                    /* ← preheader: Copy v36 ← Const(I64(0)) */
    int i;
    for (i = 0; i < MAX_CPU_LEVEL; i++) {
        if (!(req_flags[i] & ~cpu_flags[i]))
            err |= 1U << i;         /* ← or v36,v29 (I32) + two latch copies */
    }
    ...
}
```

`LCCC_DBG_RA=1` on the base binary showed the defect end-to-end:

* `values=22 assigned=10`; the accumulator web `{v36 (acc), v38 (phi dest),
  v31 (or-result)}` was the **HOMELESS web** — none of the three got a
  register; v36's interval was the *longest in the function* (21 instrs) and
  every update round-tripped a 16B frame slot.
* The poison seed: `collect_non_gpr_values` (i686) excludes "I64 values"
  from GPRs (they need register pairs). The preheader init
  `Copy v36 ← Const(I64(0))` matched its `Copy` arm: source is a constant of
  type `I64` → v36 classified non-GPR → the whole copy web (v36/v38/v31)
  excluded from GPR allocation.
* **Why the constant is spelled `I64` even though the C type is u32**:
  `IrConst::from_i64(val, ty)` (`src/ir/constants.rs:596`) stores U8/U16/U32
  constants zero-extended in an `I64` payload to preserve unsigned semantics.
  So `0` for a `u32` is *literally* `IrConst::I64(0)` in the IR. `I64`
  spelling ≠ 64-bit value. (Adding a typed `U32` const variant is the other
  root fix — rejected: `IrConst::I64` has 1004 uses across 96 files; an
  exhaustive-match change of that size for a width annotation is the wrong
  trade.)

The same mis-spelling also poisoned the *other* width reader:
`compute_value_type_map::const_type` seeded `Copy dest ← Const(I64(..))`
with `I64`, so the slot classifier and the x86-64 spill/reload width pickers
saw an 8-byte value too. Both readers had to be fixed from the same source of
truth — which is exactly what the map exists to be.

## 2. The fix (three coordinated edits, one invariant)

Invariant: **a value's materialisation width comes from the contexts that
read it, never from an ambiguous constant spelling.**

1. **`src/backend/common.rs` — `compute_value_type_map`:**
   * `const_type`: `IrConst::I64(_) | IrConst::Zero` now return `None`
     (ambiguous storage form). Unambiguous spellings (`I8/I16/I32/I128/
     F32/F64/D32/D64/LongDouble`) keep their seed.
   * The fixpoint gains **operand-context propagation** arms — every declared
     type propagates to its operand values: `BinOp|Cmp` operands ← op `ty`;
     `UnaryOp` src ← `ty`; `Store` val ← store `ty`, ptr ← `Ptr`;
     `AtomicLoad/Rmw/Inc/Cmpxchg` ptr ← `Ptr` (+ val ← `ty`); `Select` cond ←
     `I8`, values ← `ty`; `Cast` src ← `from_ty`; `Copy` bidirectional
     (constant seed kept only for unambiguous spellings); `Phi` dest +
     incoming values ← phi `ty` (declared type subsumes per-edge constant
     seeds); `GEP` dest/base/offset ← `Ptr`; `GlobalAddr|LabelAddr|Alloca|
     DynAlloca|StackSave|GetStaticChain` dest ← `Ptr`; `Call|CallIndirect`
     args ← `arg_types[idx]` (ABI width); `CallIndirect` func_ptr ← `Ptr`;
     `Memcpy/VaArg*/VaCopy` ptrs ← `Ptr`; terminator `Return` value ←
     `func.return_type`, `IndirectBranch` target ← `Ptr`.
   * The **widest-wins** monotone rule is unchanged; a value with *no*
     evidence stays untyped, and every consumer treats untyped as the
     conservative wide default (emitter `movq`/8-byte slot) — the pre-change
     behavior, so the weak seed is a pure precision gain, never a
     regression.

2. **`src/backend/regalloc.rs` — `collect_non_gpr_values` (i686):** the
   `I64 if is_32bit` Copy arm now consults the map instead of the constant
   spelling: non-GPR only unless `value_type_map.get(&dest).size() <= 4`;
   `None` → legacy conservative non-GPR.

3. **`src/backend/stack_layout/slot_assignment.rs`:** i686 compact (4-byte)
   slot admission now accepts every map value with `t.size() <= 4`
   (non-decimal) after the copy fixpoint — the mirror image of the veto in
   `wide_typed_values`.

### 6 new unit tests (`value_type_map_tests`)

Replay the defect and its safety envelope at IR level:

| test | pins |
|---|---|
| `i64_stored_const_seed_resolves_to_operand_width` | the exact `check_cpuflags` shape (const I64 seed + I32 `Or` + latch copies) → whole web 4 B |
| `i64_stored_const_seed_stays_wide_for_64bit_consumer` | a *real* `u64 x = 5;` keeps 8 B (no blanket narrowing) |
| `unused_i64_stored_const_seed_stays_untyped` | no context ⇒ untyped ⇒ conservative wide default |
| `phi_type_subsumes_wider_const_incoming` | declared phi `ty` governs over `Const(I64)` edges |
| `call_arg_width_seeds_operand` | ABI width seeds call arguments |
| `store_width_seeds_operand` | store type seeds the value (and the ptr gets `Ptr`) |

## 3. Evidence

### 3a. `check_cpuflags` RA + asm (before → after)

```
values=22  assigned=10 → 12
v36 (acc): HOMELESS web  → r0  [0,32] 21T      (was: 16B slot, 21 uses)
v38 (phi): HOMELESS web  → r5  [22,30]         (was: slot)
```

```asm
; before: 32B frame, acc round-trips the slot every iteration
    movl $0, 16(%esp)
    ...
    movl 16(%esp), %eax
    orl   %edx, %eax
    movl  %eax, 16(%esp)
; after: 16B frame, acc in %ebx, in-place or
    xorl %ebx, %ebx
    ...
    orl  %ebx, %edx
    ...
    movl %edx, %ebx        ; one back-edge copy remains (Part 2)
```

### 3b. Boot corpus (21 TUs, identical command lines, `boot_ab_size.py`)

**−160 B (−0.66%)**: video −109, string −19, tty −16, main −13, cpucheck −11,
video-vesa −7, video-vga −6, 16 objects unchanged; video-mode +1, video-bios
+20. The two regressions are attributable, not noise:

* **video-bios +20** = register-pressure tradeoff: homing the accumulator
  adds GPR demand; the greedy scan then spilled a differently-used value
  (a `leal 1(%edi)` result: 1 insn in a register before, 3 insns around a
  frame store after). Net corpus stays negative. (Same lever as session 59's
  "greedy scan spends the 4-register pool on long-lived values" open item.)
* **video-bios also shows a redundant extension** the new placement exposes:
  `movl %esi, %eax; movzbl %al, %eax` (2 insns) where a slot load was one
  `movzbl`. Fold `reg→reg copy + subregister extend` into one
  `movzbl %sil, %eax` — queued (peephole, low risk).

Instruction *count* moved +19 (5093→5112) while *bytes* moved −160: the new
code uses more, shorter reg-reg moves and smaller frames. Bytes is what the
32 KiB gate measures.

### 3c. Oracle gates (post-fix)

* `boot_size_oracle.sh`: lccc `_end=31168` (headroom **1600**, PASS) vs
  gcc `_end=22880` (gap **+10416**, was +10698) and clang `_end=22768`
  (gap **+9131**, was +9413). **−282 B on the linked setup image vs both.**
* CE oracle comparison of the preprocessed `cpucheck.i` (the kernel TU,
  headers expanded locally, compiled with the boot command line):
  * **gcc16.2**: 8-insn body, absolute real-mode addressing
    (`andl req_flags(,%ecx,4), %eax`), acc in `%edx`, **branches on the
    `andl` flags — no compare at all**.
  * **icx 2026**: 9-insn body, acc in `%eax`, hoists `1` before the loop and
    shifts per iteration (`movl %edx, %esi; shll %cl, %esi`), zero slots.
  * **clang23.1**: **inlines `check_cpuflags` into `check_cpu`** (static
    callee, -Os) — the loop runs in the caller's register context.
  * **icc 2021.10**: `-m16 not supported` — the Classic oracle cannot
    compile real mode at all; recorded as an oracle limitation, not a gap.

### 3d. Test gates (final)

* `cargo test --profile fastbuild --locked --lib`: **3131 passed, 0 failed,
  7 ignored** (baseline 3125 + the 6 new map tests). All-target `cargo test`
  green.
* `run_regression_suite.sh`: **PASS=742 FAIL=0 SKIP=8**, AB-diff failures 0
  — identical to the pre-change baseline.
* `build_kernel_vm.sh`: **SUCCESS in 1898 s**; `vmlinux` 20577712 B
  (text 11896870 — **−4528 B** vs the previous build's 11901398, the
  context-evidence width also helps the x86-64 kernel side);
  `bzImage` **5067776 B** (−4096 B); **setup `_end` 0x7280 = 29312 B**
  (was 29552) — headroom **3456 B** (was 3216); sha256
  `aa2372ed9840bb5ef8d55510674611b0a6c80114ed1fea5099991185b2d03ae0`.
* `qemu_boot_test.sh`: **16/16 PASS** (banner, SCHED_BORE/BORE init,
  SCHED_CACHE/CACHE_HOT_BUDDY, HZ_800, CACHY, BBRv3, PREEMPT, SMP, bbr
  algo list, BORE per-task score, 2 CPUs online, both serial sentinels,
  clean poweroff) — zero WARNING/Call Trace in the serial log.

## 4. Residual `check_cpuflags` gap vs GCC 16.2 (what Part 2+ must close)

Per iteration, ours still emits four things GCC does not:

| # | residual | class | candidate lever |
|---|----------|-------|-----------------|
| a | `movl $err_flags, %ebp` per iteration | real-mode store can't use absolute `sym(,%reg,4)` the way the *loads* do | i686 emitter: absolute-address stores in `-m16` non-PIC (codegen, contained) |
| b | `movl %eax, 4(%esp)` + `movl 4(%esp), %eax` | loaded value round-trips a slot — the value has no free GPR in the greedy scan | RA priority/pressure-aware scan (session 59 open item 1a) |
| c | `movl %eax, 0(%esp); cmpl $0, 0(%esp); je` | compare-against-frame-slot + missed and-flag fold | (i) same as (b); (ii) IR/peephole fold `Cmp(and(x,y), 0)` → branch on `andl` flags |
| d | `movl %ebx, %edx` / `movl %edx, %ebx` | phi pass-through copies (the web is homed but not chained) | **Part 2: phi-chaining/coalescing** — see gate below |

**Part 2 gate (refined, to keep the blast radius small):** chain
`dest ← m` where `m` is multi-def, *all* of `m`'s defs are copies, exactly
one non-passthrough def `u` exists, and at least one passthrough def
(`x == dest`) exists; require the cross-block window proof at `u`'s site and
that every other predecessor of the copy-block contains a passthrough def of
`m`. Then `{v36, v38}` chain + coalesce with `{v31}` → the web `{v36, v38,
v31}` takes one register and the `or` lands in place. Do not relax the
multi-def-source guard generally — it exists for a reason (session 59's
"the exemption stays" precedent).

## 5. Gate status (all green)

| gate | result |
|---|---|
| `cargo test --profile fastbuild --locked --lib` | **3131 passed, 0 failed, 7 ignored** |
| `cargo test` (all targets) | green |
| `run_regression_suite.sh` | **PASS=742 FAIL=0 SKIP=8**, AB-diff failures 0 (baseline-identical) |
| `boot_ab_size.py` (before/after binaries) | **−160 B** (7 better, 16 flat, 2 worse by +21) |
| `boot_insn_census.py` | 5112 vs 3183 (+60.6%); store-to-frame 561/46, load 355/67, reg-reg movl 453/226, frame bytes 5540/2996, with-frame 73/31 |
| `boot_size_oracle.sh` | lccc headroom **1600 PASS**; gap vs gcc **+10416** (−282), vs clang **+9131** (−282) |
| CE oracles (gcc16.2/clang23.1/icx/icc) | compared, §3c (icc cannot do `-m16`) |
| `build_kernel_vm.sh` | **SUCCESS 1898 s**; bzImage 5067776 B (−4096), vmlinux text −4528 B, setup headroom **3456 B** |
| `qemu_boot_test.sh` | **16/16 PASS**, clean poweroff, zero WARNING |

## 6. Part 2 — phi chaining/coalescing: DONE (session 62)

Residual (d) — the `{acc, phi-dest, or-result}` web homed but not chained
(two pass-through copies per iteration in the kernel `check_cpuflags`
`err |= 1 << i` loop) — is closed.

### 6a. Ground truth (reconstructed, `-m16 -Os`)

The kernel shape (branch, not select) is reproduced by the two-TU /
opaque-store reconstruction (`/tmp/cpucheck_fair.c`). lccc's IR after phi
elimination:

```
block 3 (orpath): v41 = v45 | v39;  v47 = v41   (m ← u)
block 4 (skip):   v47 = v45         (passthrough m ← dest)
block 5 (latch):  v45 = v47;  v46 = v43
```

Pre-fix: `orl` staged into a temp + latch `mov` back to the acc register +
skip-path pass-through `mov`. The latch copy `v45 = v47` was blocked by the
multi-def-source guard (`v47` is defined in both branch arms).

### 6b. Implementation (narrow exception, guard NOT relaxed)

`regalloc.rs`:

1. `detect_part2_chain` — for the copy `dest ← m` with `m` multi-def,
   accept only when: every def of `m` is a `Copy`; exactly one
   non-passthrough def (`m ← u`, `u` single-def) and ≥1 passthrough def
   (`m ← dest`); `u`'s block is a distinct predecessor of the copy block
   and defines `m` from `u`; **every other predecessor of the copy block
   contains a passthrough def of `m`**; `u`'s block is no shallower than
   the copy block (preheader veto). Returns `u`'s site + `m`'s def in
   `u`'s block.
2. Detection runs the EXISTING window proofs on `u`'s site (the wider
   window); the candidate records `m`'s def so the apply-phase structural
   checks and the slot-coalescing contract see a real def of the
   backedge source. Everything else in the detection path is unchanged;
   any shape outside the gate is blocked with its own debug message.
3. Sort: candidates are ranked by chain depth (a candidate whose
   backedge source is another candidate's phi dest sorts first) so the
   apply phase cascades the register down the web: `{acc, m}` before
   `{m, u}` → one register for the whole web.
4. Apply: the home-conflict check now exempts the proven same-home class
   (values united by already-applied pairs), not just the pair's two
   values — the upstream dest (acc) rides the shared home and previously
   caused the chain link to be silently rejected.

### 6c. Measured effect (reconstruction, `-m16 -Os`)

| | before | after |
|---|---|---|
| or-path | `shll; mov %eax,%edx; orl %esi,%edx; jmp` + latch `mov %edx,%esi` | `shll; orl %eax,%esi; jmp` |
| skip-path | `mov %esi,%edx` + latch `mov %edx,%esi` | *(nothing)* |
| encoded function size | 217 B | **211 B (−6 B)** |

Residual (d) is eliminated; the `diff` of the asm is exactly the three
removed copies and the in-place `or` — nothing else changed.

### 6d. Oracle check, fair shape (`-m16 -Os`, flags loop)

| compiler | result |
|---|---|
| Clang 23.1 | loop fully vectorized (SSE2 reduction) — separate mid-end tier |
| GCC 16.2 | branchless `btsl`/`cmovne` scalar loop, dead static-store DCE, direct SIB addressing — 8 body instructions |
| ICX/ICC | N/A — Intel compilers have no `-m16` mode |
| lccc (after) | branched scalar loop; **Part-2 web chained, `or` in place, zero pass-through copies** |

### 6e. Remaining gap vs GCC 16.2 — mid-end levers (not RA)

1. Dead **static-store elimination** (lccc keeps the `err_flags[i]` store
   GCC drops: static + never read again in the TU).
2. Branchless conditional-update transform (`testl`/`cmovne`/`btsl`
   instead of the explicit branch + in-place `or`) — a select/branch
   mid-end transform.
3. Direct SIB addressing of absolute globals (`cpu(,%eax,4)`) vs lccc's
   GOTOFF-base + slot-staged GEPs in `-m16`.
4. (b) slot round-trips for loaded values → RA-3 pressure-aware scan
   (still the dominant residual lever per the §3 census).

## 7. RA-3 — pressure-aware scan: characterization + first negative (session 62)

Before touching the allocator, the residual's shape was measured (RA-06
precedent: the last two pressure-machinery attempts each died on a cause
that was elsewhere).

### 7a. Census, current tree (kernel corpus, 15 hot fns)

x86-64 `-O2`: **359/264** insns (lccc/gcc, +36 %; the session-59
kernel-vmlinux census was +60.6 %). `rrmov 41/14`, `stkref 12/0`,
`push 11/5`. The rrmov excess concentrates in the auto-vectorized loops
(`adler8` 125/63, `maxv` 61/27) — that is vectorizer size/shape, not
GPR pressure, and is a separate lever.

i686 `-m32 -O2` (boot-like 6-register budget): **479/383** (+25 %),
`rrmov 41/13`, `stkref 80/53`, **`push 55/20`** — lccc pushes
4 callee-saved in every one of the 15 functions; GCC pushes 0 in the
call-free leaves. Two distinct mechanisms:

1. **push**: hot-loop values are homed callee-saved (the session-28
   Phase-1 promotion, still in effect on i686 because the caller-saved
   pool is only `%ecx/%edx` — `%eax` is Phase-2e-only). Small leaves like
   `ffs1` (loop state = 2 values) still overflow: the asm shows `x` in
   `%ebx`, a copy in `%edx`, `r` in `%esi`, temps in `%edi/%ebp`.
2. **stkref**: scalar loops (`dot` 16/3, `hash` 13/6) stage loaded
   values through slots — `dot`'s frame is 44 B and the FP accumulation
   round-trips (x87/FP-class issue, adjacent to the x87↔GP lowering item).

### 7b. Negative result — blanket i686 leaf caller-saved homes

Hypothesis: let i686 leaves use `leaf_caller_saved_homes` like x86-64
(small leaves lose the pushes; overflow still gets Phase-2c homes).
**Measured on the -m32 kernel corpus: regression.**

| bucket | before | after |
|---|---|---|
| insns | 479 | **506 (+27)** |
| stkref | 80 | **106 (+26)** |
| push | 55 | 55 (unchanged) |

`adler8` 77→88 (stkref 18→35), `isort` 45→64 (stkref 6→18), `crc32k`
40→43 (rrmov 4→7); only `bswp32` improved (26→21). The 2-Register
caller-saved pool cannot hold loop state; the overflow lands in slots
before it lands in Phase-2c, and the few small-leaf push savings do not
compensate. The historical comment in `regalloc_helpers.rs` (i686 keeps
Phase-1 promotion) is vindicated. **Reverted; do not retry the blanket
flip.** The i686 push lever, if pursued, needs per-function demand
estimation (how many simultaneously-live loop values vs the ecx/edx/eax
capacity) — a measurement campaign, not a flag.

### 7c. Consequence for the chain

RA-3 stays the dominant residual lever but is now scoped as
**demand-aware Phase-1 admission on i686** (promote to callee-saved only
when the loop's live demand exceeds the caller-saved pool), not a
pressure splitter rework. Next tractable item in the chain: **RA-4**
(the `Cmp(and(x,y), 0)` fold — residual (c): `mov %eax, 0(%esp);
cmpl $0, 0(%esp); je` → branch on the `and`'s flags).
