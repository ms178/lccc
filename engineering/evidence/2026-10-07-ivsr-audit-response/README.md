# 2026-10-07 — audit-response evidence (review of PR #774)

Backing data for
[`../../FOLLOWUP-2026-10-07-ivsr-audit-response.md`](../../FOLLOWUP-2026-10-07-ivsr-audit-response.md),
the round that answered an external audit of the merged review-hardening work
(PR #774, `bb492648`, base `9d85b134`). The audit ran no compiles; every file
here is a measurement that was run, and two of them exist because a claim in the
*previous* round's documentation did not survive contact with one.

| file | what it is |
|---|---|
| `descending-spelling-probe.txt` | the finding-F1 measurement: 16 C spellings of a descending counter, whether each forms a `BasicIV`, the recurrence and header test the frontend actually emits, and whether IVSR collects a derived expression. **10 of 16 form a `BasicIV`**, which falsifies the previous round's "no phi is ever recognised". Narrow unsigned counters record `Add(i, Const(I64(4294967295)))` — a step of `+2^32-1`, not `-1` — and their `i > 0` test canonicalises to `Ne`, which is why A1's descending arm stays unreachable |
| `descending-spelling-probe.sh` | the probe itself, re-runnable (`LCCC=target/fastbuild/lccc bash descending-spelling-probe.sh`). A row reading `COMPILE-FAIL` is a bug in the probe, not a result — an earlier draft silently printed empty rows for the `u16`/`u8` cases because its source generator emitted a cast to a pointer type, and an empty trace is exactly how a false generalisation gets shipped |
| `w1-revert-discrimination.txt` | finding F7's proof. A scratch worktree at the merged head with the A1 hunk reverted to the pre-PR-#774 bound: `descending_unsigned_address_loop_end_to_end` still **passes** (so it asserted nothing about A1, as the audit said) while the new `descending_narrow_offset_product_certificate_is_the_operative_proof` **fails** with "a descending offset that wraps its own ring must not be reduced" |
| `static-neutrality.json` | this round's assembly A/B against the merged base: **1480 comparisons, 0 changed, 1 skipped**, over three corpora, with per-corpus instruction and stack totals. All three totals are byte-identical to the previous round's record |
| `four-vendor-oracle.json` | `godbolt_oracle.py --oracle-set all-vendors` at `-O2`, re-run after the change: **9 agree / 0 diverge / 0 error**; LCCC 1512 instructions vs GCC 16.2 1852 (0.8164), Clang 23.1.0 2274 (0.6649), ICC 2021.10 3714 (0.4071), ICX latest 2441 (0.6194). Identical to the merged base, as byte-identical assembly requires. The per-program `ratio` column is `mean(oracle insns) / lccc insns`, so above 1.0 means LCCC is smaller; against GCC alone the worst program remains `ivsr_index_domains` at 165 vs 118 = 1.398 |

## Executed correctness, same tree

| gate | result |
|---|---|
| `run_regression.py --lccc target/fastbuild/lccc -j2` with `CCC_VALIDATE_SSA=1` | 887 passed, 0 failed, 13 skipped-compare, 0 skipped-run, 900 total, exit 0 |
| `check_ivsr_domains.sh` | exit 0 (`gcc-multilib` + `libc6-dev-i386` installed, so the `-m32` legs run rather than skip) |
| `check_env_test_hygiene.sh` | `ok pass-pipeline environment reads: 153 (budget 153)` |
| `cargo test --lib` | 4156 passed, 0 failed, 7 ignored (merged base: 4155 — one test added) |
| `cargo clippy --all-targets` / `cargo fmt --all --check` | no diagnostics / clean |

## What is deliberately absent

No Callgrind re-run. Byte-identical assembly on all 1480 comparisons implies
identical `Ir`, so the merged round's Callgrind numbers stand unchanged; re-running
would have produced the same figures at the cost of an hour of VM time. No wall-clock
numbers either: this host has a ~15% layout-noise floor and no PMU, and nothing in
this round claims a speedup.
