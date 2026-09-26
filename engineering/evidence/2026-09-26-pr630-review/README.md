# Post-merge PR #630 review: decisions and reproducible evidence

**Source baseline:** upstream `ms178/lccc` `main` at
`a066f0c9b835ff36445a06bd762051e5a5039043` (includes reviewed
`737e98a`). **Tested follow-up code commit:**
`da01098fe423dd023270cf256d9e527fea000ada`, whose sole parent is that
baseline. The evidence/documentation commit comes after it; it does not change
compiler code. These are local measurements, not claims that hosted CI has run
on the follow-up branch. `verification-summary.json` records versions, input
hashes, scope and test counts. All input sources and the comparison runner are
tracked here; the local machine's logs and binary paths are **not** the only
evidence.

## Review decisions (claims checked, not taken on authority)

1. **i686 `%r8d`/`%r8` broadcast acceptance: accept and fix.** On the
   unmodified merged main, GAS 2.47 rejected `vpbroadcastd %r8d,%xmm1` and
   `vpbroadcastq %r8,%xmm1`, while LCCC produced `62 d2 7d 08 7c c8` and
   `62 d2 fd 08 7c c8`, respectively. Byte, word, data-register, base,
   index, and high-vector-register variants exposed the same missing
   mode-boundary validation. The guard now checks all operands before either
   local or delegated encoding, including `.code16`, `.code16gcc`, and
   `.code32`. It **does not** reject the legal low EVEX `xmm`/`ymm`/`zmm`
   and `k` registers in 32-bit mode: the accepted/rejected corpus checks both
   sides. Qualification: a blanket `*8`-register ban would be wrong. GNU as
   accepts `mov %cr8,%eax` in i686 mode, emitting `f0 0f 20 c0`; LCCC already
   lacked that special LOCK-prefixed form. The guard deliberately does not
   classify `%cr8` as unavailable. That older unsupported operation is not
   represented as a new regression fix.

2. **Missing hosted x86-64 differential and path-only parity: accept and
   fix.** Baseline hosted CI executed an i686 asm-diff invocation, not the
   PR #629 **x86-64 90-case** corpus. The previously used parity checker
   recognized a shared script path, not the architecture, compiler, or input
   corpus. The hosted workflow now provisions GNU as 2.47 and runs exactly
   that x86-64 corpus. The local/hosted parity contract checks direct shell
   invocations, `--32`, `--lccc`, `--as` (for hosted x86-64), `--jobs`, and the
   exact casefile set. Five mutation self-tests remove each gate, substitute
   the wrong mode/compiler/corpus/oracle/jobs, turn a command into `echo`, and
   remove the installer; all must be rejected. **Limit:** this static check
   cannot prove GitHub ran the workflow or guarantee a remote installer will
   be online. Local CI ran both gates and the mutation tests; no hosted-run
   status is claimed.

3. **Dead GPR-source VEX broadcasts: qualified acceptance.** The i686 and
   x86-64 *VEX* helpers that accepted impossible GPR broadcasts were dead or
   unreachable after the merged EVEX dispatch. Removed those dispatch arms
   and helpers. **Do not** delete the x86-64 *EVEX* GPR helper: GAS accepts
   `vpbroadcastd %r8d,%xmm1` in x86-64 and `vpbroadcastd %eax,%zmm1` in
   i686, and the live path emits the matching bytes. Removing it would have
   turned valid instructions into errors, rather than fixing the review issue.

4. **Nontransactional encoder state: accept and broaden.** A rejected
   instruction could have already appended bytes/relocations (and, in the
   x86-64 post-encode decorator path, advanced the offset). Both public
   `InstructionEncoder::encode` methods now restore bytes, new relocations,
   location counter and per-instruction state on any error. Direct unit tests
   encode valid–invalid–valid with a rejected symbol-bearing broadcast and
   compare the final bytes, offsets, relocation ownership and types against
   an encoder that never saw the invalid instruction. x86-64 decorator checks
   now read **only the current instruction's slice**, and i686 validates
   directly on its slice without copying it to a temporary encoder. A
   separate VEX-then-EVEX unit test prevents an earlier instruction's prefix
   from misclassifying the later mask. The delegated-vector memory bridge
   additionally re-plans i686 16-/32-bit addresses, absolute symbols and
   EVEX disp8*N using metadata recorded by the real shared emitter (not a
   byte-pattern heuristic). As one independent pre-fix example,
   `nop; vmovdqu8 sym(%eax),%zmm1` had identical `.text` bytes but LCCC's
   `R_386_32` relocation was at **8** versus GAS's **7**; the post-fix
   whole-object comparison agrees. `.code16` symbol-bearing vector memory
   now uses `R_386_16` with the correct field offset. This atomicity claim
   is for the instruction encoder API, not every assembly directive or
   unrelated compiler subsystem.

5. **Author-local evidence paths: accept and fix.** The merged review's
   `~/...` logs cannot be independently replayed. We track two input
   manifests, a portable runner and **both** baseline and candidate results
   (`baseline-review-probes.json`, `followup-review-probes.json`,
   `followup-isolated-probes.json`). They identify the compiler binaries by
   SHA-256, record the repository HEAD at measurement time, GNU as versions
   and hashes, acceptance, `.text` bytes and ELF relocations. Historical
   inputs are exactly **88 final + 143 recorded broadcast-width + 48 EVEX
   inventory = 279**. The surviving broadcast-width log contains 143 inputs;
   calling it a portable 180-case set would invent 37 missing inputs. We
   compare acceptance/bytes/relocations, **not exact wording of errors**;
   same-rejection/different-message cases are not byte regressions. Full
   whole-object corpora and the deterministic matrix are checked into the
   repository too.

## Measured results and negative space

| Check | Merged main (pre-fix) | Follow-up code commit |
| --- | ---: | ---: |
| Independent review probes, accepted/rejected + bytes + relocations | 8/23 match (10 acceptance and 5 byte/reloc mismatches) | **23/23 match** |
| Historical isolated PR #629 inputs | Not rerun against the old binary in this record | **279/279 match** (GAS and LCCC each accept 98) |
| All pinned GNU as 2.47 whole-object x86-64 cases | — | **1045/1045**, zero failures |
| All pinned GNU as 2.47 whole-object i686 cases | — | **549/549**, zero failures |
| Generated i686 boundary matrix on system GAS 2.44 | — | **393/393**, zero failures; **subset** of the 549, not additional |
| `scripts/ci_local.sh --fast` (including rustfmt + Clippy) | — | **76 gates pass, 0 fail, 3 skip**; 3513 unit tests pass, 7 ignored |
| Target i7-14700KF timing | Not available | **Not available; no performance claim** |

The three skipped fast gates are `regression-corpus-ssa`,
`benchmark-output-oracle` and `peephole-whitespace-invariance`, by `--fast`
design. Some i686 *runtime* legs also skip locally because this 2-vCPU Xeon VM
has no i386 libc headers/loader; the i686 **assembler** differential ran.
The GNU as 2.47 source tarball used here hashes to
`154ab23b60070e8f27013c22977f1129425d67d1e8acd6e13010e617811e4cff`.
We used 8 GiB of active swap to avoid OOM. The 90-case hosted x86-64 gate is
inside, not in addition to, the full 1045-case local run. Direct Rust focused
encoder regressions: **10 pass**. The five CI-gate mutation tests: **5 pass**.

No byte-compare, Mandelbrot, `linux_find_bit`, or allocator change was shipped.
We rechecked the open P0 entries in `backlog.md` and the four-compiler oracle
selection in `scripts/godbolt.py` (GCC, Clang 23.1, ICC, ICX). The available
host is not the specified i7-14700KF, so using its timings to claim a target
performance improvement would violate the paired same-host protocol. In
particular, no unproved byte-compare widening near page/object boundaries,
FP-loop transformation, speculative branch deletion, or broad allocator
rewrite was introduced. Their previous negative results and remaining
verifiers remain in the tracked backlog/evidence; no current-main P0
A/B result is claimed here.

## Replay without author-local paths

From the repository root with the repository's Rust toolchain and 8 GiB swap
available (to avoid OOM):

```sh
bash scripts/ensure_gas_247.sh x86_64-linux-gnu
bash scripts/ensure_gas_247.sh i686-linux-gnu
scripts/build_lccc_fast.sh
python3 scripts/check_ci_gate_parity.py
python3 scripts/test_ci_gate_parity.py
python3 scripts/asmdiff.py --jobs 2 \
  --as "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as" \
  --lccc target/fastbuild/lccc-x86
python3 scripts/asmdiff.py --32 --jobs 2 \
  --as "$HOME/.cache/gas-2.47-i686-linux-gnu/bin/as" \
  --lccc target/fastbuild/lccc-i686
CI_LOCAL_JOBS=1 bash scripts/ci_local.sh --fast
```

The matrix's input list is reproducible from
`engineering/evidence/2026-09-26-pr630-review/generate_mode_matrix.py` (run
from the repository root); its checked-in casefile SHA-256 is in
`verification-summary.json`. To replay both isolated manifests, avoiding
changes to the tracked result files:

```sh
E=engineering/evidence/2026-09-26-pr630-review
GAS64="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"
GAS32="$HOME/.cache/gas-2.47-i686-linux-gnu/bin/as"
python3 "$E/verify_isolated_probes.py" --as-x64 "$GAS64" \
  --as-i686 "$GAS32" --jobs 2 --output /tmp/followup-pr629-probes.json
python3 "$E/verify_isolated_probes.py" --as-x64 "$GAS64" \
  --as-i686 "$GAS32" --jobs 2 \
  --inputs "$E/pr630-review-probe-inputs.json" \
  --output /tmp/followup-review-probes.json
```

To reproduce the *pre-fix* 15 differences, build the baseline in a separate
worktree, keeping any working-tree modifications intact. The deliberate
`--allow-mismatches` flag is only for a negative baseline run; without it the
runner fails on any disagreement:

```sh
git worktree add ../lccc-pr630-base a066f0c9b835ff36445a06bd762051e5a5039043
(cd ../lccc-pr630-base && scripts/build_lccc_fast.sh)
python3 "$E/verify_isolated_probes.py" --as-x64 "$GAS64" \
  --as-i686 "$GAS32" --jobs 2 \
  --lccc-x64 ../lccc-pr630-base/target/fastbuild/lccc-x86 \
  --lccc-i686 ../lccc-pr630-base/target/fastbuild/lccc-i686 \
  --inputs "$E/pr630-review-probe-inputs.json" \
  --output /tmp/baseline-review-probes.json --allow-mismatches
```

`--allow-mismatches` **never** makes an unsupported GNU as version pass: the
runner requires the 2.47 oracle. The archived binaries' SHA-256 may not be
identical when building at another path; compare the source revision and the
individual acceptance/byte/relocation results, not their host-specific binary
hashes. No x86-64 and i686 corpus is allowed to stand in for the other.
