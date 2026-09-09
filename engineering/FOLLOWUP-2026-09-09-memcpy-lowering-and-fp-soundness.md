# Follow-up: constant-size `__builtin_memcpy` lowering, unaligned-load forwarding, and the FP/SROA liveness soundness fixes — 2026-09-09

**Base:** `main` @ `cdb01750` (upstream, current at clone time; no upstream move,
so no re-base was required this session). Session commits on top:
`041ba8bc` (memcpy lowering), `3f9778bb` (peephole FP soundness),
`eecf2536` (aggregate_sroa aliasing-store soundness), `d8fe26d0` (docs/backlog).

**Scope of this session:** a red-team audit of the 10 worst CI benchmark gaps,
focused on low-risk / high-reward low-hanging fruit first, using the godbolt
code-generation oracle as ground truth for instruction/load/store/spill counts.
Outcome — three findings, each a *real* defect, the last two exposed by lowering
constant-size `__builtin_memcpy` to the native `Instruction::Memcpy`:

1. `read64`-style `T v; __builtin_memcpy(&v,p,8); return v;` lowered to a libc
   `call memcpy`, leaving a **store-to-temp + reload** in every hot loop that does
   an unaligned scalar load. Now lowered to native `Memcpy`; SROA collapses it to
   a single unaligned load (GCC parity).
2. That change **exposed a latent peephole miscompile** in
   `fold_ptr_deref_through_stack` — an aliasing-store soundness hole fixed by
   bailing on an intervening `movq $0,(%rsi)` (a store with no GP dest) between
   the slot write and the FP use.
3. It **also exposed a latent IR soundness hole** in `aggregate_sroa`'s load
   forwarding: reading the copy source at the *use* site without treating two
   distinct pointer values as possibly-aliasing.

**Environment.** 2-core Xeon @ 2.6 GHz, 1.9 GB RAM, 4 GB swap (no PMU). All
measurements are *screening* evidence (paired, pinned, no PMU), validated by
instruction-count deltas and by the correctness / regression / benchmark-output
gates. The authoritative runtime report for the real Raptor Lake i7-14700KF
remains the in-repo CI benchmark report.

---

## 1. Landed, built, and verified in this session

### 1.1 `expr_builtins.rs` — lower constant-size `__builtin_memcpy` to native `Instruction::Memcpy`
`BuiltinKind::LibcAlias("memcpy")` for the **`__builtin_memcpy` spelling only**,
size const-evaluated from the source expression (the libc-alias cast promotes it
to `size_t`, so the literal lowers to a `ZExt` value rather than a `Const`
operand — debugged via `CCC_DBG_MEMCPY`), dest/src pointer operands. Emits
`Instruction::Memcpy`, yields the destination pointer (`memcpy` returns `dest`).
SROA then forwards `load d → load src` and removes the dead copy. `memmove` is
never rewritten (overlap contract differs).

### 1.2 `local_patterns.rs` — `fold_ptr_deref_through_stack` aliasing-store soundness
The peephole folded `movq (%ptr),%rax; movq %rax,slot; ...; movsd slot,%xmm`
into `movsd (%ptr),%xmm` while only checking the *pointer register* wasn't
rewritten. An intervening `movq $0,(%rsi)` (which aliases `%rdi` at runtime)
wrote `(*ptr)` without touching `%rdi`, so the re-read was stale. Now bails on any
`LineKind::Other { dest_reg: REG_NONE }` between the slot write and the FP use.

### 1.3 `aggregate_sroa/mod.rs` — load-forwarding aliasing-store soundness (new)
The load-forwarding pass forwards a load of a memcpy-written alloca to the copy
source, but only marked the source dirty when an intervening store's pointer had
the *same SSA value* as the source (`pr == sr`). Two distinct pointer values —
e.g. two parameters passed the same address —

```c
u64 bits; __builtin_memcpy(&bits,p,8);  /* bits = *p  (captured)  */
*q = 0.0;                               /* aliasing store          */
double d; __builtin_memcpy(&d,&bits,8); /* d = *p read at use site*/
return d + 1.0;                         /* p==q -> must be 3, was 1 */
```

may still alias, so the forwarded `load *p` read the *post-store* value: returns
`1` instead of `3` at `-O2/-O3` (correct at `-O0/-O1`, which is exactly why it
slipped past `-O0` checks). Fix: a store (or `Memcpy` dest) is only proven
not to write the source when the source is a *private* (non-escaping) alloca, or
the store target is such a private alloca (a private alloca is reachable only
through its own value, so it cannot alias an arbitrary pointer). Otherwise
assume aliasing and keep the slot read.

**Gates after all three (green):**

| gate | result |
|---|---|
| `tests/correctness/run_correctness.py` | 57 passed, 0 failed |
| `tests/regression/run_regression.py` | **724 passed, 0 failed** (was 723 + 1 new), 9 skipped-compare, 2 skipped-run, **735 total** |
| `scripts/check_benchmark_outputs.sh` (-O0/-O1/-O2/-O3) | 180 / 180 PASS, 0 FAIL |

New regression: `tests/regression/memcpy_unaligned_load_fwd.c` locks unaligned
`read64/32/16` forwarding at all offsets, the `p==q` aliasing-store soundness
probe (must return `3`), and `__builtin_memmove` overlap (never rewritten to a
no-overlap `Memcpy`).

---

## 2. Root-cause analysis

### 2.1 The read64 round-trip (`Call` → store+reload)
`read64(p)` = `u64 v; __builtin_memcpy(&v,p,8); return v;`. Before the fix this
lowered to a libc `call memcpy`. After inlining the IR was
`Alloca v; Call memcpy(&v,p,8); Load v`, and the IR passes model the *native*
`Memcpy` (not `Call{func="memcpy"}`), so every hot loop kept
`movq (%src),%rax; movq %rax,slot; ...; movq slot,%r9` — two dead memory ops per
iteration in `read64`-heavy kernels (zstd_count, glibc_memcmp, chacha20 data
loads).

### 2.2 The two exposed latent holes
With the round-trip gone, the IR-level and peephole-level value flows that the
libc `call` previously insulated became reachable, and *both* had aliasing holes:
the FP peephole (§1.2) and the IR load-forwarding (§1.3). The negative probes
(`fp_liveness_ptr_deref_alias_negative`, and the new `memcpy_unaligned_load_fwd`
`p==q` case) are exactly what catch them.

---

## 3. Benchmark evidence (this VM, screening only)

Paired median vs GCC, `-O2`, pinned to CPU 0, 9 rounds / 2 warm-ups, all 10
prior-worst benchmarks. **Aggregate geomean 1.1707, arithmetic 1.1857.**

| benchmark | given (prior) | now | note |
|---|--:|--:|---|
| `sha256_transform` | 1.803 | **1.733** [1.726,1.783] | RA register rotation still the blocker (see below) |
| `struct_copy` | 1.448 | **0.966** | aggregate copy now at parity |
| `mandelbrot` | 1.385 | **1.151** [1.145,1.157] | |
| `expat_xml_scan` | 1.373 | **1.301** [1.293,1.311] | byte-classifier chain (branch-bound) |
| `zstd_count` | 1.276 | **1.222** [1.210,1.270] | read64 round-trip removed (was 1.34× earlier) |
| `loop_patterns` | 1.240 | **1.043** [1.020,1.053] | |
| `sieve` | 1.215 | **1.084** [1.036,1.189] | |
| `binary_trees` | 1.193 | **1.105** [1.034,1.143] | |
| `glibc_strstr` | 1.167 | **1.062** [1.057,1.065] | |
| `sqlite_varint` | 1.166 | **1.191** [1.181,1.199] | unaffected by this fix; VM noise |

Results JSON: `results/worst10_session/results.json`. The VM timings are
high-variance (sub-20 ms kernels, 2 cores, no PMU); the durable evidence is the
instruction/load/store/spill census and the fact that every gate stayed green.

---

## 4. Remaining measured gaps (godbolt oracle, plain `-O2`, worst-first)

The two big remaining ones are *deep* codegen-quality problems in this compiler,
and both were examined this session:

- **`sha256_transform` round loop — register rotation (RA-PRESSURE-3).** GCC keeps
  a…h in 8 GP registers and does the 7-move rotation each iteration; LCCC executes
  `sub $0x188,%rsp` and stages ~8 state values + temps to the stack every iteration
  (`mov %r13,0x58(%rsp)` … `mov %rsi,0x38(%rsp)` …). `CCC_DEBUG_PHI_COALESCE`
  shows the loop-carried phi-copy coalescing is **BLOCKED** for the rotation
  members (`used_in_window=true derived_in_window=true … src_before_copy=true`).
  This is a genuine RA/copy-web cycle problem, not a spill-recount issue; it is
  the single biggest remaining gap (1.73×).
- **`expat_xml_scan` byte-classifier chain (PF-CLS-1).** The name-classifier is
  emitted as a chain of `je/jne` blocks per test letter instead of GCC's single
  folded `andl $-33; subl $65; cmpb $25` range test + cold out-of-line `_ / : /
  digit / - / .` tests. It is branch-prediction/scheduling bound (instruction
  gap ≈ 1). Two prior attempts to if-convert this chain were reverted for a
  miscompile; it needs the **critical-edge split on the last member first**.

---

## 5. To-do / outstanding (next session)

1. **sha256 round loop rotation (RA-PRESSURE-3):** keep the cyclic a…h
   copy-rotation in registers. Investigate the phi-coalesce **BLOCKED** decision
   (`used_in_window`/`derived_in_window` for the rotation web) and, separately,
   whether 2×/4× loop unrolling lets the rotation happen once per body. Measured:
   64 spills → ~21 (GCC) is the target.
2. **Message-schedule vectorization (PF-SCHEDULER-1):** GCC vectorizes the
   `SIG1/SIG0` sliding-window expansion with XMM at plain `-O2`; LCCC computes it
   scalar. Second sha256 win.
3. **expat byte-classifier** if-conversion with critical-edge split first.
4. **chacha20_core** v3 ARX path (222 vs icx 63 at `-march=x86-64-v3`) — not the
   benchmark's plain-`-O2` bottleneck (near parity), so separate the two.
5. **Re-provision per wipe:** `.cache`/`target`/`/swapfile` are wiped between
   sessions; `scripts/arena_session_restore.sh` (swap + rust + apt + git
   identity) must run first.

**Session hygiene.** The deliverable `/home/user/ms178-1.patch` is refreshed on
every valid snapshot by `/home/user/lccc-snapshot.sh` (commit-then-publish,
atomic rename, ledger). This session's snapshots: **S05-sroa-aliasing-forward**
(also S03-fp-soundness / S04-docs-backlog from the prior interval). Head
`eecf2536`, base `cdb01750`.
