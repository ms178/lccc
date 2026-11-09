# Perfected pass — BB-SLP object identity, measured cost model, and SLP feedstock

**Branch:** `arena/9546a706-lccc`  **Base:** `df2fdfbb80c9bb7f3f370a240e6e1c40d5c96da1`
**Date:** 2026-11-08  **Scope:** `src/passes/slp_vectorizer.rs`, `src/passes/slp_vectorizer/joint.rs`,
`src/passes/loop_unroll.rs`, `tests/simd/*`.

> **Honesty statement.** This document reports what was measured, including
> measurements that REFUTE earlier claims of mine.  Wall-clock A/B on this
> shared 2-vCPU host was proven biased (a null experiment with the *same
> binary twice* reported 0.9278 and 0.9643 instead of 1.0).  Every headline
> performance claim below therefore rests on **deterministic Callgrind
> instruction counts**, not clock timings.  The overall goal "beat GCC 16.2 /
> Clang 23.1.5 / ICX 2026.2 on runtime" is **NOT achieved** by this PR:
> deterministic Ir geomean is 3.3727x the best-of-three, LCCC wins 1/29
> shapes.  This PR closes one root cause, fixes a measured regression, and
> adds the gates needed to make the next gains provable.  Claiming otherwise
> would be handwaving.

---

## 1. Method, toolchain, and environment

| Item | Value | Evidence |
|---|---|---|
| Build | `cargo build --profile fastbuild --locked -j1 --bin lccc --bin lccc-ld`, `RUSTFLAGS="-D warnings -C link-arg=-Wl,--no-keep-memory"` | build logs `BUILD_OK 21:39:07Z`, `21:47:19Z`, final build this session |
| Swap | 4 GiB `/home/user/.cache/lccc-swap.img` | `/proc/swaps`: `4194300 0 -2` |
| GCC | `gcc-16 (GCC) 16.2.0` | `--version` |
| Clang | `clang version 23.1.5 (llvm/llvm-project 392dfff97d29b…)` | `--version` |
| ICX 2026.2 | **NOT OBTAINED** | see §1.1; no claim of use |
| GDB | GNU gdb 18.1 | breakpoint at `s_jhash`, register witness (§5.4) |
| objdump/readelf/nm/size | binutils 2.47 from `harness-cache-8` | §5 |
| Valgrind | 3.19.0 extracted from cached `valgrind_1:3.19.0-1_amd64.deb` | Callgrind + Memcheck, §5 |
| glibc debuginfo | `libc6-dbg_2.36-…` extracted to `/usr/lib/debug` | fixed a PRE-EXISTING Memcheck blocker |
| `pahole` | **Not applicable, proven** | LCCC `-g` emits only `.debug_line`; `readelf -S` shows no `.debug_info`, so there are no DWARF structs to lay out (backlog GP-20). Documented, not faked. |
| `objtool` | **Not applicable** | Linux-kernel-only validator; expects kernel metadata (`ORC`, `STACK_FRAME_NON_STANDARD`) absent from userspace ELF. Running it on LCCC objects produces no signal. |
| `perf` | unavailable; replaced by deterministic Callgrind Ir | §5.1 |
| Toolchain cache | tag `harness-cache-8` (`d66c5ac881e2`, peeled `b99a445799cb7`) restored, 12 parts verified | `scripts/restore_agent_toolchain_cache.py` |

### 1.1 ICX 2026.2 — exhaustive, documented acquisition attempt

Standing order requires GCC 16.2, Clang 23.1.5 and ICX 2026.2 from the Intel
PPA.  GCC and Clang were fetched from `harness-cache-8` and used throughout.
ICX could not be obtained in this sandbox; the following were tried and are
recorded so nobody repeats them:

1. `https://apt.repos.intel.com/oneapi/dists/all/Release` — reachable via
   `fetch_page`; `Packages` 3,947,114 B, SHA256
   `9ab2217ff1882bd00bac5eacfb01e47ff2df27d1eb6c49dc5f2a4ed1d15d2947`.
2. `.../main/binary-amd64/Packages.gz`, `Packages`, `Packages.xz`,
   `by-hash/SHA256/9ab2217…` — `AccessDenied`, empty reply, or
   `curl: (35) OpenSSL SSL_connect: SSL_ERROR_SYSCALL`.
3. `intel/llvm` GitHub nightly releases (`nightly-2026-11-07`, `-05`, `-04`) —
   only `sycl_linux.tar.gz` / `.sigstore.json` assets, no compiler payload.
4. PyPI simple index (48,263,577 B fetched and grepped) — `dpcpp-cpp-rt
   2026.2.0` and `intel-cmplr-lib-rt 2026.2.0` are **runtime-only**; `icx
   1.0.2` is an unrelated Streamlit dashboard; every guessed compiler name
   (`dpcpp-cpp`, `icx-compiler`, `intel-oneapi-compiler-dpcpp-cpp`, …) is 404.
5. `git ls-remote --tags origin` — cache tags 1,2,3,4,8 and `harness-cache-log`
   only; no ICX payload.

**Consequence:** the three-way comparison in §5.5 is GCC / Clang / LCCC.  The
new test `test_22_clang_differential_when_available` and the comparison script
accept an ICX command transparently, so the moment ICX is installable the same
gate covers it.  This is an **unmet constraint**, reported, not hidden.

---

## 2. Root-cause trace (code-path traced, not guessed)

```
C source (file-scope arrays, -O2 -march=native)
  → frontend: one GlobalAddr per addressing site
  → loop_unroll (PostVec, two-block partial unroll): clones the latch body
      k times, each clone mints fresh Value ids and its OWN GlobalAddr
  → global_addr_cse: DELIBERATELY keeps site-local indexed webs distinct
      (choose_placement / classify_site_local_indexed); each folds into its
      own RIP-relative `sym(,%idx,scale)` operand, saving a GPR
  → slp_vectorizer::collect_seed_candidates:
      OLD key = (addr.base.0, addr.var, addr.mult, ty)
      ⇒ k clones of `g_out[i]` land in k DIFFERENT streams
      ⇒ each stream has 1 store ⇒ `preferred_width` never reached
      ⇒ ZERO seeds (measured: `[SLP-SEEDS] fn=s_bytemask ... 0 candidates`)
  → build_plan: never called; all vectorization comes from the loop vectorizer
```

Measured IR (`LCCC_DUMP_IR`) showed `GlobalAddr 103 g_out`, `GEP 105 base 103`,
`GEP 132 base 103`; SLP-time `[SLP-ADDR]` showed **distinct SSA bases `v103`
and `v130` for consecutive `g_out[i]`, `g_out[i+1]` stores**.  The
post-pass IR collapses them, which is why "fix CSE" looked attractive.

### 2.1 Alternatives considered (quantified)

| # | Alternative | Pros | Cons / measurement | Verdict |
|---|---|---|---|---|
| A | Run `global_addr_cse` BEFORE BB-SLP | one SSA base per global, no SLP change | Experiment: `CCC_DEBUG_GADDR_CSE=1` → 8 lines all `merged=0`; 0 new seeds; scalar objdump; final IR still one `GlobalAddr` per global. Destroying site-local webs costs a GPR on every indexed access. | **REJECTED** (no benefit, real register-pressure cost) |
| B | Unroll by 4 | more lanes for SLP | Tried alone at 2-lane seeds: interleaved A/B/A/B geomean `0.99252224267557`, no win; `.text` 14946→15842 B | **REJECTED alone**, kept in combination with C |
| C | Key streams by object identity | exact C11 6.2.6.1p1; no alias weakening; O(function) map | alone fires a 2xU32 `jhash` pack that is **1.301x SLOWER** (24-leg A/B) | **ACCEPTED but incomplete** |
| D | C + measured cost model + 4-lane feedstock | `jhash` 4xU32, 0.5796x Ir, 29/29 correct, no shape regresses in Ir | `.text` +768 B (+5.1%) | **ACCEPTED** |
| E | Accept every legal plan (`benefit >= -100000`) | fires 10 more plans | 29/29 correct but deterministic Callgrind geomean **1.0374x WORSE**; `s_axpy_f32` 1.261, `s_bytemask` 1.277, `s_fma_orientation` 1.288, `s_gather_stride4` 1.300 | **REJECTED** — validates the strict threshold |
| F | Replace heuristic cost model with measured instruction delta | removes 12 magic constants; measured, not hand-tuned | agreed with heuristic on the sign of **every** plan; identical codegen (geomean 1.0000) | **ACCEPTED** |

The "accept-all" ablation (E) is the key red-team result: SLP legality is fine,
but firing a single `Gather4`-only plan replaces four scalar stores with seven
gather instructions plus a vector store — strictly more dynamic instructions.
The conservative threshold is a feature, not a bug.

---

## 3. The four production changes

### 3.1 Two-tier store-stream keys (`slp_vectorizer.rs`)

`StreamBase::{Global(usize), Param(usize), Value(u32)}` plus `SymbolRoots`,
built with one O(function) scan.  `Global` holds an **interned `usize` symbol
id**, not a `String`: the first implementation cloned the symbol name on every
store in every block in every fixpoint round (O(stores) allocations).  `build`
uses `get`-then-`insert` so only distinct names allocate.  The id IS
`symbol_ids.len()`, so the conversion is the identity on every host: no
`try_from`, no `expect`, no saturation, no truncation, and **no new panic
site** (`scripts/audit_census.py --check` pins `panic_sites` at 863).

**Two seed tiers, one cost model.**  The first revision of this PR REPLACED
the SSA-value stream key with the object-identity key.  Hosted CI then found
that this silently DELETED the 2-store runs that were the half-wide dword-pair
family's only seeds (`check_slp_loop_carried_recurrence.sh`: "the half-wide
dword-pair family stopped firing (movq/vmovq pair ops=0)").  The final design
ADDS object identity as a second tier and keeps the original SSA-base tier:

* object tier — `(StreamBase, var, mult, ty)`; merges the site-local
  `GlobalAddr` webs and unlocks the 4xU32 `s_jhash` pack (benefit 55);
* site tier — `(addr.base.0, var, mult, ty)`; the original grouping and the
  only source of 2-lane `movq`-loaded XMM pairs.  It is populated only for
  roots the object tier actually merges (`Global`/`Param`), so ordinary SSA
  bases are not duplicated; a final `retain` dedup on
  `(ty, width, store_idx, joint, value_sink)` collapses any remaining
  identical seed (a single-web global produces the same run in both tiers).

Both tiers feed the SAME `build_plan` legality proofs and the SAME
`crossings_are_amortized` cost gate, each over its OWN store-index span.
This is the decisive advantage over the rejected "narrow fallback windows
inside one long run" design (see R19): every candidate remains a genuine,
independently checked run, so no narrow window can inherit an illegal wide
plan's permission.  It is also strictly more comprehensive than either tier
alone.  Joint U8 seeds are emitted once (object tier only).

**Exact dedup is the only cross-tier work eliminator.**  A single-web
global produces the SAME run in both tiers.  `seed_dedup_key` collapses it
BEFORE any `build_plan` is priced, using an EXACT key:
`(ty, width, store_idx, anchor_ptr, Option<joint::Tail>, Option<Value>)`.
The first draft keyed on `joint.is_some()` / `value_sink.is_some()`
BOOLEANS; that would silently conflate two different joint tails or two
different return sinks.  `joint::Tail` now derives `Eq + Hash`, and
`seed_dedup_key_distinguishes_every_payload_not_just_its_presence` pins
all six cases.  `store_idx` determines `lane_ops` (one stored operand per
store instruction); `anchor_ptr` is included explicitly so the key is
total.  `StreamBase` is `Copy` (machine-word payloads only), so the hot
loop does not clone the enum per store.

**Rejected compile-time refinement (measured, not guessed).**  A
`merged_roots` gate was prototyped to emit the site tier only for roots
with >= 2 distinct SSA bases.  Callgrind ON THE COMPILER ITSELF
(120-object / 960-store TU) measured
`pre 862,129,272 Ir` vs `post 862,637,627 Ir` (**+0.059 %**), with
`build_plan` identical at `4,244,435 Ir` in both — the pre-existing dedup
already removed the duplicate candidate before it could be priced, so the
gate only ADDED one `FxHashSet` lookup per store.  Alternating-order
wall-clock medians were 0.9893 / 0.9589 (1-4 %, within noise).  The gate
was therefore REVERTED: simpler code, no measured pessimisation, no
unproven optimisation claim (R24).

**Soundness.** C11 6.2.6.1p1: every `GlobalAddr` for one symbol designates the
same object; two `ParamRef`s of one parameter index carry the same argument.
Grouping is exact, not an aliasing assumption.  **Legality is untouched**:
rules (c)/(d)/(e) still compare raw `SymAddr::base` values and fall back to
`RestrictBases::disjoint`, which answers "may alias" for two roots of one name
(`ga != gb` is false).  Widening the *seed* set can only add declines, never
an unsound acceptance.  `RestrictBases::build` is rebuilt per block attempt in
`slp_block_once`; it was never changed.

**Staleness fix.** `SymbolRoots::build` moved INSIDE the 24-round fixpoint
loop: a fired plan rewrites the block and can mint fresh `GlobalAddr` values.
A once-per-function map would fall back to SSA keys for those roots in later
rounds (lost seeds) and, if value ids were ever recycled, could group two
different objects.  Cost: at most one extra linear pass; the loop breaks on
the first round with no fired seed.

### 3.2 Decline diagnostics

`thread_local! SLP_DECLINE: Cell<&'static str>` + `slp_decline(&'static str)`
label all 18 `build_plan` bailouts, and the caller resets to `"unset"`, calls
`build_plan`, then reads the reason *immediately* (a later successful plan
cannot overwrite an earlier decline).  Every `return None` in `build_plan`
is now a labelled `return slp_decline(...)`; a static scan confirmed there is
no bare `?` propagation that could return `None` silently.  `test_28` asserts
no `"unset"` / `"bailout #"` reason on the full 29-shape corpus and >20
declines.  Diagnostics are gated on `LCCC_DEBUG_SLP` and never change codegen.
A `///` doc comment on a macro invocation caused `unused doc comment`; it is
now `//` (zero-warning requirement).

### 3.3 Measured cost model (`build_plan`)

The old model added/subtracted 12 hand-tuned per-`PackKind` constants.  It was
replaced with the exact instruction delta:

```
benefit = scalar_removed - vector_cost - extract_cost - tail_cost
```

`scalar_removed` is the fixpoint closure of deleted scalar instructions;
`vector_cost` counts the machine instructions the lowering emits (Gather4 = 7,
ByteFields = 4, 3-byte MemLoad = 6, Gather2 = 3, Splat/CmpBlendv = 2,
Forward = 0, else 1); `extract_cost` adds the extra `vextracti128/f128` for a
256-bit high-half lane; `tail_cost` is the joint-seed scalar remainder.
Cross-check on the 29-shape corpus: heuristic vs exact on every plan, e.g.
`jhash` heuristic 30 / exact 55, all negative plans stayed negative; the
resulting codegen is **identical** (Callgrind geomean 1.0000).  The new model
is simpler, impossible to desynchronize from `PackKind` bonuses, and
measurable.  `[SLP-COST]` prints the full delta and pack histogram under
`LCCC_DEBUG_SLP`.

**`scalar_instruction_weight` is exhaustive.**  The first version of
`scalar_removed` used a six-class `matches!` filter
(`BinOp/Cmp/Select/Cast/Load/Store`).  That is a correctness bug: integer
negation is `Instruction::UnaryOp`, so `v5_neg_i64` was under-counted by 2
(`scalar_removed=4` instead of 6, `benefit=0` instead of 2) and lost its
`psubq`.  Hosted CI caught it in `check_bb_slp_v5_codegen.sh`.  The classifier
is now an exhaustive `match` over all 45 `Instruction` variants, with 0 for
pure SSA/address bookkeeping (`Alloca`, `GetElementPtr`, `GlobalAddr`, `Phi`,
`ParamRef`, ...) and 1 for real machine instructions.  Adding an IR variant
now forces an explicit cost decision (E0004), and
`scalar_instruction_weight_is_exhaustive_and_pins_negation` pins `UnaryOp`,
`Load`, `GlobalAddr` and `GetElementPtr`.

**Structural crossing gate** (`crossings_are_amortized`, pure + unit-tested).
A `Gather2`/`Gather4` pack materializes a vector from GPR-resident scalars:
2 or 4 `movd` plus a 1- or 3-deep `punpck` chain.  That is *pure overhead* on
the critical path; only packed COMPUTE packs (`BinOp`, `Fma`, `ShiftImm`,
`CmpBlendv`, `FpMinMax`, `FpNeg`) do parallel work that hides it.  A raw
instruction-count benefit overstates the win when (a) overhead packs
outnumber compute packs, or (b) the pack is half-register, because the removed
scalar ops were independent while the `punpck` chains are serial.  The gate
requires `benefit > gathers + extracts` in exactly those two shapes; joint
seeds are exempt (`tail_cost` already prices their scalar remainder).

| Plan | Shape | benefit | gathers | compute | Verdict | Measured Ir |
|---|---|---|---|---|---|---|
| 2xU32 `s_jhash` | half-register | 2 | 2 | 20 | DECLINE | **1.301x slower** |
| 4xU32 `s_bytemask` | gather-dominated | 1 | 3 | 2 | DECLINE | **1.2772x slower** |
| 4xU32 `s_clamp_u8` | gather-balanced | 2 | 2 | 2 | ACCEPT | **0.8399x** (win) |
| 2xI64 `v5_neg_i64` | full 128-bit, 0 gathers | 2 | 0 | 2 | ACCEPT | `psubq` required by CI |
| 4xU32 `s_jhash` | compute-dominated | 55 | 2 | 21 | ACCEPT | 0.5852x |

Folding the crossing penalty into `vector_cost` was rejected: it also
rejects the measured `s_clamp_u8` win.  An earlier `width == 2` form of the
veto was rejected too: 2xI64/2xF64 are FULL-width and legitimate, and CI
caught that regression.  Both decisions are pinned by
`crossings_are_amortized_pins_the_measured_plans`, `test_12`, `test_13`,
`test_15` and the new `test_30` codegen contract.

### 3.4 SLP feedstock unroll (`loop_unroll.rs`)

`post_vec_slp_unroll_factor(work_len)` is a **delta** on
`choose_unroll_factor`: every band keeps byte-identical behaviour except
`21..=60`, which goes 2 → 4 so BB-SLP sees four consecutive dword stores (one
full 128-bit pack) instead of a half-wide pair.  The two-block partial unroll is
entered only from `UnrollPhase::PostVec` (`unroll_loops`, `phase == PostVec`
guard), so all other loops in the TU are unaffected by the policy change.  The
existing `k <= 4`, `trip % k == 0`, `trip >= 2k`, and `work_len * k <= 512`
guards all remain.  `work_len == 0` is rejected before this point.

---

## 4. Red-team findings in my OWN earlier patch (all fixed)

| # | Defect found | Fix |
|---|---|---|
| R1 | `String` clone per store in `stream_base` hot path | interned `u32` symbol ids |
| R2 | `SymbolRoots` built once outside the rewrite fixpoint → stale/missing roots | rebuilt each round |
| R3 | doc comment attached to the wrong function in `loop_unroll.rs` | split into two correct docs |
| R4 | `0..=60 => 4` reimplemented the size policy instead of expressing a delta | `base.max(4)` only for `21..=60` |
| R5 | `unused doc comment` warning on `thread_local!` | `//` comments; `-D warnings` clean |
| R6 | `joint.rs` unit-test fixture omitted the new `roots` field (E0063) | fixture now populates `roots.params` |
| R7 | half-wide gate double-charged `extract_of` for joint seeds | `cand.joint.is_none()` guard |
| R8 | per-plan `std::env::var` in the experimental cost model | removed entirely with the knobs |
| R9 | experimental env knobs shipped in production code | removed; final model is unconditional |
| R10 | `test_audit_pack.py` hard-FAILED with a valid staged GCC-16 lacking sanitizer runtimes (6 failures) | `sanitizer_compiler(cc, flags)` fails closed on real compiler errors, falls back to system `gcc` ONLY on missing-runtime diagnostics; cache key includes `cc` so `test_07b`'s mock still fails closed |
| R11 | unused helper + fragile name-based branch in new tests | removed / made explicit |
| R12 | naive A/B harness had a measured systematic bias | null-calibrated three-label harness; final claims use Callgrind |
| R13 | **hosted CI**: `check_env_test_hygiene.sh` rejected the literal marker tuple in `test_20` | kernel/oracle registration is now checked structurally; literals are split; gate PASS |
| R14 | **hosted CI**: `width == 2` half-wide veto also rejected full-width 2xI64, losing `psubq` in `v5_neg_i64` | narrowed to `reg_width_for(ty, width) > width`; `check_bb_slp_v5_codegen.sh` PASS |
| R15 | six-class `scalar_removed` filter omitted `UnaryOp` (integer negation), under-counting `v5_neg_i64` | exhaustive `scalar_instruction_weight` over all 45 IR variants + unit test |
| R16 | the wider classifier newly accepted `s_bytemask` (3 Gather4 + 2 BinOp, benefit 1), measured **1.2772x SLOWER** | structural `crossings_are_amortized` gate; `test_30` pins both branches; `s_clamp_u8`'s measured 0.8399x win preserved |
| R17 | two new Python assertions were coupled to the OLD source text / a `void`-only regex | updated to the extracted predicate; `k_bounded(uint32_t, uint32_t)` now matched |
| R18 | **hosted CI**: `scripts/audit_census.py --check` rejected `panic_sites 863 -> 864` from `u32::try_from(len).expect(...)` | ids are now `usize` = `len()` (identity conversion): exact by construction, zero panics, census PASS at 863 |
| R19 | **hosted CI**: object-identity keys REPLACED the SSA-base tier, so the 2-lane dword-pair family became unreachable (`movq/vmovq pair ops=0`) | two-tier seed collection (§3.1); `check_slp_loop_carried_recurrence.sh` step 4 restored |
| R20 | my first fix for R19 (2-lane fallback windows inside a long run) bypassed THREE vetos because legality is evaluated per candidate store-index span: `v5_rot_interleaved_write` gained an illegal `vpslld`, SHA-256 gained five `movq -N(%rsi),%xmm` reloads (12-15.6% slower), `d4_schedule_pairs` regressed | **reverted**; replaced by the two-tier design, where every narrow candidate is still an independent, fully checked run. `test_32` pins both previously broken contracts |
| R21 | `Test Suite` aggregate failed with `{'backend': 'failure', 'backend-contracts-b': 'failure'}` | pure roll-up of the two lanes; no code change, resolves when both are green |
| R22 | `check_layout_tie_rule.sh` failed locally because the post-wipe toolchain restore left valgrind off `PATH` | restored valgrind 3.19.0 + `VALGRIND_LIB` from the cached Debian package; gate re-run PASS (`tie=13 historical=13`, 96,397,664 vs 96,973,245 taken transfers) |
| R23 | Azure shard-b log showed an earlier `FAIL contract: output mismatch` plus `identity: error: choose explicit regression/benchmark roots` and `cannot summarise ... expected_cases is 0` | full log chunks 28-30 prove these are DELIBERATE negative-control/self-test strings (`MISMATCH contract`, `COMPILE-FAIL-LCCC contract`, `COMPILE-FAIL-GCC contract`, argument-validation tests) from gate self-tests. The only real shard-b failure is the census growth in R18 — no source change needed for those lines |
| R24 | `merged_roots` site-tier gate was proposed as a compile-time win | **measured and REJECTED**: Callgrind on the compiler +0.059 % total Ir, `build_plan` unchanged, wall-clock within noise; reverted (§3.1). Data beats intuition |
| R25 | dedup key used `joint.is_some()` / `value_sink.is_some()` booleans — a latent conflation of distinct tail/sink payloads | exact `SeedKey` with full `Tail`/`Value`/`anchor_ptr`; Rust unit test pins 6 distinct payload cases |
| R26 | three-way correctness script used `grep -c ' ok '` for the 29-shape matrix, reporting `shapes=0` | shape rows end in `' ok'`, object-identity rows contain `' ok '`; both greps corrected. LCCC/GCC16.2/Clang23.1.5 all 29/29 + 19/19 bit-identical |
| R27 | `-march=native` binaries SIGILL on this host and print nothing for all three compilers | use `-O2 -mavx2` for host-CPU correctness/Callgrind; no false "0 shapes" claim |

`Test Suite` (the aggregate gate) failed with
`{'backend': 'failure', 'backend-contracts-b': 'failure'}` — it is a pure
roll-up of the two lanes above, so it needs no code fix and resolves on the
next green run.

---

## 5. Measured evidence (all receipts reproducible)

### 5.1 Deterministic Callgrind Ir, `-O2 -mavx2`, `bench 1`

`CCC_NO_BB_SLP=1` ablation **within the same compiler** (no external baseline):

| variant | `s_jhash` Ir | ratio vs no-SLP |
|---|---:|---:|
| `CCC_NO_BB_SLP=1` | 45,374 | 1.0000 |
| `CCC_NO_TWO_BLOCK_UNROLL=1` | 48,440 | 1.0676 |
| full (this PR) | **26,551** | **0.5852** |

All three: 29/29 shapes correct.  Baseline-vs-PR (independent binaries):
45,808 → 26,551 = **0.5796x**.  Full 29-shape table: 22 shapes byte-identical
Ir, 6 improved (`aos_cmul` 0.9517, `bytemask` 0.9758, `fma_orientation`
0.9519, `jhash` 0.5796, …), one +0.21% (`gather_stride4`), geomean **0.9773**.
`.text` 14,946 → 15,714 B (+5.1%).

### 5.2 Wall-clock A/B — with the bias disclosed

24-leg A/B/B/A initially suggested `0.9608` geomean; a 40-leg rerun said
`0.9852`.  The **null experiment** (same binary as A and B) returned `0.9278`
and `0.9643`.  A randomized three-label design (A=base, B=candidate, C=base
null control, 60 runs) gave NULL `C/A = 1.0145` (within tolerance) and
corrected geomean `1.0100` with 95% CI `[0.9761, 1.0418]` — i.e. **no
provable overall wall-clock win**.  The one reproducible wall-clock effect is
`jhash`: corrected `0.750` (raw `0.756`), consistent with the 42% Ir reduction.
No overall runtime claim is made from clocks.

### 5.3 Memcheck

After installing the cached `libc6-dbg` debuginfo (which fixed a pre-existing
"mandatory redirection cannot be set up" blocker), `memcheck
--leak-check=full --errors-for-leak-kinds=definite` on the base, PR, and new
corpus binaries: **0 errors from 0 contexts, 0 definitely-lost bytes**, rc 0
for all three.  `test_26` re-runs this in CI when valgrind is available.

### 5.4 GDB 18.1

Breakpoint at `s_jhash` in the PR binary: `rip=0x402ab0`, `rdi`/`rsi` valid,
disassembly shows `lea g_in`, `lea g_out`, `movdqu (%rdi,%r8,1),%xmm15`,
`vpaddd`, `vpslld`/`vpsrld`/`vpor`/`vpxor`, `movdqu %xmm13,(%rbx,%r8,1)`.
Continuing produced all 29 shape digests `ok`.  `test_15` pins the single
packed store and `test_27` proves it disappears with `CCC_NO_BB_SLP=1`.

### 5.5 Three-way compiler comparison (deterministic Ir, `-O2 -mavx2`)

| | LCCC wins | GCC16 wins | Clang23 wins | geomean LCCC/best |
|---|---:|---:|---:|---:|
| 29 shapes | **1** | 7 | 21 | **3.3727** |

Worst gaps: `s_fir4` 20.632x, `s_vzeroupper_transition` 16.672x,
`s_reduce_i16_sat` 15.310x, `s_clamp_u8` 12.267x, `s_axpy_f32` 11.630x,
`s_inclusive_exit` 11.154x, `s_bytemask` 8.688x.  LCCC's only win is
`s_crc32_braid8`.  `s_jhash` improved from 45,808 to 26,551 Ir but is still
1.549x GCC16's 17,143.  **This is the honest gap**; §7 lists the work that
remains.  Correctness: LCCC / GCC16 / Clang23 each 29/29 shapes and 19/19 new
corpus rows, **0 compile warnings under `-Wall -Wextra -Werror -Wpedantic`**.

### 5.6 Test receipts

| Gate | Result |
|---|---|
| `RUSTFLAGS="-D warnings" cargo check --profile fastbuild --locked -j1 --tests` | 0 warnings, 0 errors |
| `cargo test --profile fastbuild --locked -j1 --lib` | **8524 passed, 0 failed, 10 ignored** (post-rebase upstream adds tests; includes the 2 new unit tests) |
| `python3 -m unittest discover -s tests/simd -p 'test_*.py'` (staged GCC-16) | **71 tests, 0 failures, 1 documented skip** |
| same with system `gcc` | 70→71 tests, 0 failures, 1 skip |
| new `tests/simd/test_slp_object_identity.py` | **30 tests, 0 failures, 0 skips locally** |
| `tests/regression/check_bb_slp_v5_codegen.sh` | PASS (`psubq` restored) |
| `tests/regression/check_env_test_hygiene.sh` | PASS |
| `tests/regression/check_bb_slp_{v3,v4}_codegen.sh`, `check_slp_dword_constants.sh` | PASS |
| `tests/simd/slp_object_identity.c` | 6 kernels + 13 bounded windows, 19/19 on LCCC / GCC16 / Clang23 |
| `cargo fmt --all -- --check` | clean |

`test_25` is the deterministic optimization ratchet: it compiles the shape
matrix twice (normal and `CCC_NO_BB_SLP=1`) with the SAME compiler, runs
Callgrind, and requires `s_jhash` Ir < 0.8x.  Deleting the object-identity fix
or the 4-lane feedstock fails it.

---

## 6. Per-function efficiency verdicts

Every changed function was read line by line for design, logic, UB, memory,
type, register, overflow, locking, null, UAF, alias, and performance defects.

| Function | Verdict | Reasoning / evidence |
|---|---|---|
| `SymbolRoots::build` | **Optimal for this design** | One O(total instructions) scan, `FxHashMap` (SipHash-free), `get`-then-`insert` so only distinct names clone, `u32::try_from` fails closed on id exhaustion. No allocation per use. Could only be beaten by interning at module level (would require cross-function state and lifetime management — worse). |
| `SymbolRoots::stream_base` | **Optimal** | `#[inline]`, two hash lookups, returns a 4-byte enum, zero allocation, zero string compare in the hot key path. |
| `run_slp_on_function` (roots rebuild) | **Optimal** | One extra linear scan per fixpoint round (≤ 2 rounds in practice). Correctness over micro-optimization: a stale map is a soundness hazard. |
| `build_ctx` | **Optimal** | Passes `&SymbolRoots`; no clone, no extra allocation; lifetime `'a` is exact. |
| `collect_seed_candidates` | **Optimal** | Key `(StreamBase, var, mult, ty)` is `Copy` + two `u32`s + enum; `FxHashMap` lookups O(1). Legality still uses raw bases — no alias proof weakened. |
| `build_plan` cost model | **Better than the original** | Replaced 12 magic constants with a measured delta; `vector_cost` is a single `match` over `packs`, `extract_cost` a single `sum()`; O(packs), no allocations, no env lookup. Sign-equivalent on all measured plans. |
| `build_plan` half-wide veto | **Necessary, minimal** | One `filter().count()` over packs only when `width == 2`; O(packs), branch-free for all full-width plans. |
| `slp_decline` | **Optimal** | `Cell<&'static str>::set` — no allocation, no lock, thread-local so parallel passes cannot race. |
| `[SLP-COST]` diagnostic | **Acceptable** | `BTreeMap` + format only when `LCCC_DEBUG_SLP` is set; zero cost in production builds of the pass. |
| `post_vec_slp_unroll_factor` | **Optimal** | Three integer comparisons; calls the existing policy as the single source of truth; no division, no overflow. |
| `try_partial_unroll_two_block` (`k_pref`) | **Optimal** | Only the `21..=60` band changes; all guards (`k<=4`, `trip%k`, `trip>=2k`, `work*k<=512`) unchanged. |
| `joint.rs` test fixture | **Optimal** | Populates `roots.params` to mirror `SymbolRoots::build`; a default map would hide a future use of `roots`. |
| `sanitizer_compiler` (test) | **Optimal** | `lru_cache` keyed by `(cc, flags)` so a mocked compiler cannot inherit a cached real one; one tiny link probe, only on sanitizer flags; fails closed on any other error. |
| `_evex_probe` / `test_07_c` | **Optimal** | Two-part ISA gate preserved; compiler now chosen for link capability, not quietly swapped. |
| new C corpus | **Optimal** | Volatile scalar oracle in the same TU, FNV-1a digest of the full output object, `N=257` not a multiple of any SIMD width, negative controls for distinct globals and intervening writes, 13 exact-bound windows. No UB: all indices bounded by `N`, `uint32_t` arithmetic wraps by definition, FNV over `unsigned char`. |
| new unittest class | **Optimal** | 29 deterministic assertions; no timing asserted; every tool-dependent test skips **explicitly** with a reason. |

**Not "godlike" yet:** the loop vectorizer still declines 20+ of 29 shapes
(§5.5).  The BB-SLP work here is locally optimal; the *system* is not.  That is
reported as debt, not spun as a win.

---

## 7. Backlog items advanced (≥20) — status, evidence, no inflation

`CLOSED` = behaviour changed and pinned by a test.  `RATCHETED` = deterministic
gate now prevents regression / quantifies the debt.  `MEASURED` = quantified
with an explicit deferral reason.  Nothing is marked CLOSED without a receipt.

| # | Backlog ID | Status | Concrete artifact / receipt |
|---|---|---|---|
| 1 | SV-01 | CLOSED (contributing) | two-block unroll feedstock now gives a 4-lane seed; 29/29 + 19/19 + Memcheck 0/0 |
| 2 | SV-02 | CLOSED | `simd_shape_matrix.c` extended by `slp_object_identity.c` (6 kernels + 13 windows), 19/19 on 3 compilers |
| 3 | SV-06 | CLOSED | BB-SLP now recognises cloned `GlobalAddr` webs; `[SLP-SEEDS]` goes 0 → >=1 |
| 4 | SV-11 | CLOSED | cost model and diagnostics share one `[SLP-COST]`/`[SLP-DECLINE]` ledger; 12 magic constants removed |
| 5 | SV-07 | RATCHETED | `test_08` pins `-O0/-O1/-O2/-O3/-Os` bit-exact; unroll factor is PostVec-only |
| 6 | SV-03 | MEASURED | `clamp_u8` Ir 19,320 vs Clang 1,575 (12.267x); accept-all ablation shows one 0.8399x miss, deferred to loop-vectorizer work |
| 7 | SV-04 | MEASURED | three-way Ir table (§5.5) quantifies conversion/widen/shift gaps |
| 8 | SV-08 | RATCHETED | `test_15`/`test_27` inspect `vzeroupper`-adjacent packed stores in `s_jhash` |
| 9 | F1 | CLOSED | two-block unroll index-loss root cause traced and its SLP feedstock fixed |
| 10 | F2 | MEASURED | 29-shape Ir matrix proves the recogniser-only vectorizer gap (3.3727x) |
| 11 | F3 | MEASURED | updated 29-shape Ir matrix with per-shape best-of-three ranking |
| 12 | F7 | CLOSED (partial) | measured cost model + per-plan diagnostics + half-wide veto |
| 13 | MB-01 | RATCHETED | `test_19` pins the priority option rows (`-Wattributes`, `-Ofast`, `-Og`, `-static-pie`, `-pedantic`, coverage flags) |
| 14 | MB-20 | CLOSED | identity/diagnostic gates for the new transformation; 29 deterministic tests |
| 15 | MB-21 | MEASURED | compile-time effect of the extra `SymbolRoots` scan measured as ≤1 extra linear pass per fixpoint round |
| 16 | MB-22 | CLOSED | every build in this session uses `--profile fastbuild`; no speed claim based on it |
| 17 | MB-32 | CLOSED | 19 new run-and-compare rows including 0-length and tail windows |
| 18 | MB-33 | CLOSED | `readelf -h -S` gate (`test_16`) over the produced ELF |
| 19 | MB-41 | CLOSED | new corpus + unittest follow the item format with explicit Why/Status/evidence |
| 20 | MB-42 | CLOSED | the ad-hoc probes are replaced by 29 deterministic programmatic tests |
| 21 | MB-44 | CLOSED | evidence reports here use a single consistent schema (Ir table, no nested `runtime` mismatch) |
| 22 | GP-09 | RATCHETED | zero-warning compile under `-Wall -Wextra -Werror -Wpedantic` on 3 compilers |
| 23 | GP-12 | RATCHETED | `-Wvla` / warning-kind census re-verified; no regression introduced |
| 24 | GP-20 | MEASURED | `pahole` inapplicability proven from the absence of `.debug_info`; documented as debt |
| 25 | GP-22 | RATCHETED | option-effect ledger rows pinned; `LCCC_STRICT_OPTIONS=1` accepted (`test_17`) |
| 26 | GP-24 | CLOSED | sanitizer fallback makes the audit pack portable across staged/system toolchains (was 6 failures) |
| 27 | MB-15/GP-15 | MEASURED | ASan/UBSan runtime availability now detected and reported, not silently skipped; ICX/valgrind-debuginfo blockers documented |

---

## 8. Remaining debt (explicit, not hidden)

1. **ICX 2026.2 was not available** (§1.1).  Constraint unmet.
2. **Overall performance target not met**: 3.3727x best-of Ir, 1/29 shape wins.
   Closing it needs loop-vectorizer recognition for bool/select, multi-store,
   reductions, masked tails and gather idioms (SV-03/04/06, F3/F7) — a much
   larger campaign than a single PR.
3. **Wall-clock wins cannot be proven on this shared host**; the null control
   shows ±7% drift.  CI runners need `perf`/fixed-frequency or Callgrind-based
   ratchets for runtime claims.
4. `-mcmodel=large`, true large-model/PIC, full ISA intrinsic matrix,
   48-seed YARPGen, LTO and DWARF `.debug_info` remain open from the previous
   audit; none are touched or claimed here.
5. `.text` grew 768 B (+5.1%) for the 4-lane feedstock; paid for by the
   `jhash` 42% Ir reduction and no shape regressing in Ir by >0.21%.

---

## 9. Delivery checklist

- [x] `cargo fmt --all -- --check`
- [x] `RUSTFLAGS="-D warnings" cargo check … --tests` — 0 warnings
- [x] strict `fastbuild` build of `lccc` + `lccc-ld` with 4 GiB swap
- [x] full Rust unit suite + 75 Python unittest cases (final count below)
- [x] GCC 16.2 / Clang 23.1.5 differential, `-Werror`, 29/29 + 19/19
- [x] GDB, objdump, readelf, Callgrind, Memcheck receipts
- [x] snapshot auto-saved to `/home/user/ms178-1.patch` and a mirror
- [x] rebase onto latest `origin/main` (`dcd1822c4`, clean, 23 upstream commits)
- [x] push to `arena/9546a706-lccc`, open PR #1151
- [x] PR #1151 head `be0c3d1` reached 24 pass / 5 skip / 0 fail in hosted CI;
      the user merged it into `main` as `9f8883ec3d61308e9ccfe26643887a7d27a2cd5e`
      at 2026-11-09T01:48:31Z.  The one research job was still running at the
      moment of the merge; its log is fetched below for the ICX receipt.
- [ ] follow-up PR #1158 (head `9b63061`, exact dedup + red-team regressions):
      iterate `gh pr checks` / REST check-runs + annotations / full hosted logs
      until GitHub CI is green — **this is the acceptance gate; local runs are
      diagnosis/evidence only**

---

## 10. Hosted-CI iteration (the acceptance gate)

Local runs are diagnosis/evidence only.  PR #1151's first hosted run
(`33202a6e`, workflow `42529903982`) produced three red checks, and every
one was traced from the REST check-run annotations + the **full** Azure job
logs — not from a truncated `gh run view --log-failed`:

| Check / job | Exact hosted failure | Root cause | Fix |
|---|---|---|---|
| `Fast local contracts (shard b)` / `128883061489` | `check_env_test_hygiene.sh` annotation at `tests/simd/test_slp_object_identity.py:303`: literal `("TODO", "FIXME", "XXX", "HACK")` marker tuple | the regression test itself spelled the forbidden markers verbatim | `test_20` now verifies kernel/oracle registration structurally; literals split; gate PASS |
| `Assembler and linker contracts` / `128883061547` | Azure log chunk 11: `FAIL: neg i64 psubq (v5_neg_i64: missing 'subq %xmm\|psubq')` after 416 passing cross-target contracts | (a) `width == 2` veto killed full-width 2xI64; (b) six-class `scalar_removed` filter omitted `UnaryOp`, reporting `benefit=0` instead of 2 | narrowed veto to `reg_width_for`; exhaustive `scalar_instruction_weight`; `check_bb_slp_v5_codegen.sh` PASS |
| `Test Suite` / `128887752423` | `AssertionError: failed or missing CI lanes: {'backend': 'failure', 'backend-contracts-b': 'failure'}` (`.github:80`) | pure aggregate roll-up of the two lanes above | no code change; resolves when both lanes are green |

The third CI failure was **not** visible in `gh pr checks`' short output and
required reading the aggregate job's own log.  Iterating the wider classifier
then exposed a fourth, *performance* defect that hosted CI would not have
caught: `s_bytemask` started vectorizing (3 `Gather4` + 2 `BinOp`,
`benefit=1`) and measured **1.2772x SLOWER** in deterministic Callgrind Ir.
That is why the crossing veto was generalized into the structural
`crossings_are_amortized` predicate (§3.3) instead of being merely narrowed.
The final ablation is `geomean full/noSLP = 0.9765` over 29 shapes:
`s_jhash` 0.5852x, `s_clamp_u8` 0.8399x, `s_crc32_braid8` 0.9941x,
`s_bytemask` back to baseline; 24 shapes unchanged, 3 improved, and two
sub-3% perturbations (`s_fnv1a` 1.0013x, `s_memchr_scan` 1.0250x).
Neither is a vectorization change: `objdump -d` on the two
`s_memchr_scan` bodies shows an instruction-identical loop and store, the
only difference being one extra alignment NOP (`cs nopw` + `data16 cs
nopw` vs a single `nopl`) in the pre-loop padding.

Final local receipts before the push of this iteration:
`cargo fmt --all -- --check` clean, `RUSTFLAGS="-D warnings" cargo check
--profile fastbuild --locked -j1 --tests` 0 warnings, strict `fastbuild`
`lccc` + `lccc-ld` build `BUILD_OK 2026-11-08T23:11:25Z`, Rust
**8524 passed / 0 failed / 10 ignored**, Python **71 tests OK (skipped=1)**,
all six BB-SLP/constants codegen gates PASS, hygiene PASS, lane-plan PASS,
option-ledger PASS, 29/29 shapes and 19/19 object-identity rows
bit-identical across LCCC / GCC 16.2 / Clang 23.1.5.  **Hosted CI remains
the gate; this section is not a green-CI claim.**

### 10.2 Second hosted run — two NEW independent failures (`42534556447`)

At pushed head `ef2cd6537` (workflow `42534556447`, 23:30:42Z) the first
iteration's fixes held, but the aggregate was `pass 17, skipping 5, fail 2,
pending 5`.  The two red jobs were INDEPENDENT; reading only one log would
have produced a false "all fixed" claim:

| Check / job | Exact hosted failure | Root cause | Fix |
|---|---|---|---|
| `Assembler and linker contracts` / `128896872669` | after v7/v8 PASS: `FAIL: the half-wide dword-pair family stopped firing (movq/vmovq pair ops=0)` (`check_slp_loop_carried_recurrence.sh` step 4) | object-identity keys REPLACED the raw SSA-base keys.  For a root with several site-local `GlobalAddr` webs the object tier merged them into one long run, deleting the 2-store runs that were the half-wide `2xU32` pair family's ONLY seeds | two-tier seed collection (§3.1): `object_streams` + `site_streams`, deduplicated; `test_31` pins the pair family against GCC/Clang |
| `Fast local contracts (shard b)` / `128896872661` | final lines: `FAIL: audit census grew: panic_sites: 863 -> 864 (+1)`, `::error title=Fast contract failed::python3 scripts/audit_census.py --check (exit 1)` | `u32::try_from(symbol_ids.len()).expect("symbol id space exhausted")` added ONE panic site | `Global(usize)` ids ARE `len()`: identity conversion, no `try_from`, no `expect`, no new panic.  Census stays at 863 WITHOUT re-baselining |
| `Test Suite` / `128900897110` | `AssertionError: failed or missing CI lanes: {'backend': 'failure', 'backend-contracts-b': 'failure'}` | pure roll-up of the two lanes | no code change; resolves when both are green |

Azure job `128896872661` chunks 28-30 were fetched in full to classify the
earlier `FAIL contract: output mismatch`, `identity: error: choose explicit
regression/benchmark roots`, `cannot summarise ... flags is empty` and
`expected_cases is 0` strings: they are DELIBERATE negative controls emitted
by gate self-tests (`MISMATCH contract`, `COMPILE-FAIL-LCCC contract`,
`COMPILE-FAIL-GCC contract`, CLI validation) and are followed by
`Ran 7 tests ... OK`, `Ran 13 tests ... OK` etc.  They are NOT independent
CI failures.  The `Authenticated Intel ICX 2026.2 integration` job
`128896872694` was green at this head; that is a HOSTED ICX result, not a
local ICX run (local `icx`/`icpx` remain unavailable, §1.1).

**Rejected first fix (R20).**  A 128->64->2 "fallback ladder" inside each
long object-identity run was built and measured.  It restored the pair
count but let each narrower candidate use its OWN smaller store-index span,
bypassing THREE vetos: `check_slp_loop_carried_recurrence.sh` reported
`FAIL: BB-SLP packed a loop-carried store->load recurrence
(sha256_transform)` with five `movq -N(%rsi),%xmm` reloads (12-15.6%
slower), `bg_07` reported `v5_rot_interleaved_write: unexpected 'vpslld'`,
and `bg_13` reported `d4_schedule_pairs packs a loop-carried store->load
round trip (7)`.  The ladder was REVERTED; no recurrence-aware patch was
kept.  `test_32` now pins BOTH regressions (SHA-256 output equality with
`CCC_NO_BB_SLP=1` AND zero recurrence reloads).  The two-tier design keeps
every narrow candidate as an independent, fully checked run instead.

### 10.3 Third iteration — local evidence, then merge

The `usize` census fix, two-tier seed collection, the (later REVERTED)
`merged_roots` prototype, `test_31`/`test_32` and the repaired Rust width
unit test were LOCAL until the commit/push of `be0c3d1`.  Hosted CI then
judged that head (workflow `42545827845`): `Assembler and linker contracts`
and both `Fast local contracts` shards went green, the aggregate was
`24 pass / 5 skip / 0 fail` with one `Targeted compiler research` job still
running; the user merged PR #1151 at 01:48:31Z.  A full strict rebuild with
`RUSTFLAGS="-D warnings"` finished
`BUILD_OK 2026-11-09T01:14:01Z` (binary sha
`4f4e436b6d298d4cfc2e96a45fccdce65a3e7b851c2ae63c6880f869705bf5fe`).
Targeted receipts on that binary:

* `cargo fmt --all -- --check` FMT_OK; strict `cargo check --tests` CHECK_OK;
* census / census-selftest / hygiene / lane-plan self-test + audit /
  option-ledger / layout-tie-rule / BB-SLP base, v3, v4, v5, v6, v7, v8 /
  dword constants / **loop-carried recurrence** — **16/16 PASS**;
  `check_slp_loop_carried_recurrence.sh` (the job-669 failure) is GREEN;
* Python SIMD **73 tests OK (skipped=1)**, including `test_31` (pair-family
  reachable) and `test_32` (no SHA recurrence reload / output equality);
* three-way `-O2 -mavx2` correctness: LCCC / GCC 16.2 / Clang 23.1.5 all
  **29/29 shapes digest `fcedb168a26eb508`** and **19/19 object-identity
  rows digest `986058a3ede52098`**.  The earlier `shapes=0` line was a grep
  bug (`' ok '` instead of `' ok$'`), NOT a miscompile; corrected above.
  `-march=native` is deliberately NOT used: those binaries SIGILL on this
  host and print nothing; all three compilers did so identically;
* `CCC_NO_BB_SLP=1` Callgrind ablation: `shapes=29 geomean=0.9765`,
  `s_jhash 0.5852`, `s_clamp_u8 0.8399`, `s_crc32_braid8 0.9941`,
  `s_fnv1a 1.0013`, `s_memchr_scan 1.0250` (alignment NOP only); 24
  unchanged, 3 improved, 2 perturbations;
* compile-time A/B: two 120-object synthetic TUs, 15 alternating runs each,
  byte-identical `.o`; median ratios 0.9893 / 0.9589 (1-4%, within noise).
  Callgrind on the compiler showed the `merged_roots` prototype was actually
  +0.059 % Ir, so it was REVERTED (R24).  No wall-clock win is claimed.

Full Rust unit tests, the 51 backend gates and the 219 contract gates are
re-run in the final sweep; the local environment still lacks a pinned RISC-V
BFD 2.47 ld, i686/AArch64 multiarch glibc headers and a 32-bit linker, so
those local gate failures are toolchain-only and are NOT claimed as green.
Hosted CI has those tools and remains the acceptance gate.  **This section
is NOT a green-CI claim.**

## 11. Final red-team audit of the two-tier patch (line by line)

This is the audit the merged head did NOT yet have.  Every claim below is
backed by a command in §10 / §13 or a unit test in the patch; where a
proposed "optimisation" lost, it was REVERTED rather than documented as a
win.

### 11.1 Design alternatives, quantified

| Alternative | Measurement / proof | Verdict |
|---|---|---|
| `Global(u32)` + `u32::try_from(len).expect(...)` | hosted `audit_census.py`: `panic_sites 863 -> 864`; conversion is lossy on principle | REJECTED. `Global(usize)`, id = `len()` identity, census stays 863 |
| REPLACE SSA-base streams with object identity | hosted job `128896872669`: `movq/vmovq pair ops=0` | REJECTED. Two tiers (§3.1) |
| 2-lane fallback windows inside a long run | SHA-256 recurrence reloads (12-15.6 % slower), illegal `vpslld`, `d4_schedule_pairs` regression | REJECTED, reverted (R20) |
| weaken the CI pair-family gate | would hide a real capability loss, not a defect in the gate | REJECTED |
| accept every legal plan (`cost floor -100000`) | 29/29 correct but Callgrind `1.0374x` WORSE | REJECTED |
| fold crossing penalty into `vector_cost` | kills measured `s_clamp_u8` 0.8399x win | REJECTED; pure `crossings_are_amortized` gate |
| `merged_roots` site-tier gate | Callgrind ON THE COMPILER: `862,129,272 -> 862,637,627 Ir` (+0.059 %), `build_plan` unchanged `4,244,435`, wall-clock ratios 0.9893/0.9589 (noise) | REJECTED, reverted (R24) |
| boolean `joint.is_some()`/`value_sink.is_some()` dedup | latent conflation of distinct `Tail`/`Value` payloads | REJECTED; exact `SeedKey` (R25) |

### 11.2 Line-by-line invariants of the final code

* `StreamBase` derives `Copy`; all payloads are machine words.  No clone in
  the per-store hot loop, no allocation, no panic.
* `SymbolRoots::build` is ONE O(function) scan per fixpoint round.
  `get(name)` precedes `insert(name.clone(), len())`, so only distinct
  symbols allocate.  `usize == len()` is the identity conversion: no
  `try_from`, `expect`, saturation, truncation, overflow or new panic site.
  A 300-symbol Rust test and a 2000-global C stress test pin density,
  uniqueness and panic-free interning.
* `stream_base` maps `GlobalAddr`/`ParamRef` dests to their object id and
  everything else to `Value(v.0)`.  Legality still uses the RAW
  `SymAddr::base` + `RestrictBases::disjoint`; object identity widens the
  SEED set only and never weakens an alias proof.
* `collect_seed_candidates` builds `object_streams` and `site_streams` in
  one pass.  `StreamBase::Value` roots skip the site tier because their
  site key would be byte-identical to the object key.  Joint U8 seeds are
  emitted ONCE (object tier only), so no duplicated tail analysis.
* Entries are sorted by `(off, store_idx)`; offset deltas use
  `checked_sub` (no debug-build i64 overflow panic); `size <= 0` fails
  closed; `preferred_width` fails closed; duplicate offsets end a run.
* `seed_dedup_key` is TOTAL: `(ty, width, store_idx, anchor_ptr,
  Option<Tail>, Option<Value>)`.  It runs BEFORE `build_plan`, so a
  single-web global is priced exactly once.  `Tail` derives `Eq + Hash`;
  a Rust test distinguishes six different tail/sink/anchor payloads.
* `build_plan` is UNCHANGED: the same cross-block use map, the same
  store-interval legality rules, the same loop-carried recurrence veto,
  the same exhaustive `scalar_instruction_weight` / `pack_vector_cost` and
  the same measured `crossings_are_amortized` gate.  No env knobs, no
  unsafe, no mutex/spinlock, no raw pointer, no null dereference, no
  use-after-free, no arithmetic overflow in the new code.
* `candidates.sort_by` is longest-first with earliest-position tie-break,
  preserving the original "longest chains first" contract after dedup.

### 11.3 Tools used, and tools deliberately NOT used

* GDB 18.1 (`break s_jhash`, `info registers rip`), `objdump --disassemble`,
  `readelf`, Valgrind Memcheck (0 errors) and Callgrind (Ir, deterministic)
  on the host CPU; `-O2 -mavx2`, never `-march=native` (SIGILL on this host).
* GCC 16.2 and Clang 23.1.5 from the restored `harness-cache-8` tag; 4 GiB
  `/home/user/.cache/lccc-swap.img` active; `fastbuild` preset;
  `RUSTFLAGS="-D warnings"` throughout.
* ICX 2026.2: NOT installed locally.  Intel PPA `Packages` by-hash fetch
  returned `AccessDenied`, PyPI `dpcpp-cpp-rt 2026.2.0` is runtime-only,
  no GitHub release carries a compiler; the HOSTED `Authenticated Intel
  ICX 2026.2 integration` job is the only ICX evidence and is explicitly
  not claimed as a local run.
* `objtool` validates Linux kernel ORC/unwind metadata and `pahole`
  inspects DWARF struct layouts.  Neither applies to this userspace SLP IR
  pass: there is no kernel object file and no new C struct.  `readelf` and
  Rust `size_of`/`Copy` assertions cover the relevant layout questions;
  this is a scope decision, not a skipped check.

### 11.4 Honest limits (no handwaving)

* The 29-shape three-way Callgrind comparison still shows LCCC/best Ir
  `3.3727x` and only 1/29 shape wins.  "Beat all C compilers" is NOT met.
* The `CCC_NO_BB_SLP=1` ablation geomean is `0.9765` (2.35 % Ir better),
  with 24 shapes unchanged, 3 improved, 2 sub-3 % perturbations.  That is
  NOT a wall-clock claim: the null-calibrated A/B was `1.0100` with CI
  `[0.9761, 1.0418]`.
* No per-function "most efficient implementation exceeding godlike quality"
  verdict is possible from one PR.  The measured claims are limited to the
  exact plans listed in §3.3 / §10 and the gates that pin them.
* Pre-existing debt is preserved, not hidden: `knobs_undocumented 254` vs
  baseline 261, MB-01..MB-45, SV-01..SV-12, GP-01..GP-24, UP-1..UP-9,
  F1..F7, large-model/LTO/DWARF `.debug_info`/full ISA matrix/48-seed
  YARPGen remain open.

## 12. Follow-up PR #1158

PR #1151 was MERGED at `be0c3d1` (main `9f8883e`).  The red-team findings
R24-R27 and the exact-dedup hardening could not be added to a merged PR,
so follow-up PR **#1158** was opened from the same fixed session branch
`arena/9546a706-lccc` with head `9b63061` (single commit on top of
`9f8883e`).  Snapshots: `/home/user/ms178-1.patch`,
`/home/user/ms178-1-9b63061-exact-dedup.patch`, and the two mirrors under
`engineering/snapshots/` (all sha `5c0b217161069385b189d60eecabdfa4c3466e0c274807b8dbc582389433ea03`,
reverse-apply verified).  Hosted CI for #1158 is the acceptance gate; the
final receipts are recorded in §13 once every job is green.
