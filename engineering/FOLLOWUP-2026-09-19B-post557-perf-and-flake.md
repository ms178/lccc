# Follow-up: post-#557 audit — hot-path cost, harness wiring, and a live test flake

Date: 2026-09-19 (session 56)
Base: `407eb492` (merge of PR #557) — the tree is CI-green on all three checks.

PR #557 fixed the CI blocker from PR #555 (`cargo fmt --check`, which runs
before clippy in the *Clippy* job) and, more importantly, found three real
defects #555 had shipped: the unsound `rcx_is_live_at` kill heuristic, the
missing `vec_arx` ISA gate, and `encode_push`'s rejection of segment
registers. This session audits #557 itself. It found no correctness defect in
the merged code; it found **cost** defects in the merged fixes, two harness
wiring gaps, and one **live CI flake** that predates both PRs.

## 1. `is_label_like` — two heap allocations per call on the hottest predicate in the assembler

#557 kept #555's guard verbatim:

```rust
if parse_integer_expr(s).is_ok() { return false; }   // first statement
```

`is_label_like` is called from ~20 sites in `parser.rs` and runs for every
candidate symbol of every operand of every assembly line. On a **miss** — i.e.
for every ordinary symbol name, which is the overwhelming majority of calls —
`parse_integer_expr` allocates an error `String` in `parse_single_integer`
(`format!("bad integer: {}")`), then allocates a second one in `tokenize_expr`
(`format!("unexpected char '{}' …")`) before the result is discarded.

Replaced with an allocation-free byte scan. Equivalence is structural, not
empirical: a string that survives the label charset contains no operator,
parenthesis, sign, quote or whitespace, so the evaluator can only accept it
through its single-literal fast path — and that path necessarily begins with an
ASCII digit. Identifiers therefore never enter the scan at all.

Measured against the real evaluator, which was included verbatim into an
out-of-tree harness (`#[path = ".../src/backend/asm_expr.rs"] mod asm_expr;`,
built with `rustc -O --edition 2021`) so the "before" side is the shipped code
path and not a model of it. Corpus weighted 18:2 symbol:literal, as in a real
object file:

```
parity: OK on 22 tokens
calls: 1136000  main: 156.86ms (138.1 ns/call)   new: 14.19ms (12.5 ns/call)   speedup: 11.06x
calls: 1136000  main: 153.10ms (134.8 ns/call)   new: 18.52ms (16.3 ns/call)   speedup: 8.27x
```

Guarded by `test_bare_integer_literal_matches_shared_evaluator`, which
exhaustively compares the recognizer against `parse_integer_expr` over all
69,904 strings of length ≤ 4 drawn from a 16-char alphabet covering every
decision the recognizer makes (radix prefixes in both cases, valid and invalid
digits per radix, the octal-vs-decimal selection on a leading zero, suffix
stripping, charset-only separators), plus width-boundary cases the corpus
cannot reach (16/17 hex digits, 63/64 bits, i64::MAX and 2^63 in octal,
u64::MAX and u64::MAX+1 in decimal, and leading-zero padding).

Note the radix subtleties the recognizer has to reproduce exactly: a leading
zero selects octal and **never falls back to decimal**, so `09` is not a
literal; and width limits are applied to the *significant* digits, because
`from_str_radix` tolerates leading zeros (`0x0000000000000000001` is 1).

## 2. `rcx_is_live_at` — #557's correct kill test is expensive, and the caller runs it three times

#557 replaced the mnemonic-prefix heuristic with the crate's acceptance-grade
predicates, which is the right call. The cost was not accounted for: per
candidate line the kill test now runs `writes_family_full` (which itself walks
the `implicit_full_write_refs` mnemonic table and re-splits the destination),
`implicit_read_refs` (a **second** mnemonic-table walk through
`classify_implicit_operands`), `is_read_modify_write`, and four independent
`contains` substring searches.

Meanwhile `eliminate_rcx_address_copy` still rebuilt the successor index `k`
and re-ran the whole walk in each of its three sub-patterns — up to **three
full walks per candidate**, for an answer that cannot differ between them (the
walk is pure and nothing mutates in between; every success path `continue`s).

Three changes, all semantics-preserving:

* The walk is computed **once per candidate**, behind a guard that is exactly
  the union of the three sub-pattern guards (`line_j.starts_with("mov")` — both
  `movq (%rcx), %rax` and `movsd (%rcx), %xmmN` start with "mov"), so it is
  never paid for a line no fold could use.
* The conjunction is ordered cheapest-and-most-selective first. The common case
  for a line that mentions the family is a **use**, which the single-pass
  source scan now settles before any predicate is called. The cached
  `reg_refs` bitmask test also moved ahead of the first touch of line text.
* `mentions_rcx_family` replaces the four `contains` calls with one scan
  anchored on `%` (unit-tested against the exact four spellings and the
  look-alikes a coarser search would over-match, e.g. `%ch`, `%rrcx`).

## 3. A robustness defect in #557's source-operand split

#557 derived the source operand by length arithmetic:

```rust
let dest_token = t.rsplit(',').next().unwrap_or(t).trim();
let head = &t[..t.len() - dest_token.len()];
```

`dest_token` is **trimmed** but the cut is taken from the **raw** length, and
`LineStore::get` returns the raw line slice — `LineStore::new` splits on `\n`
and never trims the right edge. With four or more trailing blanks (or a stray
`\r` from a CRLF file plus blanks) the window reaches back into the destination
itself, the source side then appears to mention `%rcx`, and a genuine
full-width kill is reported as a use: the address copy survives. Conservative,
so not a miscompile — but it silently makes the fold dependent on invisible
trailing whitespace in externally supplied assembly.

Fixed by splitting once on the last comma (`rsplit_once`), which is also one
pass instead of two. `test_rcx_address_copy_full_kill_ignores_trailing_whitespace`
pins it. All six of #557's soundness tests still pass unchanged.

## 4. ISA gate placed behind the work it gates

`vectorize_const_trip_map_loops` put its (free) thread-local ISA gate **fourth**,
after a whole-function `DynAlloca` scan, `func_has_volatile_loop_access`, and a
`std::env::var("CCC_NO_MAP_VEC")` allocation. A gated TU — the kernel's
`-mno-sse -mgeneral-regs-only` contract, i.e. precisely the case #557 added the
gate for — paid two IR walks and an environment allocation per function to
reach a decision that was made before the pass ran. Hoisted to first; all four
guards are pure `return 0`, so ordering is semantic-free.

`vec_arx_function` already gates first (#557), and the const-trip call site in
`passes/mod.rs` is already `Target::X86_64`-only, so neither gate can starve
AArch64/RISC-V (whose `X86Isa` is `NONE` and would otherwise read as "SIMD
unavailable").

## 5. `boot_size_oracle.sh` sourced `boot_offsets.sh` and never called it

#557 added `. "$here/boot_offsets.sh"` but no `ensure_boot_offset_stubs` call,
while `build_kernel_boot.sh` does call it. `LCCC_BOOT_OBJS` (from
`boot_flags.sh`) includes `header`, and `header.S` includes `zoffset.h` /
`voffset.h` for `ZO_efi*_stub_entry` / `VO__text`. The oracle therefore died on
the missing header instead of reporting a size delta — the exact failure the
helper exists to prevent. #557's own follow-up #4 records that the boot
harnesses were never run, which is how this survived.

Wired up after `cd "$K"`. Both toolchains compile the identical stubbed header,
so the oracle's contract ("a delta is a code-generation delta and nothing else")
holds.

## 6. `qemu_boot_test.sh` firmware union assumed one package split

The union guard hard-coded a single assignment — BIOS in `$bios_dir`, all three
option ROMs in `$rom_dir` — while the loop that stages the union already
resolved each file independently. Any other complete split (a bundle with the
BIOS beside the ROMs but not all four in one directory, or a distro that puts
`efi-e1000.rom` with SeaBIOS) was rejected with "could not find a complete QEMU
firmware set", and the naive loosening of that guard would have created
dangling symlinks.

Now resolved per file: accept the union iff every required file exists in one of
the two directories, and symlink the resolved path. Verified against four mock
layouts — Debian split, reversed split, mixed split, and a genuinely incomplete
set (correctly still rejected), with no dangling links in any accepted case.

## 7. A live CI flake: `CCC_NO_TWO_BLOCK_UNROLL` mutated the process environment under parallel tests

Observed directly during this session's verification: a full
`cargo test --all-targets` run failed
`passes::loop_unroll::tests::two_block_unroll_profitability_positive_control`
with `load+store latch must unroll: left 0, right 1`, on a tree whose only
changes were in the assembler parser, the peephole and a vectorizer guard — and
the same test had passed in the previous run.

Root cause: `two_block_unroll_declines_when_killed` did

```rust
unsafe { std::env::set_var("CCC_NO_TWO_BLOCK_UNROLL", "1") };
let n = unroll_loops(&mut func, UnrollPhase::PostVec);
unsafe { std::env::remove_var("CCC_NO_TWO_BLOCK_UNROLL") };
```

while `unroll_loops` itself read that variable (`loop_unroll.rs:253`).
`std::env::set_var` is process-global and libtest runs tests on parallel threads
of one binary, so every sibling `unroll_loops(.., PostVec)` test that happened to
run inside the window saw the kill switch ON.

Reproduced and measured on the merged tree:

```
two_block_unroll group, --test-threads=2:  FAILING RUNS 11 / 150   (~7%)
```

which matches the "~1-in-10 flake in this module" that `loop_invert.rs` documents
for the identical defect — and that module already states the fix: *"Configuration
a caller can supply belongs in a parameter, not in ambient global state."*

Applied the crate's own established remediation (the `set_x86_simd_isa` pattern):
a `TWO_BLOCK_UNROLL_ENABLED` thread-local defaulting to enabled, a
`set_two_block_unroll_enabled` setter called once at the top of `run_passes`
(which is where both `unroll_loops` call sites live, spans 921–2594), and the
kill-switch test now toggles per-thread state instead of the environment. As a
side benefit the hot path no longer reads the process environment per call.

```
after:  two_block_unroll group, 200 runs  ->  FAILING RUNS 0 / 200
```

The driver contract is unchanged and verified end to end:
`tests/regression/check_two_block_unroll_redteam.sh` (which exercises
`CCC_NO_TWO_BLOCK_UNROLL=1` differentials) reports
`ok: two_block_unroll_redteam (runtime, tri-config, kill-switch, SSE2 baseline,
Max kill-switch, asm contracts)`.

The same `set_var` pattern exists in `vec_interleave.rs`, `vec_load_sink.rs` and
`loop_align.rs`. Measured at 0/120 each, so they are latent rather than live and
were left alone; `location_alloc/policy.rs` is already correct by construction
(uniquely named throwaway variables, with a SAFETY comment saying so).

## Verification (final tree)

* `cargo fmt --all -- --check`: green.
* `cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings`: green.
* `cargo test --profile fastbuild --all-targets --locked -j 2` with
  `RUSTFLAGS=-D warnings`: **2917 passed, 0 failed, 6 ignored** (5 new tests).
* `two_block_unroll` group: 0/200 failing runs (was 11/150).
* `tests/regression/check_two_block_unroll_redteam.sh`: ok.
* `tests/regression/check_vectorize_isa_gate.sh`: PASS.
* `bash -n` on both changed shell scripts; `git diff --check` clean.
* QEMU firmware union: 4/4 mock layouts behave correctly.
* `is_label_like`: exhaustive evaluator parity over 69,904 inputs + width boundaries.

Boot-size harnesses still cannot run here (no kernel tree), as in #557.

## Follow-ups not taken (deliberate, with reasons)

1. **`Ret` is not a scan barrier in `rcx_is_live_at`.** Control cannot flow past
   a `ret`, so `%rcx` is genuinely dead there and returning `false` would be
   sound and would enable more folds. Not taken: it changes codegen decisions,
   and the scan already terminates a few lines later at the next function's
   label, so the compile-time benefit is negligible. Needs the kernel harness to
   validate the codegen side.
2. **`writes_family_full` misses single-operand full redefinitions.**
   `dest_operand_is_full_width` splits on the last comma, so `popq %rcx` — a
   genuine full kill — is not recognized and the address copy survives.
   Conservative and pre-existing in a shared helper used by several passes;
   fixing it is a cross-pass change.
3. **`xorl %ecx, %ecx` as a kill** — #557's own follow-up #3, unchanged.
4. **`note_home_written` reads `std::env::var_os("CCC_TRACE_NOTES")` per call.**
   A per-instruction environment lookup in `emit.rs`; #557's typed-call change
   routes slightly more traffic through it. Pre-existing and unrelated to this
   series.
5. **`vectorize_with_analysis_mode` computes `find_natural_loops` before its ISA
   gate** — the same class as §4 and more expensive. Not hoisted because the
   `LCCC_DEBUG_VECTORIZE` diagnostic prints the loop count, so moving the gate
   would change observable debug output.
