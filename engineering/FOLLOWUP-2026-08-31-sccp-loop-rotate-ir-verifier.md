# Follow-up — 2026-08-31 — SCCP adoption, the `loop_rotate` stale-phi miscompile, and the new IR verifier

Status at end of session: **1369 unit tests + 553/553 regression tests, 0 failures, 0 A/B diffs.**
Snapshot `S03-loop-rotate-stale-phi-and-ir-verifier`, base `1a9bf60`.

---

## 1. What was accomplished

### 1.1 SCCP + use-def chains adopted (audit of `CrazyTodd-one/claudes-c-compiler`)

`src/passes/sccp.rs` (Wegman–Zadeck sparse conditional constant propagation) and
`src/passes/use_def.rs` (CSR use-def chains) were ported, red-teamed, and hardened.
Seven genuine defects in the upstream implementation were found and fixed before
adoption; each has a minimal-IR regression test in `src/passes/sccp/tests.rs`
(29 tests). See `engineering/SCCP_ADOPTION_AUDIT.md` for the full findings table
(F1, F2/F8, F3, F4, F20, F23, plus the verified non-issues F5/F24/F25/F26).

Integration (`src/passes/mod.rs`), deliberately **without** adding a 12th phase index
(`NUM_PASSES` stays 11, so no `should_run!` renumbering):

* main loop **phase 4**, immediately before `constant_fold`, gated `opt_level >= 2`;
* `run_inline_phase`, immediately before the cleanup `cfg_simplify` — inlining has
  just substituted literal arguments, so parameter-dependent branches become
  decidable, which `cfg_simplify` alone cannot do (it only folds an *already*
  constant condition).

Kill switch: `CCC_DISABLE_PASSES=sccp`.

### 1.2 The bug SCCP exposed: a pre-existing `loop_rotate` miscompile

Integrating SCCP turned `loop_rotate_default_enable` into a **SIGSEGV**. It was not
an SCCP bug.

**Reproducer** (`tests/regression/loop_rotate_stale_phi_pred.c`, needs `CCC_LOOP_ROTATE=1`;
distilled to 12 lines during debugging). Bisected with the new per-action gates to
phi-operand pruning alone, then to this IR after `loop_rotate`:

```text
block 0 (.LBB6):  term: Branch(BlockId(7))
block 1 (.LBB7):  Phi v22 = [(0, BB6)]                  ; guard   — correct
block 2 (.LBB8):  Phi v72 = [(0, BB6), (v16, BB8)]      ; body    — BB6 is NOT a predecessor
```

Rotation creates the guard (`BB7`) and rewires every original header predecessor onto
it, so the body's only entry edge is `guard → body`. The init incoming was still
labelled with the **original preheader**. `loop_rotate.rs` even documents the hazard
under "Guard C" and its own step-6.6 doc comment specifies the correct shape
(`phi[header_label: v_pre, latch_label: v_latch]`) — but the code wrote `pre_label`.
A one-token code/comment divergence. The existing `Guard C` bail only fires for
*multiple* outside predecessors, so the single-predecessor case was unguarded.

**Why it stayed latent for months:** phi elimination
(`src/ir/mem2reg/phi_eliminate.rs`, `emit_single_phi_copies`) resolves a phi operand's
label to a block *index* and places the init copy there. `BB6` still dominated `BB8`,
so the copy landed somewhere that happened to execute first and the program worked by
accident. SCCP is the first consumer that *trusts the predecessor list*: it proved
the `BB6 → BB8` edge non-existent, pruned the operand, and the IV initialisation
(`xorl %ebx, %ebx`) vanished from the preheader. The loop then indexed `arr` with an
uninitialised `%rbx`.

**Fix** (`src/passes/loop_rotate.rs`, step 6.6): label the init incoming with
`header_label` (the guard) instead of `_pre_label`. `pre_op` is unchanged and still
dominates — it is defined in the preheader, which dominates the guard. This matches
the exit-phi construction a few lines below, which already used `header_label`.

### 1.3 SCCP hardened: prune only edges *proved* dead

Independently of the root cause, SCCP now distinguishes

* "this edge exists in the CFG and I proved it non-executable" → prunable, from
* "this label was never a predecessor of this block" → malformed IR, **keep it**.

A `real_edges` set (all CFG edges as written, including `asm goto`, regardless of
reachability) is built alongside `live_edges`; the prune predicate is
`live_edges.contains(k) || !real_edges.contains(k)`. Malformed input from *any* pass
is now safe, not just this one.

### 1.4 New: `src/passes/verify.rs` — IR structural verifier

lccc had a register-allocator verifier (`CCC_VERIFY_REGALLOC`) but **nothing** for the
mid-level IR. That gap is exactly why a pass could emit malformed IR for months
without a single test noticing. Six invariants:

1. unique block labels;
2. every terminator / `asm goto` target names a real block;
3. phis form a contiguous prefix of their block;
4. **every phi incoming label is a real CFG predecessor** ← the defect above;
5. no duplicate phi predecessors;
6. every real predecessor has an incoming.

Hooked after **every** pass (main loop, pre-loop, and inline phase), so the reported
stage names the pass that *produced* the bad IR, not one that tripped over it later:

```console
$ CCC_LOOP_ROTATE=1 CCC_VERIFY_IR=1 lccc -O2 r1.c -o r1     # unfixed compiler
[ir-verify] after `loop_rotate` in `main`: block #2 (BlockId(8)): phi v72 has an
    incoming from BlockId(6), which is not a predecessor (real predecessors: [7, 8])
[ir-verify] after `loop_rotate` in `main`: block #2 (BlockId(8)): phi v72 has no
    incoming for predecessor BlockId(7)
```

`CCC_VERIFY_IR=1` reports; `CCC_VERIFY_IR=abort` panics on the first violation for a
backtrace. Off by default and zero-cost when off (mode cached in a `OnceLock`;
the disabled path is one atomic load). 10 unit tests in `src/passes/verify/tests.rs`,
including `the_loop_rotate_stale_guard_label_is_caught`, which pins the exact shape.

### 1.5 New: SCCP per-action diagnostic gates

Bisecting *which* of SCCP's rewrites broke a workload took one rebuild instead of a
day. Kept as a permanent facility:

| Variable | Suppresses |
|---|---|
| `CCC_SCCP_NO_PRUNE` | phi-operand pruning on dead edges |
| `CCC_SCCP_NO_DEFS`  | rewriting definitions to `Copy{dest, Const}` |
| `CCC_SCCP_NO_SUBST` | constant substitution into operands |
| `CCC_SCCP_NO_FOLD`  | folding `CondBranch`/`Switch` to `Branch` |
| `CCC_SCCP_TRACE_PRUNE` | prints every pruned phi operand with its edge verdict |

This is how the segfault was localised: `CCC_SCCP_NO_PRUNE=1` was the only gate that
restored correct output.

---

## 2. To do next — ranked, with evidence already gathered

### 2.1 **`loop_unroll` emits the same class of malformed IR (HIGH — do this first)**

A full verifier sweep of all 553 regression tests at `-O2` and `-O3` (both loops in
`/home/user/work/verify/`, script inline in the session log) attributes each violation
to the **first** pass that reports it:

| Kind | First-reporting pass | # configs |
|---|---|---|
| `STALE_PRED` | `loop_unroll` | 112 |
| `STALE_PRED` | `loop_unroll_post_vec` | 107 |
| `PHI_ORDER` | `loop_unroll` | 60 |
| `PHI_ORDER` | `loop_unroll_post_vec` | 57 |
| `MISSING_PRED` | `lowering(pre-O2)` | 16 |
| `MISSING_PRED` | `cleanup-sccp` | 14 |
| `PHI_ORDER` | `if_convert` | 6 |
| `MISSING_PRED` | `cleanup-mem2reg-params` | 6 |
| `STALE_PRED` | `cleanup-sccp` | 4 |
| `MISSING_PRED` | `sccp` | 2 |
| `PHI_ORDER` | `cleanup-cfg-simplify` | 2 |

Representative:

```text
accumulator_reassociation -O3
  after `loop_unroll` in `adler16`: block #3 (BlockId(9)): phi v45 has an incoming
      from BlockId(10), which is not a predecessor (real predecessors: [7])
  after `loop_unroll` in `adler16`: block #3 (BlockId(9)): phi appears after a
      non-phi instruction
```

`loop_unroll` is the **dominant** producer and is enabled by default at `-O2`/`-O3` —
a much larger exposure than `loop_rotate` (which is opt-in via `CCC_LOOP_ROTATE=1`).
The suite is green only because the current consumers are forgiving *and* SCCP is now
hardened. The `PHI_ORDER` violations are independently dangerous: `loop_rotate.rs:412`
and `mem2reg` both assume phis are contiguous at block start.

Method that worked for `loop_rotate`, reuse it verbatim:
`CCC_VERIFY_IR=abort` on the smallest failing test → backtrace names the pass →
`CCC_DUMP_EACH_PASS=1 CCC_DUMP_FUNC=<fn>` to read the before/after IR → fix →
re-sweep and confirm the row drops to 0.

### 2.2 Triage `MISSING_PRED` — decide whether it is a real invariant

`MISSING_PRED` (a real predecessor with no phi incoming) fires for `lowering(pre-O2)`
and `cleanup-*` too. Before fixing anything, **decide whether lccc's IR dialect
permits an implicitly-undef incoming**. If it does, downgrade check 6 to opt-in
(`CCC_VERIFY_IR=strict`) so the signal-to-noise of the default stays high. If it does
not, these are 38 more latent miscompiles. Check `phi_eliminate.rs` behaviour for a
predecessor edge with no matching incoming — if it emits no copy, the register is
live-in undefined, which is the same failure mode as the bug fixed this session.

### 2.3 Relax `loop_rotate`'s Guard C bail (performance)

With init now routed through the guard label, the reason for the `multi_pre_header`
bail (`src/passes/loop_rotate.rs`, ~line 287) is weaker: multiple outside predecessors
all get rewired onto the guard, and the guard's own phis already merge them. Using the
**header phi's dest** (rather than a single `pre_op`) as the body phi's init incoming
should make the multi-predecessor case representable and unlock rotation on loops that
currently bail. Requires its own reproducer + measurement; do not bundle with a
correctness fix.

### 2.4 Carried over from earlier sessions (unchanged)

* Real-workload validation: zlib-ng, gzip, expat. **Run these under `CCC_VERIFY_IR=1`** —
  they exercise far more loop shapes than the regression corpus and will likely surface
  more of 2.1.
* Extend `scripts/lccc-bootstrap.sh` to reinstall rustup **1.98.0** (pinned by
  `rust-toolchain.toml`; the base image ships 1.85, which is too old). Currently the
  only manual step after a harness wipe.
* Godbolt oracle derived from `scripts/godbolt.py`, comparing ICX/ICC, GCC 16.2, latest
  Clang; use it to chase low-risk/high-reward codegen wins.
* Linker oracle: honour the user's build/version preferences (lld 23.1, mold 2.42,
  bfd 2.47); mold must use the X86/i686-only CMake preset.

---

## 3. Environment notes (cost real time this session)

* `/tmp` is **wiped between tool calls**. Scratch must live under `/home/user/`
  (`/home/user/work/red/`, `/home/user/work/verify/`).
* Heredocs (`cat > f <<'EOF'`) fail inside the agent's bash tool; use the file-write
  tool for source files. `python3 - <<'PY'` *does* work.
* `mawk` rejects a multi-line ternary chain; use `python3` for log analysis.
* Build only with the `fastbuild` Cargo profile and `-j2` (1.9 GiB RAM + 6 GiB swap).
* The VM is Ice Lake-SP with AVX-512; the user's real target is **Raptor Lake, which
  has none**. Never tune to this VM's ISA.
* No PMU: use wall-clock with repetition, instruction-count proxies, and static asm
  analysis.
