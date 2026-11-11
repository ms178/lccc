# DA audit (§15, `dcd1822c`) — red-team reassessment and landed work

Date: 2026-11-09. Baseline: `2477d554bf140a21e79c0218d50a69d605e73cc1`
(current `main` when this session started; the audit measured `dcd1822c`,
63+ commits earlier). Host: 2 vCPU / 2 GiB sandbox, swap file installed,
fastbuild (`-O1`, `-j2`) per standing policy, GCC 14.2 as the local oracle
(GCC 16.2 / Clang 23.1 / ICX were not installable in this sandbox; every
layout claim below was chosen to be identical on GCC 14.2 and 16.2, and the
regression gate runs the GCC 16.2 oracle in CI).

Method: every DA item was re-probed against the *current* tree with a
runnable reproducer before any code was touched. Several items were
already fixed upstream after `dcd1822c`; claiming them again would have
been double-counting. Verdicts:

| ID | Verdict on current main | Evidence |
|---|---|---|
| DA-01 | **Mostly stale; step 3 landed here.** Steps 1–2 (line-index resolution, per-scope local-object index) already landed upstream (`560e291`, `d3d0351`, `335de74`). Re-measured: 80k block declarations compile in 1.40 s total (sema 0.36 s), not 41 s. Remaining step 3 (skip the cloned unused-object list when all three warning kinds are disabled) is implemented in this patch; step 5 sibling sweep found only per-struct field lookups (bounded by field count, not declaration count). | `CCC_TIME_PHASES=1` runs at 10k/40k/80k; new scaling leg in `check_parser_depth_limits.sh` pins sema(80k) < 3.0 × sema(40k). Warning output verified byte-identical for default / `-w` / `-Wall` / `-Werror=unused-variable` / `_Pragma` re-enable cases. |
| DA-02 | **Confirmed at the time, since closed (L).** *(This row is the historical finding; see the 2026-11-11 reassessment below and `engineering/BENCH-2026-11-11-BN-MUL128-CLOSURE.md` for the measurement that supersedes the number.)* `tests/bench/k_bn_mul128.c` (new) measures lccc 2.421 ms vs GCC 0.199 ms = **12.2× slower** at `-O3` in this sandbox (Claude: 14.2× on their host); checksums agree, `bench_verify` passes. Assembly: 104 vs 70 instructions in `bn_mul`, three multiplies per 64×64 product vs GCC's single `mulq` + `adc` chain, `is_non_gpr_type` still parks every I128/U128 in a stack slot (`src/backend/regalloc.rs:3392`). | This patch adds the permanent runtime oracle (kernel + verify) so RAP-04/RAP-05 work (§9d) is measured against it. The allocator legalisation itself is **not** in this patch: it is a multi-week programme with its own follow-up (`engineering/RA_FOLLOWUP_2026-10-31.md`) and landing an unvalidated slice of it here would violate the correctness-first rule. |
| DA-03 | **Stale.** The per-case ratchet baseline already exists (`engineering/evidence/gcc17-torture-baseline/*.json`, identity/provenance-checked) and the runner creates its results dir before preflight and records `preflight-failure.txt`. | Source inspection + `FOLLOWUP-2026-11-08-astra-audit-reassessment.md` §DA-04. No change made. |
| DA-04 | **Disagreed, deliberately.** The budgets are not hand-raised numbers: each carries a measured stack-cost calibration in `parse.rs` (e.g. pointer 65,536 vs the ~400k clone/drop overflow, with `check_parser_depth_limits.sh` pinning both ends), and record nesting is additionally capped because post-parse layout is quadratic in depth (9.7 s at 8,000). GCC's limits tests are *designed* to exceed implementation limits; a clean single diagnostic (current behaviour, gated) is the correct contract. Making the descents iterative is a valid long-term item, but not at the cost of destabilising the parser for three adversarial corpus files. | Gate output: the three cases reject with exactly one bounded diagnostic, rc=1, no signal. |
| DA-05 | **Partly stale, partly disagreed.** RELR: already implemented (verified: `-pie -Wl,-z,pack-relative-relocs` emits `DT_RELR/RELRSZ/RELRENT`, 4 packed locations, binary runs; `GLIBC_ABI_DT_RELR` version need emitted at `emit_exec.rs:338`). `--build-id`: implemented when requested (verified SHA-1 note). Default PIE / default build-id: **rejected** — both are distribution *specs* defaults, not GCC/ld defaults; upstream GCC and GNU ld both default to no-PIE/no-build-id, and this repo has a measured, gated decision that a position-fixed `ET_EXEC` pays no PIE price (`tests/regression/check_default_codegen_mode.sh`). `-static-pie`: still an honest hard error; keep until `rcrt1.o` support is real. | `readelf -h/-d/-n` probes in session log. |
| DA-06 | **Confirmed open (L, §9d).** Only re-measured here via DA-02's kernel (12.2×). RAP-01…12 stand; no allocator change in this patch (see DA-02). | bench_kernels output above. |
| DA-07 | **Two items landed; three were stale.** Landed: `-fpack-struct[=<n>]` (was a hard error; `pr67106` torture failure) and `-mcpu=` on x86 (was refused). Stale: `mode(SI)` pointer attribute — GCC 14.2 rejects the probed form with the same "invalid pointer mode 'SI'" diagnostic lccc gives; `({ })` — GCC 14.2 and lccc agree on every probed context (expression statement OK, value/condition use = "void value not ignored"); the float `asm` operand case compiles in both. `dll.c`/`pr29201`/`pr115143-*`: baseline/oracle-budget items, no code change (as Claude also concluded). | `tests/regression/check_pack_struct_mcpu.py`: 35/35 checks pass against GCC, including runtime layout output. |
| DA-08 | **Stale.** The four PRs are merged; nothing to re-land. | Prior follow-up §DA-11; GitHub state. |
| DA-09 | **Noted, no structural change.** Census ratchet exists; splitting 3,592-line `allocate_registers` is §9d work, not a drive-by refactor. | — |
| DA-10 | **One landed, two stale, one kept.** Landed: the two x86 trampoline tests now carry the real owner ID `AUD-X86-LTC-1` (defined in `loop_trampoline.rs`) instead of the ARM `AUD-ARM-GSF-1`. Stale: the i686 "disabled peephole pass" comment documents a pass that was investigated, deleted and restored as a CFG pass in 2026-09 (the comment is the design record, not a TODO); the unresolved-`typeof` fallback — every probed unresolvable form (undeclared identifier, bad member, undeclared callee) is already diagnosed by sema with GCC-compatible wording, so adding a second diagnostic at the fallback would double-report; the TODO comment stays until someone produces a form that reaches it silently. Kept: i686 `-m16` boot-size TODO in `external_tools.rs` (a stated size goal, not a bug). | Probes in session log. |

## Landed in this patch

1. **DA-01 step 3** — `warn_unused_scope` returns before cloning the
   unused-object list when none of `UnusedVariable`,
   `UnusedButSetVariable`, `UnusedConstVariable` is enabled (consulting
   `_Pragma` warning scopes via `warning_enabled`). This is the default
   and `-w` common case on every scope exit.
2. **DA-07 `-fpack-struct[=<n>]`** — bare form packs every record like
   `__attribute__((packed))` (explicit member alignment still escapes;
   `#pragma pack` becomes inert with GCC's `-Wpragmas` warning); valued
   form seeds the `#pragma pack` baseline (pragmas override, bare reset
   restores the cap); invalid values (`0`, `3`, `32`, non-numeric) are a
   hard error with GCC's wording. Plumbing: `Driver::pack_struct_cap /
   pack_struct_all` → `Parser::set_pack_struct` at both pipeline parse
   sites, so `-fsyntax-only` agrees with full compilation. Verified on
   x86-64 (static asserts + runtime output vs GCC) and compile-checked on
   i686/AArch64/RISC-V drivers.
3. **DA-07 `-mcpu=`** — x86: deprecated `-mtune=` alias (warning, same
   vocabulary validation, never touches `-march=`; assembly identical to
   the `-mtune=` spelling). Non-x86 keeps the refusal.
4. **DA-10** — `AUD-X86-LTC-1` relabel + owner note.
5. **DA-02 oracle** — `tests/bench/k_bn_mul128.c` (auto-discovered by
   `scripts/bench_kernels.py`), with a 64-bit-only `bench_verify`.
6. **Gates** — `tests/regression/check_pack_struct_mcpu.py` (new, wired
   into the existing `pragma-pack-underflow` fast gate);
   `check_parser_depth_limits.sh` updated (`-fpack-struct` leg now asserts
   implementation, invalid cap still fails closed) plus a new DA-01
   scaling-shape leg; Rust unit tests for both flag families in
   `src/driver/cli.rs`.

## Deliberately not done (with reasons)

* **DA-02/DA-06 allocator programme** — L-effort, §9d/RAP-owned; this
  patch supplies its missing runtime oracle instead of an unvalidated
  partial legalisation.
* **Default PIE / build-id (DA-05)** — see verdict above; changing the
  default would break the gated ET_EXEC performance contract and is not
  what upstream GCC/ld do.
* **Iterative parser (DA-04)** — measured budgets + clean rejection are
  the safer contract today.
* **`-static-pie`** — keep the honest hard error until `rcrt1.o` exists.

## Validation (this sandbox)

* fastbuild `-O1 -j2`, `-D warnings`: clean.
* `check_pack_struct_mcpu.py --reference gcc`: 35/35 PASS.
* `check_parser_depth_limits.sh`: 17/17 PASS (incl. new scaling leg).
* Warning-parity probes (DA-01): identical output across 6 flag sets.
* `bench_kernels.py --kernels bn_mul128`: correctness PASS, 12.2× gap
  recorded above.
* `cargo test --lib` for the new unit tests could **not** run here: the
  lib-test rustc is OOM-killed at ~1.8 GiB RSS on this 2 GiB sandbox
  (global OOM in `dmesg`, pre-existing host limit — the same kill occurs
  with optimisation/codegen-unit overrides). The tests are written to the
  existing `mtune_vocabulary…` pattern and must be confirmed by GitHub
  CI's `cargo-test` gate; the flag behaviour they assert is additionally
  covered end-to-end by `check_pack_struct_mcpu.py` against the built
  binary. `scripts/ci_local.sh --fast` is subject to the same host limit
  (its `cargo-test` gate) plus missing local GCC 16.2/Clang 23 oracles;
  the compiler-free and built-binary gates named above were run directly.

## Follow-up (next session, in ROI order)

1. RAP-04/RAP-05 wide-integer legalisation, gated by `k_bn_mul128`
   (target ≤ 1.1× GCC) plus Poly1305/Montgomery holdouts.
2. DA-04 iterative-descent spike *only* if a real (non-limits) TU ever
   hits a budget.
3. `-static-pie` (`rcrt1.o`) if a golden workload requests it.
4. A reachable reproducer for the `typeof` fallback, or delete the TODO
   with the probe matrix as evidence.

## Post-merge red-team sweep (2026-11-11, `fc8350a`)

Baseline is the merge of PR #1174 (`fc8350a`). The toolchain is the pinned
`harness-cache-8` tag (`d66c5ac881e2`): Rust 1.99.1, GCC 16.2, Clang 23.1.5,
GDB 18.1, Valgrind 3.19, binutils 2.47, qemu-user 11.1.3, plus exactly 4 GiB
of active swap. Detailed evidence and decisions live in
`engineering/REDTEAM-2026-11-11-I128-TYPEOF.md`.

| Follow-up | Post-merge verdict |
|---|---|
| 1. RAP-04/RAP-05 | **DA-02 runtime gap closed; allocator programme still open.** The generic x86-64 128x128 multiply was the last stack-round-trip in the dynamic multiply path and is now an 8-instruction register-only sequence. Re-measured on 2026-11-11 with the same harness (`scripts/bench_kernels.py --kernels bn_mul128 --reps 9 --inner 1000`), `k_bn_mul128` went from **12.2× slower to parity–1.3× faster than GCC 16.2** and parity with Clang 23.1.5; per-pass attribution is 4.63× from `i128_split` and 3.74× from `index_hoist` (`engineering/BENCH-2026-11-11-BN-MUL128-CLOSURE.md`). The allocator/IR legalisation is still not complete, and the suite-wide geomean is 0.86× GCC — the widest remaining shape is byte classification (0.19×), root-caused in the same document. |
| 2. DA-04 | **No change.** A scan of 3,249 real `tests/**/*.c` TUs found max parenthesis nesting 8; the only 16,385-brace file is the deliberate `clang-c/Parser/parser_overflow.c` adversarial case. No real TU reaches a budget. |
| 3. `-static-pie` | **No change.** No golden workload requests it and `rcrt1.o` support is still absent; the honest hard error stays. |
| 4. `typeof` TODO | **Done.** A 15-shape negative matrix (identifier, member, arrow, call, subscript, address-of, deref, nested, sizeof, bad subscript, cast, binary, comma, conditional, statement expression) is rejected by LCCC, GCC 16.2 and Clang 23.1.5 with a named unresolved operand. A positive 7-result `typeof` matrix is bit-identical. The fallback comment is now an explicit recovery-only contract; adding a second diagnostic would double-report. |

The sweep also removed the unreachable x86 `prep_i128_binop` twin that still
contained the old push/pop sequence, and added raw-emitter, peephole-disabled,
`-O2`/`-O3`, ELF/Capstone, cross-ISA and differential regression gates.
