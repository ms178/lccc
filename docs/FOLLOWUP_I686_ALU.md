# Follow-up work — i686 ALU lowering and surroundings

*Maintained per session. Read `I686_ALU_AUDIT.md` first for the evidence.
Every item has: evidence, expected gain, affected workloads, difficulty,
where to start.*

## Accomplished this session (2026-09-03, base `82d7009`)

| id | item | validation |
|----|------|-----------|
| A-1 | `scripts/i686_alu_redteam.py` — deterministic 3 122-function differential tester for every `alu.rs` lowering, sharded, parallel, per-function hashes, `gcc -O0`/`-O2` cross-checked ground truth, `--stats` census | PASS ×5 levels before and after |
| A-2 | `synth_mul`: exhaustive bounded-search multiply-by-constant synthesiser (memoised) replacing the 12-entry table; accumulator path reads register-homed lhs in place | unit tests (1 400 constants × shapes × budgets), red-team, oracle |
| A-3 | Constant remainder without stack traffic (`emit_udiv_magic_keep_n`, `emit_sdiv_magic_core`) | red-team, oracle |
| A-4 | Constant div+rem pair fusion on i686 (`emit_divrem_const_in_eax_edx`, analysis gate `DivRemTarget::I686`) | red-team incl. rem-first, q-used-between, pathological-home shapes |
| A-5 | `scripts/lccc-harness-snapshot.sh` — patch/ledger/docs embedded into the web bundle after every validated step | ledger |
| A-6 | Compile-time superlinearity measured and recorded (F-6) | probe in audit §4.3 |

## Open items, priority-ordered (impact × breadth × confidence ÷ cost)

### F-1 Leaf-function prologue and argument homing (i686) — **largest remaining gap**
* Evidence: every one-argument leaf in the census: `pushl %ebx; subl $8,%esp;
  movl 16(%esp),%ebx; …; addl $8,%esp; popl %ebx` vs GCC `movl 4(%esp),%eax`.
  Census ratio after this patch: divide groups ≈1.7×, `mul` 2.8×, shifts 2.7×
  — almost entirely frame + homing, not ALU.
* Expected gain: 3–6 instructions per leaf call, fewer callee-save stores;
  measurable on zlib-ng/expat hot leaves compiled `-m32`.
* Where: `prologue.rs` (stack size / callee-save selection),
  `regalloc.rs` (prefer caller-saved `%eax/%ecx/%edx` for short-lived
  arguments in leaves; avoid the `subl $8` alignment pad when no call exists).
* Difficulty: medium–high (ABI, alignment, the accumulator-cache model).

### F-2 Redundant `andl $31` on variable shift counts
* Evidence: `vshl`: `andl $31,%eax` before `shll %cl` (hardware masks to 5
  bits) — `rotl/rotr` idioms and every `x << (n & 31)` in hashing code.
* Fix options: (a) IR demanded-bits: `Shl x, (And n, 31)` → `Shl x, n` is
  legal only if IR shifts are defined as masking on this target (document
  it), or (b) an i686 isel def-map (the emitter has no def lookup today —
  add `value_def: FxHashMap<u32, (block, idx)>` built in `prologue.rs`).
* Expected gain: 1 insn + 1 µop per variable shift; small but broad.

### F-3 Rotate recognition (`(x << n) | (x >> ((32-n)&31))` → `roll %cl`)
* Evidence: `rotl` 26 insns vs GCC 4. Needs the def map from F-2 or an IR
  `Rotl/Rotr` op (the x86-64 backend should share it; check `isel.rs`).
* Workloads: hash functions (FNV/murmur/xxhash), crypto in OpenSSL, zlib-ng
  `crc32` fold helpers, SQLite `hash`.

### F-4 `sdiv`/`udiv` from a register-homed lhs without the `%eax` round trip
* Evidence: `movl %ebx,%eax; movl %eax,%ecx; movl $M,%eax; imull %ecx` —
  with lhs in `%ebx` the sequence can be `movl $M,%eax; imull %ebx` and use
  `%ebx` as the preserved n (`negl %eax; addl %ebx,%eax` for the remainder).
  Saves 1–2 movs (rename-eliminated, but still front-end slots).
* Where: the four `emit_*_const_in_eax` helpers take a `n_reg: &str`.

### F-5 Direct-path constant division into a register-homed dest
* Evidence: `try_emit_int_binop_direct` refuses div/rem; the result goes
  `%eax → store_eax_to(dest)`. When `dest` is register-homed and ∉
  {eax,ecx,edx}, the magic sequence could end with `movl %eax,%dest`
  directly (it does today via `store_eax_to`, but the acc cache then keeps
  `%eax` = dest and the next op often re-stages). Audit the acc-cache
  interaction with a census of `movl %eax,%r; movl %r,%eax` pairs.

### F-6 Compile-time superlinearity in large functions
* Evidence: 100/200/400 loops in one `main` → 0.1/0.2/0.6 s and 18/27/46 MiB
  at `-O0`; a 1 900-loop `main` exceeds 1.6 GiB. Profile with
  `perf record`-less tooling: `CCC_TIMING`-style timers per pass, or
  `valgrind --tool=callgrind` on the 400-loop probe (see
  `scripts/i686_alu_redteam.py --shard 100000` to regenerate the monolithic TU).
  Suspects: per-block bitset liveness (O(blocks × values)), the accumulator
  chain analysis, `compute_i686_divrem_pairs` being recomputed **three
  times** per function (`regalloc.rs:64`, `:673`, `:5588`, `prologue.rs`)
  — cache it in the function context.

### F-7 Signed pow-2 pair: shorter form
* `emit_divrem_const_in_eax_edx` signed pow-2 emits 8 insns; `r = n − (q << k)`
  from the biased value could save one `movl` when `%edx` may be
  reused. Low priority (rare shape: `x/16` together with `x%16` signed).

### F-8 Three-step chains in the *in-place* multiplier path
* Today `same == true` (dest is the accumulator) is limited to 2 steps
  because no scratch register is admissible. `%ecx` is `Mul`-clean in the RA
  model only for the *immediate* form; extending the model to grant `%ecx`
  as scratch for `Mul` by immediate on the accumulator path would unlock
  7/11/13/17/19/21/… there too. Requires the hazard-model change at
  `regalloc.rs` (`(const_imm(rhs), true)` for `Mul`) — measure first with the
  census: how many `imull $c,%eax,%eax` remain after this patch.

### F-9 Extend the red-team tester
* Add `-mpopcnt -mlzcnt` runs to CI (`--extra`), a `--fuzz N` mode that
  re-seeds the random trees per run (csmith is not installed in the sandbox
  — `apt-get install csmith` and reuse `scripts/csmith_diff.py` with
  `-m32`), and 64-bit (`long long`) operands to cover the wide-value paths
  that `binop_loc` rejects.

## A/B protocol used this session (no PMU available)

1. Build the upstream binary (`git stash; scripts/build_lccc_fast.sh; cp
   target/fastbuild/lccc-i686 /tmp/lccc-i686-base; git stash pop; rebuild`).
2. `tests/benchmark/programs/i686_alu_chains.c` — latency-bound dependent
   chains (`x = x % 7 + i`, div+rem pairs, `x = x * 7 ^ i`, …) with a checksum
   so both binaries must agree; 5 repetitions, report min wall-clock.
3. Static: `scripts/i686_alu_redteam.py --stats` before/after (census in the
   audit).
4. Oracle: `scripts/godbolt.py compile {gcc16.2,clang,icx} kernel.c --flags
   '-m32 -O2'`.

## Session hygiene (harness)

* Swap: `scripts/ensure_swap.sh` (4–8 GiB) **before** any cargo build.
* Build: `scripts/build_lccc_fast.sh` (fastbuild profile, `-j2`).
* Snapshot after every validated step: `scripts/lccc-harness-snapshot.sh
  "<slug>" "<desc>"` — the patch lands in `public/patches/ms178-1.patch`,
  `src/generated/patch.ts`, and `dist/index.html`.
* Long commands must run under `nohup … &` and be polled; the tool call
  budget is ~2 min per call.
