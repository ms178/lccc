# Debug parity and exact FP policy: audit and evidence

## Scope and independent assessment

Baseline: merged main `7e9444174f9b76181504329cd4ff8010b1a1483b`.
The prior change had native/Callgrind evidence in its delivery, but not in the
PR body. That discoverability criticism is valid; "no numbers existed" is not.
Earlier gains against an older main are not gains from this follow-up.

The proposed debug parity check uncovered **seven pre-existing failures in
57 programs**, not just the newly reviewed jump pass. Mandelbrot/XML already
passed on the baseline. Traces identified GP relay/staging/extension folds,
FP load folding, LEA folding and vector-copy adjacency as the other causes.
Unpinned inert debug markers now cross those instruction windows without
being discarded; CFI, alignment, data, sections and inline asm remain opaque.
Control-flow cleanup retains its existing phase-order boundaries: blindly
extending every NOP walk interfered with identical-block merging and was
rejected by an unchanged regression test.

The directive guard now sees `Directive` in any alternative position within
the supported `matches!` grammar. **36 witness checks; 10 exemptions** (formerly
8). One new exemption selects branch lines; one marks an i686 block boundary.
The other two cited i686 sites already consult `opaque_data_directive` and
need neither an exemption nor behavior changes. This remains a source guard,
not a general Rust parser or a substitute for execution tests.

FP doubling protects contraction inputs only when contraction is permitted
and the resolved target ISA has FMA. No-candidate functions avoid map allocation;
unchanged modules avoid the additional verification call. Also removed a
redundant positive-change loop-square checkpoint: `timed_pass!` already verifies.
Named square plans and numeric local-label keys replace positional/string data;
conditional-opcode recognition uses a stack buffer, not per-branch allocation.

Size mode is explicit at both production peephole entrypoints. It disables
jump threading rather than guessing instruction layout or deleting labels
whose address/metadata uses are not proved absent. Unused-relay deletion is
**not** claimed. Cross-target FP and i686 threading expansion remain unproved.

## Four-arm bounded corpus (44 workloads)

Flags: O2, x86-64-v3, fwrapv, checked-in reduced problem sizes. All four arms
completed all 44 cases with matching stdout. `on` compares against merged main;
the disabled arms compare against the candidate with every pass enabled.

| Arm | Mandelbrot Ir | XML Ir | Other changes versus candidate-on |
|---|---:|---:|---|
| Candidate on | 6,005,656 | 1,666,838 | All 44 equal merged main |
| `loop_square` off | 5,599,386 | 1,666,838 | None |
| `fp_double` off | 6,005,657 | 1,666,838 | None |
| `jump_thread` off | 6,005,656 | 1,696,333 | lz4_compress +1; lz4_match_extend +1 |

The existing recurrence has **higher Ir** despite earlier native gains; do not
call an instruction count a speedup. Single-instruction whole-process deltas
are not evidence of a meaningful runtime gain. No new no-debug O2/v3 corpus
improvement is established: all 44 Ir counts and all 15 worst-15 executables
are identical to merged main.

## Code size and compiler-pass cost

All **57 programs**, compiled to objects at O2 and Os (v3), measured by
`objcopy --only-section=.text -O binary`, not file size:

| Mode | Baseline .text sum | Candidate .text sum | Regression cases |
|---|---:|---:|---:|
| O2 | 41,703 B | 41,703 B | 0 |
| Os | 31,607 B | 31,599 B | 0 |

The Os reduction is 8 bytes in chacha20_block; no program grows.
`CCC_TIME_PASSES=1`, 57 O2 compilations: candidate fp_double totals 0.106 ms,
median 0.001 ms, maximum 0.008 ms, three rewrites. Loop-square totals 0.2 ms
at its coarser 0.1-ms reporting resolution, one rewrite. These are diagnostic
single-run pass timings, not a statistically established compile-time speedup;
zero rounded values do not prove zero work. Raw compile wall times are retained.

## Debug-build measurements

Shared Debian 13 KVM Xeon (family 6/model 106), two vCPUs, about 2 GiB RAM and
8 GiB active swap. Native work pinned to CPU 0: default workload sizes, O2/g/v3/
fwrapv, eleven alternating-order pairs after an output/warmup execution.
Intervals are seeded 10,000-resample paired bootstrap intervals, unadjusted
for multiple comparisons. Native time includes process startup. Callgrind
uses reduced sizes (base64's tiny fixed input has no size override).

| Debug workload | Median time change | Paired candidate/base [95% interval] | Ir change |
|---|---:|---|---:|
| base64_enc | -2.94% | 0.9672 [0.9499, 0.9869] | -0.0631% |
| bitops | -1.04% | 1.0086 [0.9830, 1.0353] | +0.0000% |
| glibc_memcmp | -4.12% | 0.9876 [0.9512, 1.0245] | +0.0000% |
| libm_round_family | -2.37% | 0.9739 [0.9493, 0.9960] | -0.0023% |
| nbody | +1.49% | 0.9940 [0.9746, 1.0167] | -1.4633% |
| sqlite_varint | -6.89% | 0.9298 [0.9121, 0.9486] | -0.2189% |
| strlen_bench | -3.83% | 0.9789 [0.9501, 1.0100] | -1.0998% |

SQLite's debug-build reduction is the clearest timing result here. Base64 is
startup-dominated; its small interval must not be sold as kernel acceleration.
Nbody's median is worse even though its Ir improves, and its interval includes
1. Several other intervals also include 1. The byte-identical struct-copy
control has a noisy median change and an interval including 1. No universal
runtime or vendor superiority is established.

## Four-vendor distillation and stronger tests

`tests/regression/fp_recurrence/oracle.c` has a strict checksum of
`26eeaeca4508465b`: local base/candidate, GCC 16.2, Clang 23.1.0, ICC 2021.10 and
ICX agree under matched strict controls. ICX's provider latest resolves to
2024.0.0, not a verified newest Intel release. Initial ICX default-FP-model
results differed even with `-ffp-contract=off`; explicit `-fp-model=strict
-ffp-contract=off -fno-fast-math` restores agreement. Fast-contraction/model
checksums also differ across vendors and are not treated as a correctness vote.
The raw unsuccessful/mismatched controls are retained, not overwritten.

For each of twice_plus, twice_minus and twice_plus32, the candidate strict
sequence uses two arithmetic instructions plus return, with no multiply or
constant memory operand. GCC/Clang/ICC make the same reduction; controlled
strict ICX retains a multiply by a memory constant. This is codegen evidence,
not vendor native timing. Reusing Godbolt's existing script avoids a new client.

The FP regression now prints canonical-byte-order FNV-1a over every compared
result and exception set, including new add/sub consumers in both widths.
Host GCC O0 and eight LCCC O0/O1/O2/O3 × generic/v3 runs agree on
`e778c29691d2c901`; internal bit/flag assertions remain intact. The strict oracle
also runs in all eight configurations. Each IR pass is disabled separately;
Mandelbrot's innermost FP loop and whole main each lose exactly one multiply/
FMA with square reuse, under default fast contraction, with identical output.
Tab/comment cases, semantic directives, inline asm, address-taken/table/
fall-through labels and contraction-disabled ISA cases have explicit tests.

## Reproduction and acceptance

- `python3 scripts/fp_recurrence_evidence.py --baseline BASE_CC --candidate NEW_CC --out RESULTS`
  runs four full bounded Callgrind arms plus size/pass-time census; active swap
  required. JSON is marked incomplete until every stage succeeds.
- `CCC=NEW_CC python3 tests/regression/check_debug_code_parity.py` checks all
  57 O2 debug/plain .text pairs and all 57 Os default/explicit-no-thread pairs.
- `CCC=NEW_CC bash tests/regression/check_fp_recurrence.sh` enforces exact FP,
  contraction, independent pass controls and Mandelbrot loop contracts.
- `python3 scripts/check_peephole_directive_step.py` includes its witnesses.
- `scripts/ci_local.sh --fast` is the required local gate, including rustfmt
  and Clippy. Full/slow local CI is deliberately not run; hosted final-head
  checks are separate. A score, old CI result or reviewer recommendation is
  not a replacement for exact-tree validation.
