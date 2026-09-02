# Session 11 — BitTest index-0 miscompile, slot-stress fuzzing, SQLite differential

Base: `6fcbb1d` (upstream main, incl. `bc72818` "stack_layout: size slots from
emitter width; disable unsound Tier-2 sharing").

## 1. `BitTest(base, 0)` folded to zero — real miscompile (FIXED)

### Symptom

`scripts/gen_slot_stress.py` seed 1, `-O2`: lccc and gcc print different
checksums. -O0/-O1 agree, -O2/-O3/-Os diverge.

### Localisation

Instrumenting the reduced case (`/tmp/min/u16_red1_p.c` lineage) showed the two
compilers taking **different arms of the same `if`** while printing identical
`g_sink` just before it:

```
if (((g_sink >> 0u) & 1u) ^ 1) { ... } else { ... }
```

With `g_sink = 0xacc0cec466c569cf` (bit 0 = 1) the condition is false, yet the
final IR passed `Const(I32(0))` as the printf argument for bit 0:

```
printf(.Lstr4, Const(I32(0)), Value(206) /*bit1*/, ... Value(248) /*bit7*/)
```

Bits 1..7 survived as `BitTest` instructions; **bit 0 alone was folded away**.

### Root cause — `src/passes/simplify.rs`, `IrBinOp::BitTest` arm

```rust
// `(base >> index) & 1`: a zero base or zero bit index always
// yields zero.                     <-- WRONG for index == 0
if rhs_zero || lhs_zero { return Copy(0); }
```

`BitTest(base, index)` is `(base >> index) & 1` (see `IrBinOp::eval` in
`src/ir/ops.rs` and `fold_binop` in `src/passes/constant_fold.rs`, both of which
are correct). For `index == 0` that is `base & 1` — **bit zero of the base**, 1
for every odd base. The rule was copy-pasted from the shift arms above it, where
`rhs_zero` legitimately means "`x << 0` is `x`".

### Fix

Only a zero *base* folds to zero; a zero *index* is left alone (keeping the
`BitTest` lets x86 still select `BT`). Three unit tests in
`src/passes/simplify.rs` pin the semantics: `bit_test_zero_base_is_zero`,
`bit_test_zero_index_is_bit_zero_of_base`, `bit_test_zero_base_and_index_is_zero`
(the previous `bit_test_zero_operand_is_zero` asserted the buggy behaviour and
was replaced).

### Verification

* `cargo test --lib bit_test` → 5 passed, 0 failed.
* New `tests/regression/bittest_index_zero.c` → PASS at -O0/-O1/-O2/-O3/-Os,
  identical to gcc.
* `scripts/run_regression_suite.sh` → **PASS=572 FAIL=0 SKIP=15, AB-diff 0**.
* `scripts/run_slot_stress.sh 1 12 -O2 -Os` → **PASS=96 FAIL=0 SKIP=0** (each
  seed × opt level × 4 slot configurations).

## 2. New test infrastructure (in-tree)

* `scripts/gen_slot_stress.py` — seed-deterministic generator of UB-free C
  programs that stress stack-slot allocation: many simultaneously live values of
  9 widths (1/2/3/4/8/12/16/24/32 bytes incl. `long double`, 16-byte vectors and
  3-byte structs) across if/else diamonds, loops, switches, call barriers, and
  address-taken / volatile / alloca / inline-asm / setjmp / VLA / bit-test
  regions. Every program prints one 64-bit checksum; all arithmetic is unsigned
  so any gcc divergence is a real miscompile.
* `scripts/run_slot_stress.sh` — four-way differential per case
  (default, `CCC_NO_SMALL_SLOTS=1`, `CCC_TIER2_GRAPH=1`, both) × opt levels,
  against gcc. A layout that is only correct when sharing is disabled is a
  miscompile waiting for the right TU; this harness says so.

## 3. Tier-2 liveness-packed slot sharing: status

The disable in `bc72818` is **not** re-enabled by this session. What changed is
the evidence:

* The -O0/-O1 pass, -O2/-O3/-Os fail profile of the original ZSTD
  `corruption_detected` failure matches the BitTest bug exactly, and the ZSTD
  HUF decoder is dense in `bitstream` bit tests. The BitTest bug is a
  sufficient cause; whether it is *the* cause cannot be settled because the
  4 MB `vmlinux.bin.zst` payload and the prepared kernel tree were destroyed by
  a sandbox wipe (see §5).
* With the fix, Tier-2 ON is clean on: the 572-case regression suite
  (PASS=572 FAIL=0), the 96-run slot-stress corpus, and the geometric
  interference proof now uses convex hulls (upstream `bc72818`), which can only
  make more values interfere.
* **Re-enable criteria (next session):** rebuild the payload, run
  `scripts/zstd_oracle_cc.sh` at -O0..-O3 with `CCC_TIER2_GRAPH=1`, and require
  MATCH. Until that reproduces, flipping the default would be an assertion, not
  a proof. The measured cost of the fallback is tiny (+6.4 % frame on the zstd
  TU, +6 B on the boot corpus), so the stop-gap is not buying correctness at a
  real price — but it is still a stop-gap and must be retired.

## 4. NEW, OPEN: lccc miscompiles SQLite 3.50.4 at -O1 and -O2

Found while using SQLite as a large real-world differential oracle (a standing
test case for the codegen-supremacy programme):

```
/home/user/oracle/mini.c + sqlite-amalgamation-3500400/sqlite3.c
gcc  -O2 → open=0 create=0 insert=0 prepare=0 row=42        (correct)
lccc -O0 → open=0 create=0 insert=0 prepare=0 row=42        (correct)
lccc -O1 → SIGSEGV (no output)
lccc -O2 → create=11 err="malformed database schema (?)"    (SQLITE_CORRUPT)
```

The `(?)` is the table-name argument of the error message coming out empty, so
the corruption is inside the schema parse/prepare path, not in the caller. The
full `sqmain.c` workload (5,000 inserts, GROUP BY aggregate, index, self-join)
fails at the first `sqlite3_exec` under lccc and produces 5,003 lines of correct
output under gcc. **This is the highest-value next target**: SQLite is 250 kLOC
of dense integer/bit/string code and is a far better fuzz oracle than anything
currently in-tree. Triage plan: bisect by opt level (the -O0 pass narrows it to
an optimization pass), then `CCC_DISABLE_PASSES` over the -O1 pass list, then
reduce `mini.c` to the failing statement.

## 5. Environment note

`/home/user/lccc/.git`, `target/`, `/home/user/.cargo` and the kernel trees were
wiped mid-session again. Recovery: restore `.git` from
`/home/user/artifacts/lccc-git.tgz`, `rustup` 1.98.0 fresh install, 4 GiB swap
at `/swapfile` (the `fastbuild` link OOMs at 2 GB RAM without it — SIGKILL,
signal 9), `sudo apt-get install zstd`, and re-download
`linux-6.18.47.tar.xz` extracting only `lib/`, `include/`, `arch/x86/include/`.
The git bundle in `/home/user/artifacts` is **not** self-contained (it
references parents outside the shallow clone) and cannot restore the repo.

## 7. Rebase onto upstream `5633355` (PR #359 + #360) and full re-validation

Upstream moved twice while this session ran:

* `2146d77` / `ab9c11a` (PR #359) — x86 peephole: fold FP loads past register
  redefinition, hoist repeated pool constants;
* `5633355` / `983a635` (PR #360) — "Apply ms178-1.patch: MachInst
  flush-shadow fix, small-slot copy gate, size-aware block layout"
  (`emit.rs`, `isel.rs`, `peephole/passes/memory_fold.rs`, `prologue.rs`,
  `pipeline.rs`, `block_layout.rs`; +926/-33).

My work was rebased with `git rebase --onto 5633355 6fcbb1d`; the delta is
conflict-free and shrank to exactly my five files (639 insertions, 12
deletions) because the earlier patch content was already merged upstream.

Rebuilt with `cargo build --profile fastbuild -j2` (1m23s, 4 GiB swap active)
and re-ran the whole battery **on the rebased tree**:

| battery | result |
|---|---|
| `cargo test --lib bit_test` | 5 passed, 0 failed |
| `scripts/run_regression_suite.sh` | **PASS=572 FAIL=0 SKIP=15 (AB-diff 0)** |
| `scripts/run_slot_stress.sh 1 10 -O1 -O2 -Os` | **PASS=120 FAIL=0 SKIP=0** |
| (earlier this session, pre-rebase base) `13 24 -O1 -O2 -Os` | PASS=144 FAIL=0 SKIP=0 |
