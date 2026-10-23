# FOLLOWUP 2026-10-23 — red-team audit of the open PRs, and what the layout ties actually buy

Four PRs were open against `44abd1b` when this audit started (#923, #926, #929,
#932), and the previous twenty were closed or merged. This document records what
each one is, what it was measured to do on *this* tree, what was taken, and what
was refused — including the two claims that did not reproduce.

Host for every number below: 2 vCPU Xeon @ 2.60 GHz (SMT pair, no PMU),
GCC 16.2.0 (`harness-cache-1` bundle), Clang 23.1.3 (`harness-cache-4`), rustc
1.97.0 (`rust-cache-1.97.0`), `-O2 -std=gnu17`, `--reps 11..13` interleaved
paired rounds through `tests/benchmark/run_benchmarks.py`, which pins the run to
one CPU and randomises the compiler order within each round.

## The audit target: what each open PR is

| PR | files / lines | CI at audit time | mergeable | what it actually contains |
| --- | --- | --- | --- | --- |
| #932 | 3 / +35 −3 | 17 pass, 3 pending | DIRTY | a dual-layout tie-break (`src/backend/generation.rs`, +19 −1) plus docs and a census bump. Its other half — `coalesce_forwarding_phi_copies` — is already on `main` (#928, hardened by #930). |
| #926 | 83 / +10334 −711 | 21 pass | DIRTY | the SIMD stack: `aggregate_pair_roundtrip` (aggcopy16), `movdqu`-class slot forwarding in `store_forwarding.rs`, a rewritten `vectorize.rs`, intrinsic/ABI plumbing. Declares itself unvalidated (its author had no cargo, GCC 16 or Clang 23 on the box). |
| #923 | 60 / +11739 −276 | 17 pass | DIRTY | the same stack at an earlier point: SSE round-trip collapse, baseline matmul vectorization. |
| #929 | 63 / +11110 −272 | **3 fail** | DIRTY | #923 plus a per-instruction SSE2 ratchet; numerically the least healthy of the three. |

All four are `CONFLICTING` against `main`: #923/#929 are ~11 000 lines each on a
base from before #931, #926 is a superset of #923 with 30 overlapping `src/`
files, and rebasing any of them means resolving conflicts in
`vectorize.rs`, `passes/mod.rs`, `module_symbol_facts.rs` and the CI gate
scripts by hand, then re-validating 10 000 lines of SIMD semantics that no
locale toolchain here can even build end-to-end. That is a rewrite, not a
merge, and a rewrite of unvalidated code is exactly the "workaround dressed as a
fix" this repository is supposed to refuse.

## Measured: the SIMD stack is unreachable from the benchmark corpus

`src/backend/x86/codegen/peephole/passes/store_forwarding.rs` on `main` has no
`MoveSize::DQ` at all: 16-byte XMM slot moves are not classified, so the
round trip `movdqu %xmmN, k(%rsp)` / `movdqu k(%rsp), %xmmM` never forwards.
That is the gap #926 closes — and it is worth closing *for code that keeps
`__m128` in memory*. The 49-program corpus does not:

```
$ for p in matmul nbody spectral_norm double_reduction mandelbrot ...; do
    lccc -O2 -S tests/benchmark/programs/$p.c -o - | grep -cE 'movdq[au] +%xmm[0-9]+, -?[0-9]+\(%rsp\)'
  done
0   (every program)
```

Zero 16-byte stack operands in every FP benchmark: the vectorizer keeps results
in registers, and the corpus is scalar C, so no `xmmintrin.h` aggregate crosses a
call. `aggregate_pair_roundtrip` (aggcopy16) has the same reachability problem —
it fires on the `I128` carrier of a 16-byte aggregate crossing a call boundary,
which the corpus does not produce either.

So the #923/#926/#929 stack cannot be *measured* here, only *reviewed*, and a
review is not evidence. It is left open, not taken. The one thing the audit
does confirm from reading: their soundness arguments (single-block pattern,
single-use intermediates, non-overlapping ranges, memory-clobber barriers,
multi-definition refusal) are the right ones, and `aggcopy16` should be the
first thing revisited when a corpus with SSE-shaped aggregates exists.

## The one measurable claim: #932's tie-break

### It is not a corner case

The dual-layout emission builds both the static-chain order and the pre-chain
order and keeps the one the metric likes. On the 49 benchmark programs
(`CCC_DEBUG_CHAIN=1`, counting `[LAYOUT]` records):

```
dual-layout sites = 44
exact ties        = 32   (73 %)
```

Three quarters of the functions the machinery touches are ties, and the
historical rule (`chain_count < orig_count`) handed every one of them to the
order the passes happened to append. That is a real defect in the decision rule,
and #932's reading of it is correct: on a tie the metric has no opinion, and the
chain order's fall-throughs were at least chosen by a greedy walk over the same
structural weights the metric sums.

### But the claim attached to it does not reproduce

#932 reports `expat_xml_scan` 55.66 → 51.01 ms (−5.6 %) and a 0.9757 geomean
over 67 programs. On `44abd1b`, with the same 16-benchmark set, 11 interleaved
reps, the tie-break moved `expat_xml_scan` from **50.28 to 50.46 ms**
(+0.4 %, inside the paired CI). The win belonged to the PR's own base: the
phi-copy coalescing it was rebased over changed which functions tie and what
the append order already is. This is the second half of the same lesson #930
taught about #928 — a performance number is only valid against the tree it was
measured on.

### And taken literally, it costs more than it pays

A/B of the tie rule exactly as written (`chain < orig || chain == orig`) against
`main`, same set, 11 reps (`/tmp/bench/focus.json`):

| benchmark | main | tie rule | Δ |
| --- | ---: | ---: | ---: |
| `hot_web_loop` | 103.08 ms | 99.71 ms | −3.3 % |
| `sqlite_varint` | 28.15 | 27.31 | −3.0 % |
| `llvm_word_counts` | 48.68 | 47.30 | −2.8 % |
| `switch_dispatch` | 554.35 | 539.60 | −2.7 % |
| `struct_copy` | 21.32 | 20.75 | −2.7 % |
| `binary_search` | 1.94 | 1.89 | −2.6 % |
| `llvm_scalar_facts` | 77.31 | 75.75 | −2.0 % |
| `linux_rbtree` | 16.98 | 16.69 | −1.7 % |
| `arith_loop` | 111.91 | 110.45 | −1.3 % |
| `bitops` | 214.26 | 212.25 | −0.9 % |
| `expat_xml_scan` | 50.28 | 50.46 | +0.4 % |
| `qsort` | 134.77 | 135.28 | +0.4 % |
| `sieve` | 39.85 | 40.41 | +1.4 % |
| `zstd_count` | 10.04 | 10.21 | +1.7 % |
| `ascii_case_fold` | 1.98 | 2.05 | +3.5 % |
| `strlen_bench` | 226.27 | 241.62 | **+6.8 %** |

Geomean 0.9941 (10 better, 6 worse): a real but thin win carrying a 6.8 %
regression on one benchmark. Landing that as-is would repeat the exact mistake
the stack-traffic veto was added to prevent.

### The synthesis: a strict stack rule *on ties only*

The 10 % stack band exists because "a wrongly rejected layout costs the whole
placement win". On a tie there is no placement win to protect — the metric has
already declined to separate the orders — so the only remaining evidence is the
stack term, and there the honest comparison is strict:

```rust
let tie = chain_count == orig_count && !CCC_LAYOUT_TIE_ORIG;
let stack_ok = if tie { chain_stack <= orig_stack }
               else   { chain_stack * 10 <= orig_stack * 11 };
let adopt = stack_ok && (chain_count < orig_count || tie);
```

That is the whole change: one predicate extracted into `adopt_chain_layout` so
the rule is unit-testable instead of a comment, the historical band kept exactly
where it was measured, and the tie band made strict where it is new.

### The final rule, measured

Three independent paired runs (interleaved, pinned, warm-ups excluded):

| run | set | reps | `expat_xml_scan` main → tie-strict | Δ | suite geomean |
| --- | --- | ---: | ---: | ---: | ---: |
| A | 21 benchmarks | 13 | 51.94 → 48.30 ms | −7.0 % | 0.9965 |
| B | `expat`, `strlen`, `switch` | 31 | 52.29 → 47.80 ms | −8.6 % | — |
| C | `expat`, `strlen`, `switch` | 31 | 53.73 → 49.07 ms | −8.7 % | — |

Run A, full table of the eleven benchmarks that moved more than 1 %:

```
sieve              39.79 →  38.30   -3.7%
binary_search       2.20 →   2.10   -4.6%
llvm_word_counts   47.51 →  46.00   -3.2%
chacha20_block    273.78 → 267.66   -2.2%
arith_loop        108.95 → 107.40   -1.4%
glibc_memcmp        8.14 →   8.03   -1.3%
bitops            212.55 → 217.76   +2.5%
strlen_bench      217.14 → 222.55   +2.5%
qsort             130.53 → 135.41   +3.7%
llvm_scalar_facts  71.05 →  75.12   +5.7%
```

`expat_xml_scan` is the only benchmark whose movement is both large and
*reproducible*: three runs, three hosts-worth of noise, −7.0 / −8.6 / −8.7 %.
The rest is the 2 vCPU SMT box: `strlen_bench` was +6.8 % at 11 reps in the
first run and −2.6 % at 31 reps, which is what "no PMU, two threads" looks like.
The rule is therefore justified by the one case it fixes, and the strict stack
band is what keeps it from being a licence to spill.

Equivalence to the measured binary, checked rather than assumed: the landed
tree and the arm that was measured compile all 67 sources in
`tests/benchmark/programs` to byte-identical assembly (`-O2 -S`, md5 per file,
67/67 identical).


## What was taken, and what was not

Taken from #932: the tie rule itself, rewritten as `adopt_chain_layout` with the
strict stack band above, its own unit tests, and this document.

Not taken, with reasons: the #923/#926/#929 SIMD stack (unreachable from the
corpus, therefore unmeasurable here, and a 10 000-line conflict resolution whose
correctness could not be revalidated on this host); #932's `expat_xml_scan`
claim (does not reproduce on this tree); #932's census bump (its +1 was for a
knob its own commit added, which this commit re-does against the current
baseline).

## The last twenty closed PRs

#908–#931 are merged into `main` except #918, #928, #904 and #903, whose content
arrived through #926/#930/#931 (fold + coverage rule, the four audited fixes to
#928, and the sret/hoist/inline work). The audit re-checked the two that carry
soundness arguments rather than performance ones:

* **#930's coverage rule** (`s_defs.len() == preds.len()`) is still the thing
  that keeps `fold_entry_load_relay` sound; its assertion still flips when the
  rule is removed.
* **#931's sret retarget** now declines a candidate whose destination alloca is
  defined *after* the call (`CCC_VERIFY_IR=abort` found this on
  `ZSTD_decompressSequences*`), and declines one whose destination is read or
  written between the call and the copy.

Both are covered by unit tests in the files they live in.

## Environment limits of this audit

* **ICX 2026.2 could not be provisioned here.** `scripts/icx_provision.py` pulls
  from `https://apt.repos.intel.com/oneapi`, which is not reachable from this
  build sandbox (`SSL_ERROR_SYSCALL` on connect; the sandbox allows GitHub,
  npm and PyPI only). Every ICX comparison therefore stays CI-side; none of the
  numbers above involve ICX, and none of them are dressed up as if they did.
* **gdb, pahole, objtool and perf are not in the cached bundles** and cannot be
  installed (same egress limit; `apt-get install gdb` reports "no installation
  candidate"). Evidence was gathered with `objdump -d`/`readelf` plus the
  tools the bundles *do* carry — `valgrind` 3.19 (installed from
  `harness-cache-1`'s `debs-bookworm` archive), `qemu-i386`, `binutils` 2.47 —
  and with the compiler's own IR/asm dumps.
* The clone is shallow; merge-bases were recovered with `git fetch
  --deepen=400`, without which none of the three-dot diffs above are meaningful.
