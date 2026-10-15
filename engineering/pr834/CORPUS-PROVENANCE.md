# Retained PR834 follow-up corpus evidence

## Exact capture and scope

The measured candidate is **b52a114434d446db5979bccf8d759aae619b6f2e**,
tree **b3146dd4f7836e01a4ae7c433a46389d22c64682**, not the subsequent
documentation/Python commit. Baseline: merged main
**f07f4c514460e4116a574773cd84b69d185bf65e**. The frozen compiler hashes are:

- candidate: `de11b050716e2d36d2ef223cfb4e172c116ab3d4a0bb7a4eb38e735967fa6576`
- baseline: `bc77239efb20d1115ac2092145af6b496e6fce7100a1d6127f222f969b454f7a`

`engineering/evidence/pr834-corpus.json` binds the runtime archive, inventory,
exact capture-controller bytes, 45 source/flag combinations and compiler build
inputs. Before/after source, compiler, controller, tool and header identities
are inside the archive and are equal. The controller completed with zero
failures. `capture-controller.py` is the historical controller verbatim; its
background-load note describes this capture, not an automatically detected
condition on a different machine.

The self-contained in-repository runtime archive retains **14,639 files**, with
backward hardlinks for byte-identical content: all 225 bounded program pairs,
450 program Callgrind profiles, compiled program binaries, 2,250 default-size
compiler invocations, process streams, sidecars and summaries. Its decoded
logical size is 92,715,833 bytes. It does not contain the compiler executables
or the additional compiler-parent profiles; those are separate delivery
attachments, not prerequisites for verifying the program `[1,1]` statement.

## Verification and fresh reproduction

```
python3 scripts/verify_corpus_receipts.py
python3 scripts/verify_corpus_receipts.py --check-current-inputs
```

The second command additionally checks the current compiler build inputs;
it intentionally fails after a later compiler change. CI verifies historical
evidence without incorrectly demanding that future source remain unchanged.
The verifier streams archive members in 1 MiB chunks, bounds metadata and
logical decoded scope, refuses unsafe paths/symlinks/forward hardlinks, and
checks completeness, raw-output/sidecar links, executable hashes, source,
flags, geometry, counters and derived metrics. A real-parent RED demonstrates
the missing reproducibility feature; eight tests cover the real capture and
post-decoding mutations (metric lie, uncovered path, deleted sidecar plus
inventory row, compiler identity and geometry), plus digest/schema rejection.
Hash consistency is not authenticity or proof that a fresh run happened.

For fresh execution, build baseline and candidate independently with
`./scripts/build_lccc_fast.sh` (fastbuild, opt-level 1, `-j 2`). Keep source and
binaries immutable. Run `scripts/callgrind_corpus.py` with the retained common
flags, per-form bounds, GCC builtin-header identity and cache geometry; alternate
`--mine` and `--ref` for five rounds and normalize even-round ratios. The exact
original argv and include path are retained in every manifest and receipt.
For full capture reproduction, the historical controller accepts `--repo`,
`--baseline-repo`, `--candidate`, `--baseline`, `--out`, `--compile-batch 5`.
Use distinct equal-length output paths, pin the controller to CPU1, and record
actual background load rather than adopting the historical note blindly.

## Results and limits

- Every bounded candidate/base program-Ir ratio in all five rounds is **1**.
  Outputs agree natively and under instrumentation. This is not a native-speed
  result and does not resolve the standing worst-15/fannkuch performance debt.
- **44/45** program ELFs are identical in every round. `loop_patterns` differs:
  two `vpxor`, three `vmovdqa`, one `vpshufd` and one `vpaddq` promotion are
  conservatively withheld before reachable calls; padding adjusts accordingly.
  See `loop-patterns-promotion-diff.txt`, generated with the recorded objdump.
  The 48-copy budget and existing promotion-count assertions are unchanged.
- All 45 candidate program ELFs match the earlier F1/F6 candidate capture;
  the F2 refresh-cost change introduced no additional program-byte changes in
  this corpus. This is a bounded corpus observation, not a universal proof.
- Default-size compiler walltime: five paired batches of five invocations per
  arm/form. Geometric mean of median ratios: **0.9996120480112506**. Seven
  medians in this initial capture exceed 1.05:
  arith_loop, byte_load_reassembly, constant_recursion, expat_siphash24, fib,
  sha256_transform, zstd_count. No threshold is waived or installed as a gate.
- Additional default-size **compiler-parent** Callgrind: all 45 forms, Ir geo
  **1.0008280867352997**, range **[0.9977620997240416,1.0063138232679372]**.
  Exec children are excluded; native compiler walltime includes assembler and
  linker work. These distinct scopes must not be conflated. Small simulated-Ir
  changes do not exonerate the seven native-walltime regressions.
- Hardware: shared Xeon VM, two vCPUs. The main controller used CPU1 while the
  long baseline stress Callgrind used CPU0; not isolated hardware, not 14700KF.
- GCC16's default absolute assembler/linker paths bypass a PATH-only pin.
  The primary LCCC comparison uses the built-in assembler/linker. Future GCC
  reference runs must explicitly select the 2.47 provider using COMPILER_PATH
  or a GCC-only `-B` option and record actual resolution; earlier runs must not
  be retroactively described as having that provider selection.

Immutable FAST/format/strict-Clippy acceptance and hosted follow-up checks are
separate from this capture. The merged PR's 13 green checks do not cover these
follow-ups; the hosted post-change observation cohort is still 0/20.

## Subsequent measurements

[MEASUREMENT-RESULTS.md](MEASUREMENT-RESULTS.md) preserves the later quiet all45
and interleaved six-form diagnostic alongside this initial capture. It does
not overwrite these measurements or turn their simulated counters into speed.
