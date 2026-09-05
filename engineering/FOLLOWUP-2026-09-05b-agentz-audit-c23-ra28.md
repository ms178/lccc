# Follow-up work — 2026-09-05 (session 2) — Agent Z audit, C23 dialect, RA-28 resolution

Base: `ms178/lccc` main @ `1883599` (contains PR #412: the five miscompile
fixes and the position-relative RA cost model from session 1).
Deliverable: `ms178-1.patch`. Audit: `engineering/AUDIT-2026-09-05-agentz-followup.md`.

---

## 1. Accomplished

### 1.1 Agent Z's follow-up patch: audited, adopted in part, hardened

Full reasoning in `AUDIT-2026-09-05-agentz-followup.md`. Summary:

**Adopted** — four of six commits are real improvements on upstream:
* `canonical_addr_key_impl` now records the `Shl` scale, fixing a real
  address-aliasing miscompile (`d[i]` and `d[2*i]` shared a key, so an arm's
  load could be rewritten to the other's address and two stores to different
  addresses could be merged).
* If-conversion speculation moved from a **shape** heuristic ("IV-addressed
  inside a loop") to the correct **dereferenceability** rule (the address must
  be touched on every path pred→merge). The old gate speculated loads the
  scalar program never performs.
* `fp_copy_web_groups` — coalescing copy-connected scalar-FP webs so a
  reduction's combine→carry handoff stops bouncing through a stack slot.
* `vex_promote` BFS liveness, direct-emit, and the phi-coalesce peel gating.

**Hardened** — three residual soundness holes closed here:
1. Speculation coverage compared address *keys*, not access *extent*. A 1-byte
   covering dereference licensed an 8-byte speculated load; with the address
   on the last byte of a mapping the scalar is fine and the converted form
   faults. `block_deref_keys` now returns the widest access per key and
   `arm_load_speculation_ok` requires `covering_width >= load_width`.
2. The address-key walk descended through **truncating** casts, so
   `d[(int) big]` and `d[big]` collapsed to one key. Agent Z fixed the `Shl`
   half of injectivity and missed the `Cast` half in the same walk.
3. `fp_copy_web_groups`'s interference validation failed **open**: a member
   with no recorded segment was `continue`d, i.e. "no data" was read as "no
   interference" — exactly backwards for a soundness gate. Now `return false`.

**Rejected** — the test-harness changes:
* Four `.txt` sidecars containing `LCCC_NO_COMPARE=1`. The runner reads
  `.env`, not `.txt`, so the files are **inert**; all four tests pass with
  full oracle comparison. Removed as dead weight.
* `vectorize_map_expr_tree.flags` rewritten from
  `-O3 -march=x86-64-v3 -ffp-contract=fast` to `-lm`, which drops a
  *vectorization* test to default `-O0` so the vectorizer never runs.
  Restored to the original flags **plus** `-lm` — and it passes, so the
  destructive edit was never necessary.
* The `VecMaskedAddI32x8` width fix is redundant (already upstream in PR #412,
  which also fixed the three sibling instances and the scalar-result
  violation in the same list).

New adversarial test `tests/regression/ifconv_speculation_adversarial.c`
covers the two speculation holes with a page-guarded allocation (so an
over-eager hoist faults rather than silently reading adjacent heap), plus
scale-key and truncating-cast injectivity and the covered shape that must
still convert.

### 1.2 RA-28 resolved with data: eviction mode 6 stays opt-in

New tool `scripts/ra_ab_census.py` — a static, deterministic A/B census of one
RA knob against itself (same binary both sides), reporting `insns` / `rrmov` /
`stkref` / `push` per function. On a 2-core VM with no PMU this is the only
measurement that separates signal from scheduling noise.

First result over `tests/benchmark/{programs,kernel_corpus}`:

| bucket | whole corpus | driver (`main`) | **hot kernels** |
|--------|--------------|-----------------|-----------------|
| stkref | −9 (−1.21 %) | −26 | **+17** |
| rrmov  | −20 (−1.85 %) | −16 | −4 |
| insns  | −22 (−0.30 %) | −38 | **+16** |

Identical at `-O2` and `-O3`. The aggregate looks like a win and **is not
one**: all of the improvement is in benchmark setup/timing code, and hot
kernels get worse (`arith_loop` alone is +15 stkref / +9 insns). The census
tool was extended to report the split and to base its verdict on the kernel
total — the first version's "no regression" was an artifact of weighting cold
`main` equally with the kernel.

**Decision: `CCC_EVICT_MODE` default stays 3.** The project's own guard-rails
(gzip and the Adler DO8 loop must not regress) are the reason mode 5 and both
dead-victim variants were reverted; flipping on an aggregate number would
repeat that mistake.

**The interesting part is *why*** (BACKLOG RA-28b): under pure lifetime
demotion, a *more accurate* cost model evicts more readily, and every eviction
is permanent — so an unrolled accumulator chain pays all of the victim's later
uses in memory. Mode 6 wins exactly where lifetimes are long and pressure is
high (`struct_copy` −51 stkref) and loses where a nearly-dead partial sum is
re-read after the loop. **Mode 6 is a prerequisite for RA-06 (splitting), not
a standalone win**; re-measure after splitting lands and the sign should flip,
because the victim will be reloaded at its next use instead of demoted for its
whole lifetime.

### 1.3 C23 dialect support: two real frontend defects

`-std=` was parsed but only ever set `gnu_extensions` and `gnu89_inline`.
`__STDC_VERSION__` stayed pinned at `201710L` for **every** dialect, so every
C23-conditional header took its pre-C23 branch even under `-std=c23`.

* `-std=` now selects `__STDC_VERSION__` with the full GCC mapping (C89/C90
  define nothing, `iso9899:199409` → `199409L`, C99/C11/C17/C23 →
  `199901L`/`201112L`/`201710L`/`202311L`, C2y → `202400L`), applied **before**
  the user's `-D`s so an explicit override still wins.
* lccc injects its own `<stdarg.h>` macros, and `va_start` was a fixed
  two-parameter macro, so C23's one-argument `va_start(ap)` expanded to
  `__builtin_va_start(ap,)` and died in the parser. It is now variadic;
  `__builtin_va_start`'s lowering already read only `args.first()` (the second
  argument is not evaluated in any C dialect), so both forms lower identically.

This unblocks `gcc.c-torture/execute/pr117432.c` at every optimisation level —
a test the **host GCC 14.2 cannot compile at all**.

### 1.4 Torture suite: what "100 %" actually means here

Every remaining non-`pass` case on both targets is a **reference skip** — the
host GCC 14.2 could not establish a reference — not an lccc failure:

| test | host GCC 14.2 | lccc |
|------|---------------|------|
| `pr118623` | **aborts** (bug fixed in GCC 15) | **passes** |
| `pr118915` | **aborts** (bug fixed in GCC 15) | **passes** |
| `pr119291` | **aborts** (bug fixed in GCC 15) | **passes** |
| `pr117432` | cannot compile (no C23 `va_start`) | **passes** with `-std=c23` |

So on every test where a reference is establishable, lccc passes; and on four
tests lccc is *correct where the reference compiler is not*. Raising the
measured rate further requires a newer host GCC, not compiler work.

---

## 2. Validation

| gate | result |
|------|--------|
| `run_regression_suite.sh` | **630 PASS / 0 FAIL / 7 SKIP**, AB-diff 0 |
| `gcc.c-torture/execute` x86-64, `-O0..-Os` (8380) | 8368 pass, **0 run-fail** |
| `gcc.c-torture/execute` i686, `-O0..-Os` (8380) | 8355 pass, **0 run-fail** |
| `ra_ab_census.py` | new tool; mode-6 verdict recorded above |

---

## 3. To-do, in priority order

1. **RA-06 reload-at-next-use / in-place splitting** — still the #1 gap, and
   now with a measured reason to do it first (RA-28b). The cost model and the
   next-use machinery it needs are already in place.
2. **RA-32** — prove or make conservative the merged FP-web leader's segment
   coverage: the leader's interval is the fat `[min start, max end]` while
   `attach_scan_segments` only sees the leader's own segments, so a member's
   live hole may look free to the scan.
3. **IC-05** — audit `sink_conditional_stores` against the same two properties
   the load path now enforces (access extent, cast injectivity in the key).
4. **FE-30** — decide whether the default dialect should move to `gnu23`
   (GCC 15 and Clang 19 have). This changes semantics (K&R definitions,
   `bool`, `nullptr`) and needs its own full torture matrix; record the
   decision rather than guessing.
5. **RA-29/30/31** — rematerialization in `spill_cost_at`, the
   `occupied_points()` spill-weight denominator, and per-use fold cost classes.
6. Godbolt oracle comparison runs; the "10 worst results" performance table.
   The oracle is reachable from the sandbox (`scripts/godbolt.py list` resolves
   `cg162`).

---

## 4. Environment notes

* **The harness wiped the worktree during this session.** What survived:
  `/home/user/artifacts/*` and `/home/user/ms178-1.patch`. What did **not**:
  `.git`, `~/.cargo` (Rust), the extracted torture corpus, and — because the
  snapshot is file-count/size capped — most of `src/`. The `lccc.bundle` was
  **not** restorable (`git clone` of it fails: the source repo was a shallow
  clone, so the bundle has no complete history). Recovery that worked, and the
  one to use next time: re-clone upstream and apply `ms178-1.patch`, which is
  verified `APPLIES-CLEAN` on every snapshot. Keep doing that; do not rely on
  the bundle.
* Rust 1.98.0 via rustup; **every new shell needs
  `export PATH=/home/user/.cargo/bin:$PATH`**.
* `gcc-multilib` + `libc6-dev-i386` must be reinstalled after a wipe or the
  i686 torture runner passes **vacuously** (1676 `reference-compile-skip`).
* `/home/user/dl/gcc-15.1.0.tar.xz` survives wipes, so
  `scripts/ensure_gcc_torture.sh` re-extracts the corpus in seconds with no
  download.
* Full 5-level torture run: ~10 min per target at `-j2`. Incremental rebuild:
  10–45 s. **Do not rebuild while a torture run is in flight** — the runner
  reads `target/fastbuild/lccc` per test and the results become meaningless.
