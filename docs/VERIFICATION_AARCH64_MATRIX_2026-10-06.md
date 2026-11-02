# Verification: the AArch64 matrix, the sysreg table, and the gates — 2026-10-06

This is the evidence behind the counts asserted in
`SESSION_FOLLOWUP_AARCH64_ENCODER_PARITY.md` and the commit that carries this
file.  Every section is a command that was run, with the output it produced, so
a reviewer can re-run or contradict it instead of trusting prose.

## 0. The oracle, and its provenance

The sandbox has no route to `ftp.gnu.org`, no `aarch64-linux-gnu` distribution
packages, and no Rust toolchain download host (`static.rust-lang.org`), so the
pinned oracle was built from the upstream release tag:

| item | value |
|---|---|
| source | `codeload.github.com/gnutools/binutils-gdb/tar.gz/refs/tags/binutils-2_47` |
| source sha256 | `b6903c3438c8b64097b960870cb5edbdbf99baa85393d4efa36900bb74dd780c` |
| configure | `--target=aarch64-linux-gnu --disable-gdb --disable-sim --disable-gprofng --disable-nls --disable-werror --disable-ld --disable-gold` — the provisioner's own flags |
| `as --version` | `GNU assembler (GNU Binutils) 2.47.20260726` |
| installed at | `$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/{as,objcopy,objdump}` |

The dated token is the expected shape, not a different oracle: the 2.47 release
tarball's own build stamps `2.47.20260726`, which is exactly why
`scripts/ensure_gas_247.sh` accepts dated snapshot tokens by design.  This is
*not* the provisioner's pinned tarball — that one is unreachable from here — so
the honest statement of what the runs below prove is: **a binutils built from
the `binutils-2_47` release tag with the provisioner's flags agrees with all
10411 recorded verdicts and all 1619 system-register encodings.**  Agreement at
that scale is itself the evidence that the two builds are the same oracle.

## 1. The table half: `--check`

```
$ time python3 scripts/aarch64_operand_legality_matrix.py --check \
    --as "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as" \
    --objcopy "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/objcopy"
operand-legality matrix: 10411 rows agree with GNU as (.../bin/as)
real    0m0.115s
```

Two batched assembler runs.  The per-row implementation it replaced measured
**37.8 s** for the same table on the same box — a 328x reduction, and the
reason the gate is now cheap enough to run on every pass rather than sampled.

## 2. Batched verdicts ≡ per-row verdicts

The batching is only sound if it reproduces the per-row verdicts exactly, so it
was tested against the per-row path on the whole table, three ways:

* **Sample comparison.**  A stratified sample (the first 20 rows plus every
  97th row, 127 rows, covering aliases, rejects, sweeps and immediates):
  `0 disagreements`; the per-row cost measured 3.4 ms/row, i.e. 35.3 s
  extrapolated for the full table.
* **Round trip.**  The table was regenerated with the batched path and compared
  byte-for-byte with the file the per-row path produced:
  `cmp` → identical (10411 rows, 6205 accepted, 4206 rejected).
* **`--check` agreement.**  Section 1 re-derives every row through the batched
  path and finds zero drift against the stored per-row verdicts.

## 3. Fail-closed guards

The batch path attributes verdicts by line number, so its failure modes were
exercised deliberately.  Each of these must be an error, never a silently
re-labelled row:

| injected fault | result |
|---|---|
| assembler that fails without reporting a line (`/bin/false`) | `SystemExit`: "failed without reporting a line; cannot attribute verdicts" |
| a row that is not exactly one word (`.quad 1` among `mov`s) | `SystemExit`: "assembled 2 instructions but .text holds 12 bytes; a row did not encode to exactly one word" |
| duplicate instruction texts | `SystemExit`: "batch_encode needs unique instructions" |
| happy path | four verdicts attributed per row, rejects included: `mov x0,x1 → OK e00301aa`, `ldrb x9,[x10] → REJECT`, `and w0,w1,#0x100000001 → REJECT`, `mov x0,#0x100000001 → OK e00300b2` |

The exit-status/diagnostic agreement checks are the same two the sysreg
sweeper already carried: `as` reporting errors while exiting 0, and `as`
exiting non-zero with nothing to attribute, are both fatal.

## 4. The encoder half: `--jobs`

The differential half stays one encoder invocation per row — whether this
compiler reports a failing line and continues (the property that lets GNU as be
batched) is not pinned by any measurement, and a mis-attributed verdict would be
worse than a slow gate.  `--jobs` buys the wall clock back with parallelism
instead: each row gets its own scratch stem, and `map` returns in row order.

Verified end-to-end with a stand-in compiler (GNU as behind an
`aarch64-linux-gnu-ccc` symlink, which exercises the same symlink, `-c`,
prologue-free plumbing):

```
$ ... --check-lccc <stand-in> --objcopy <pinned> --jobs 1 --max-diffs 0 > j1.txt   # 56.0 s
$ ... --check-lccc <stand-in> --objcopy <pinned> --jobs 4 --max-diffs 0 > j4.txt   # 31.1 s
$ cmp j1.txt j4.txt && echo identical
identical
```

1.8x on the 2-vCPU sandbox, with **byte-identical output** (rows and order), so
the reported result cannot depend on the job count.  Both mirrors and the
registered invocation contract pass `--jobs 2`.

## 5. The sysreg table

```
$ python3 scripts/aarch64_sysreg_table.py --check \
    --def <binutils-2_47>/opcodes/aarch64-sys-regs.def \
    --as "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as" \
    --objcopy "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/objcopy"
aarch64_sysreg_table: 1619 registers agree with GNU as
```

`--def` was used because the release *tarball* is unreachable; the checked-in
`.def` member is byte-identical to the one inside the tag.  The tarball path
itself now verifies the bytes against the provisioner's pin:

```
$ python3 scripts/aarch64_sysreg_table.py --check --tarball <a non-release tarball> ...
aarch64_sysreg_table: <file> is not the pinned binutils-2.47 tarball: sha256
b6903c34..., expected 154ab23b...; refusing to extract it        (exit 2)
```

The pin is read out of `scripts/ensure_gas_247.sh` rather than copied, so there
is one pin in the tree and not two that can drift; an unreadable pin is fatal
rather than a silent unpinned extraction.

## 6. The spelling sweeper

```
$ python3 scripts/aarch64_legality_probe_differ.py --lccc <gas stand-in> \
    --as "$HOME/.cache/gas-2.47-aarch64-linux-gnu/bin/as" --objcopy <pinned>
legality probe: 411 spellings, 0 unpinned disagreement(s), 0 pinned
```

This validates the sweeper's plumbing (generation, both verdict directions,
the pinned-divergence bookkeeping); the real `--lccc` leg is CI's, since it
needs the compiler.

## 7. Table integrity and the ratchet, three ways

| check | result |
|---|---|
| generator keys == table keys, *including order* | True, 10411 rows |
| duplicate `(group, instruction)` keys | 0 |
| malformed expectations (not `REJECT` / `OK <8 hex>`) | 0 |
| `--ratchets` == the table's per-group counts | True, 55 groups |
| `elf_writer.rs`'s `GROUP_RATCHETS` == `--ratchets` | True, and *textually*: the printed block is line-for-line the block the file holds, so the documented paste is a no-op |
| declaration shape | `const GROUP_RATCHETS: &[(&str, usize)] = &[` … `];` asserted exactly, not by substring |

The declaration-shape assertion exists because an earlier revision of this
document's change pasted the `--ratchets` output with a script that anchored on
the first `[` in the line: it ate the type annotation (`&[(&str, usize)] = &[`
became `&[`), and it pasted the generator's `_` placeholder, which is not
expression syntax.  Hosted CI caught it in eleven seconds — the `Check Rust
formatting` step cannot parse the file — and the generator now prints the
`("", N)` form the file actually holds, so the paste is safe by construction
rather than by care.

## 9b. The frozen group roster

Every integrity check in the matrix is a comparison between two things that
shrink together. `--check` compares the table to the generator; the Rust test
compares the generator's per-group counts to the values pasted into
`elf_writer.rs`. Delete a family from `GROUPS`/`SWEEPS`, regenerate, and both
agree perfectly at the new, smaller size -- the table lost rows, the ratchets
lost rows, nothing disagrees. The matrix would rather fail than lose a rule
quietly, so the roster is now written down in `EXPECTED_GROUPS` (55 names, in
generator order) and every mode checks it before doing anything else, including
`--regenerate`, which is the mode that would otherwise write the loss down.

The mutations below are applied to a copy in-tree (so `REPO` resolves), each
expects exit 1, and the copy is deleted afterwards; `git status` is clean
apart from the two intended edits.

| mutation | exit | diagnosis (first line of stderr) |
|---|---|---|
| delete a family (`neon ext`) | 1 | `1 missing (first: 'neon ext'); 1 unexpected …` |
| rename a group (`ldst prfm` → `ldst prefetch`) | 1 | `1 missing (first: 'ldst prfm'); 1 unexpected …` |
| reorder the frozen roster only (generator untouched) | 1 | `same names in a new order (position 36: …)` |
| add a group to `GROUPS` | 1 | `(56 groups, expected 55): 1 unexpected (first: 'zz added family')` |
| delete a family and run `--regenerate` | 1 | same as the deletion above |
| unmodified tree | 0 | -- |

The last row matters because `--regenerate` never used to read the table at
all: that is the one path where a silent shrink becomes permanent. With the
roster check ahead of the mode dispatch, the write cannot happen. The
unmodified tree still reports `10411 rows agree with GNU as` in 0.130 s and
`--ratchets` is still textually identical to the block in `elf_writer.rs`
(56 of 56 lines).

## 10. Rust syntax without a toolchain

No Rust toolchain is reachable from the sandbox (§9), so every `.rs` file in the
tree is parsed with a tree-sitter Rust grammar and required to contain no
`ERROR` or missing node:

```
$ python3 /tmp/rs_syntax.py $(git ls-files '*.rs')
all 498 Rust files parse
```

This is a syntax check, not a type check: it cannot see an unresolved name or a
borrow error, and it is not a substitute for `cargo check`.  It is what turned
the mangled `GROUP_RATCHETS` declaration above from "found by CI eleven seconds
later" into "found before the push" once it existed, and it is cheap enough to
run on the whole tree on every pass.

## 7b. Every other gate that runs without a compiler

The gates hosted CI runs in the same step as the parity checker, plus the
provisioner's own contract suite, all run in this sandbox:

| gate | result |
|---|---|
| `check_ci_workflow_shell.py` (validates the edited `ci.yml`) | ok (2 workflow files) |
| `check_script_imports.py` | ok (134 python helpers) |
| `ensure_gas_247.sh --self-test` (the provisioner's validation matrix) | every case lands on its verdict |
| `python3 -m unittest discover -s tests/fuzz -p "test_*.py"` | 4 tests OK |
| `test_hot_loop_metric.py` (covers the new `tools/oracle/hot_instr.py`) | OK |
| `test_differential_corpus_paths.py`, `test_oracle_delta_gate.py`, `test_godbolt_cache.py`, `test_glibc_check_triage.py` | OK / PASS |

## 8. Gate parity (the check that hosted CI runs)

```
$ python3 scripts/check_ci_gate_parity.py
CI/local standalone gate parity: PASS (139 commands, 17 registered invocation
contracts, ... hosted steps mirrored)
$ python3 scripts/test_ci_gate_parity.py
Ran 66 tests in 10.1s
OK
```

Both were **failing before this change**: the hosted matrix step had been
switched from the provisioned pinned pair to the runner image's
`binutils-aarch64-linux-gnu`, the two new local gates had no hosted step, and
the `ARM codegen assembler-parity` step had been deleted.  Hosted CI agreed:
run `37498533630`, job `112389226068`, **87 of 88 steps green, the single
failure step 74 "AArch64 operand-legality matrix"** — the distro `as` disagrees
with the 2.47-measured expectations, which is why the provisioning is restored
and spelled literally at each invocation.

## 9. Not verified from this sandbox

* **The `--check-lccc` half against the real encoder.** It needs
  `target/fastbuild/lccc`, which needs a Rust toolchain; `static.rust-lang.org`
  and every mirror are unreachable here, so no local build was possible.  CI
  builds and runs it.  What *was* verified is everything around it: the
  plumbing (§4), the tool requirement (it no longer demands an unused `as`),
  and the table half it compares against (§1).
* **`ci_local.sh --fast` end to end.** Same missing toolchain, plus the 32-bit
  oracle packages (`deb.debian.org` is unreachable).  The gates that do not
  need either were run individually and are listed above.
* **rustfmt / clippy.** Not runnable without the toolchain; hosted CI's
  `Clippy` job (run `37498533630`, job `112389226283`) passed on the parent
  commit, and this change touches the Rust sources only in `neon.rs` (an
  `unwrap` replaced by `ok_or_else`) and two comments.
