# Single-bit selection — measurements

Evidence for §12 of [`engineering/llvm-missed/SCALAR-PREDICATE-FOLDS.md`](../../llvm-missed/SCALAR-PREDICATE-FOLDS.md):
`x | (1u << n)`, `x & ~(1u << n)` and `(x & (1u << n)) != 0` become the new
`BitSet` / `BitClear` IR operations (`BitTest` already existed), which x86 and
i686 lower to `bts` / `btr` / `bt`.

Everything here is reproducible from this directory; the reproduced numbers are
checked in so a reviewer can diff a rerun against them.

| File | What it holds |
| --- | --- |
| `bit-selection.c` | 23 probe functions: 13 selected shapes, 5 controls that must not fold, 5 popcount/parity shapes |
| `bit-selection-exec.c` | executed probe — every function at every width, compared against hand-written expectations; prints one checksum |
| `names.txt` | the probe function names `count.py` counts |
| `count.py` | per-function instruction counts from a GAS dump (framing included for all compilers) |
| `x86-64-counts.txt` | the four columns of the table below |
| `lccc-before.s`, `lccc-after.s`, `gcc.s`, `clang.s` | the dumps those columns came from (gcc 16.2.0, clang 23.1.4, `-O2`) |
| `mpopcnt-counts.txt` | the same corpus with `-mpopcnt`, where `lccc` matches its `popcnt` path |
| `lccc-mpopcnt.s`, `gcc-mpopcnt.s`, `clang-mpopcnt.s` | those dumps |
| `cross-target.sh` | rebuilds the fold-on/fold-off counts on all four targets (`CCC_NO_BIT_SELECT=1` is the off switch) |
| `cross-target-counts.txt` | its output, checked in |
| `sweep.py` | corpus A/B: every `.c` file compiled twice, instruction lines counted |
| `sweep-x86_64.json`, `sweep-x86_64-artifacts.json`, `sweep-i686.json`, `sweep-aarch64.json` | its results |

## x86-64, one column per compiler (`-O2`)

`before` is the pre-patch compiler, `after` the patched one; both use this
corpus, and the two right-hand columns are GCC 16.2.0 and Clang 23.1.4.

| function | before | after | gcc | clang |
| --- | --- | --- | --- | --- |
| `bset32` | 6 | **3** | 3 | 3 |
| `bset64` | 6 | **3** | 3 | 3 |
| `bset_imm` | 3 | 3 | 3 | 3 |
| `bset_masked` | 6 | **4** | 3 | 3 |
| `bclr32` | 8 | **3** | 3 | 3 |
| `bclr64` | 8 | **3** | 3 | 3 |
| `bclr_imm` | 3 | 3 | 3 | 3 |
| `btest32` | 7 | **4** | 4 | 4 |
| `btest32z` | 7 | 7 | 4 | 4 |
| `btest64` | 7 | **4** | 5 | 4 |
| `btest_imm` | 4 | 4 | 4 | 4 |
| `bset_mem` | 6 | **4** | 5 | 5 |
| `bclr_mem` | 7 | **4** | 5 | 5 |
| `pop32` | 17 | 17 | 5 | 16 |
| `pop64` | 21 | 21 | 4 | 20 |
| `parity32` | 18 | 18 | 7 | 7 |
| `parity64` | 22 | 22 | 10 | 10 |
| `pop_and1` | 18 | 18 | 7 | 7 |
| `keep_shl` | 5 | 5 | 5 | 5 |
| `keep_two` | 6 | 6 | 5 | 5 |
| `keep_xor` | 6 | 6 | 3 | 3 |
| `keep_eq4` | 8 | 8 | 8 | 8 |
| `keep_and` | 6 | 6 | 5 | 5 |

The five `keep_*` rows are the controls: `x << n` on its own, a two-shift
selection, a shift that also feeds a live `xor`, a masked compare against a
four-bit constant and a plain `x & 1u << n` whose value is returned raw. None of
them may move, and none did.

`btest32z` is `(x & 1u << n) == 0`, the one selected shape that is deliberately
left alone (see §12): the `bts`-style instruction produces a 0/1 bit, and the
`== 0` would need a second instruction on top of it.

## `-mpopcnt` (`mpopcnt-counts.txt`)

| function | lccc | gcc | clang |
| --- | --- | --- | --- |
| `pop32` | 3 | 3 | 2 |
| `pop64` | 3 | 3 | 2 |
| `parity32` | 4 | 4 | 3 |
| `parity64` | 4 | 4 | 3 |
| `pop_and1` | 4 | 4 | 3 |

Without the feature bit LCCC stays with the 17-instruction SWAR sequence where
GCC emits a call to `__popcountdi2` (5 instructions plus a libgcc dependency on
every object). That is a linking decision, not an instruction-selection one, and
§12's "what is not done" explains why it is not taken here.

## Cross-target (`cross-target-counts.txt`)

Fold off → fold on, same corpus, `-O2`. No function on any target got longer;
the printed `REGRESSION` column is empty.

| target | shapes that moved | largest win |
| --- | --- | --- |
| x86-64 | all 13 | `bclr32`/`bclr64` 8 → 3 |
| i686 | all 13 | `bclr32`/`bclr64` 16 → 11, `bclr_mem` 18 → 13 |
| aarch64 | 6 (`bclr32` 15 → 12, `btest64` 13 → 11, `bclr_mem` 18 → 15, …) | `bclr32` |
| riscv64 | 10 (`btest32` 19 → 11, `btest64` 15 → 7, `bclr_mem` 21 → 15, …) | `btest64` |

AArch64 wins less because `x | 1u << n` is already two instructions there; what
it gains is the `BitClear`/`BitTest` shapes, where the recogniser removes the
temporary and the widened copy of the index. Its first lowering read the base
into a scratch register before `bic`, which cost one instruction on the
shift-shared shapes (12 → 13); the shipped form reads the base in place from its
home register and the corpus is at or below parity everywhere.

There is no qemu and no 32-bit libc in the development sandbox, so i686,
AArch64 and RISC-V are covered by instruction counts plus the recogniser's own
tests, and by `check_bit_select_fold.sh` failing if any function gets longer.
x86-64 is executed: `bit-selection-exec.c` prints `cc281c3c` under every one of
`-O0 … -Os`, plus GCC and Clang.

## Corpus-wide sweep (`sweep.py`)

Every `.c` file under `tests/` and `engineering/` (3,078 files), compiled twice
by the same binary with and without `CCC_NO_BIT_SELECT=1`, instruction lines
counted. A file whose two builds disagree on success is a `verdict` change and
is the invariant this sweep exists to keep at zero.

| target | shorter | same | longer | both reject | verdict | net instructions |
| --- | --- | --- | --- | --- | --- | --- |
| x86-64 | 3 | 2,300 | **0** | 775 | **0** | −47 |
| i686 | 1 | 1,269 | **0** | 1,808 | **0** | −33 |
| aarch64 | 1 | 1,224 | **0** | 1,853 | **0** | −11 |

The x86-64 run was also given `artifacts/` (98 more files, x86-only by
construction): 0 shorter, 94 same, 0 longer, 4 both-reject, net 0 —
`sweep-x86_64-artifacts.json`, which is why `sweep-x86_64.json` lists 3,176.

`both reject` is corpus noise, not a measurement: those files need `-I` paths,
`-m32`, a target-specific intrinsic or a header the harness does not pass, and
they fail identically in both builds. The clang-c import dominates the bucket
(parse and semantic errors on features LCCC does not implement yet). Compiling
the whole corpus a third time to classify each one would not change a single
row of the table above.

The three x86-64 `shorter` files are `tests/regression/simd_avx2_defer_chain.c`
(178 → 164 — a real corpus file that uses the `bts` shape in a bit-flag loop),
and the two probes in this directory. Nothing else in the corpus contains a
single-bit selection the frontend lowers in this form, which is the honest size
of the win: this is a parity fix for code that already asks for it, not a
transformation that fires everywhere.

## Reproduce

```sh
source ~/.cache/lccc/agent-toolchains/env.sh   # gcc 16.2.0, clang 23.1.4
./scripts/build_lccc_fast.sh                   # target/fastbuild/lccc (+ per-target variants)

# the executed probe and the gate
bash tests/regression/check_bit_select_fold.sh

# the tables
python3 count.py lccc-before.s lccc-after.s gcc.s clang.s $(cat names.txt)
./cross-target.sh

# the sweeps (per-target binaries share the frontend and the passes)
python3 sweep.py --bin ../../../target/fastbuild/lccc        --roots ../../../tests ../../../engineering --jobs 2 --level=-O2 --json sweep-x86_64.json
python3 sweep.py --bin ../../../target/fastbuild/lccc-i686   --roots ../../../tests ../../../engineering --jobs 1 --level=-O2 --json sweep-i686.json
python3 sweep.py --bin ../../../target/fastbuild/lccc-arm    --roots ../../../tests ../../../engineering --jobs 2 --level=-O2 --json sweep-aarch64.json
```

`--level` needs the `=` form: `--level -O2` is parsed as another flag.
