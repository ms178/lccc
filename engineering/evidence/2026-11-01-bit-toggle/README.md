# Bit toggle — measurements

Evidence for the `BitToggle` half of §12 in
[`engineering/llvm-missed/SCALAR-PREDICATE-FOLDS.md`](../../llvm-missed/SCALAR-PREDICATE-FOLDS.md):
`x ^ (1u << n)` becomes the fourth single-bit IR operation, which x86 and i686
lower to `btc` / `btcl` and AArch64/RISC-V lower to `eor`/`xor` against a shifted
one. The set/clear/test shapes shipped in the previous round
([`../2026-11-01-bit-selection/`](../2026-11-01-bit-selection/)); this directory
is the toggle's own probe, and it shares that directory's `count.py` rather than
re-implementing "instruction".

The `before` column is the `CCC_NO_BIT_SELECT=1` build, which is exactly the
pre-fold spelling of all four shapes — the fold is the only thing that knob
turns off, and the probe's counts were measured against the pre-patch compiler
before it was written (6 / 3 / 3 for `btog32` / `gcc` / `clang`).

| File | What it holds |
| --- | --- |
| `bit-toggle.c` | 8 toggle shapes, 3 controls that must not fold |
| `names.txt` | the probe function names the counter counts |
| `toggle-counts.txt` | the four columns of the table below |
| `toggle-before.s`, `toggle-after.s`, `toggle-gcc.s`, `toggle-clang.s` | the dumps those columns came from (`-O2`, gcc 16.2.0, clang 23.1.4) |
| `cross-target.sh` | rebuilds the fold-on/fold-off counts on all four targets |
| `cross-target-counts.txt` | its output, checked in |
| `sweep-x86_64.json`, `sweep-i686.json`, `sweep-aarch64.json` | the corpus A/B, run with `../2026-11-01-bit-selection/sweep.py` |

## x86-64, one column per compiler (`-O2`)

| function | before | after | gcc | clang |
| --- | --- | --- | --- | --- |
| `btog32` `x ^ (1u << n)` | 6 | **3** | 3 | 3 |
| `btog64` | 6 | **3** | 3 | 3 |
| `btog_masked` (`n & 31`) | 6 | **4** | 3 | 3 |
| `btog_imm` (`1u << 5`) | 3 | 3 | 3 | 3 |
| `btog_mem` (memory base) | 5 | **4** | 5 | 3 |
| `btog_twice` `x ^ m ^ m` | 2 | 2 | 2 | 2 |
| `btog_swapped` `(1u << n) ^ x` | 5 | **3** | 3 | 3 |
| `btog_offset` `x ^ (1u << (n + 1))` | 7 | **4** | 4 | 4 |
| `keep_xor_two` (`3u << n`) | 6 | 6 | 5 | 5 |
| `keep_xor_not` (`~(1u << n)`) | 8 | 8 | 4 | 4 |
| `keep_xor_hi` (64-bit shift, 32-bit XOR) | 6 | 6 | 5 | 5 |

Three rows carry the thinking:

* **`btog_twice` is a *no-regression* row, not a win.** The toggle is its own
  inverse, and the pre-fold compiler cancelled `m ^ m` outright (2
  instructions). Folding both XORs is arithmetically right and still a
  pessimisation — it measured 5 — so the matcher answers
  `BitToggle(BitToggle(x, n), n)` with a **copy of `x`** and DCE finishes it.
  The gate pins 2 in both builds and refuses a `btc` in that function.
* **`keep_xor_hi` must not fold.** `unsigned long` is 64 bits on x86-64, so the
  XOR is 32-bit and the shift is 64-bit: the low 32 bits of `1ul << n` are zero
  for `n >= 32`, while `index mod 32` would wrap to bit 0, 8, … Folding it is a
  miscompile, and the matcher refuses a shift whose width is not the XOR's.
* **`btog_offset` shows the index is just an operand.** `n + 1` folds like `n`
  does; the shift is what has to be single-use, not the parameter.

## Cross-target (`cross-target-counts.txt`)

Fold off → on, same corpus, `-O2`. No function on any target got longer; the
`REGRESSION` column is empty.

| target | moved | largest win | unchanged |
| --- | --- | --- | --- |
| x86-64 | 5 of 8 | `btog32`/`btog64` 6 → 3 | `btog_imm`, `btog_twice` |
| i686 | 6 of 8 | `btog32` 14 → 11, `btog_mem` 15 → 12 | `btog_imm`, `btog_twice`, `keep_xor_hi` (which folds here: `unsigned long` is 32 bits) |
| AArch64 | 0 | — | every row: `lsl`+`eor` already costs what `lsl`+`orr` does, so the recogniser's win is the eliminated middle instructions only where the shift itself was shared |
| RISC-V | 6 of 8 | `btog_mem` 15 → 13, `btog_swapped` 14 → 12 | `btog_imm`, `btog_twice` |

AArch64 being flat is expected and is the reason the cross-target check is a
*length* check rather than a win check: the toggle is one instruction there in
both builds, so the only thing the fold can do wrong is pessimise, and it does
not.

There is no qemu and no 32-bit libc in the development sandbox, so i686, AArch64
and RISC-V are covered by instruction counts, `check_bit_select_fold.sh`'s
never-longer rule, and the recogniser's own tests. x86-64 is executed.

## Corpus A/B

Every `.c` file under `tests/` and `engineering/`, compiled twice by the same
binary with and without `CCC_NO_BIT_SELECT=1` (`sweep.py` in the sibling
directory). `verdict` — a file that compiles in one build and not the other —
must stay zero; `longer` must stay zero.

| target | files | shorter | same | longer | both reject | verdict | net |
| --- | --- | --- | --- | --- | --- | --- | --- |
| x86-64 | 3,079 | 6 | 2,298 | **0** | 775 | **0** | −68 |
| i686 | 3,079 | 2 | 1,269 | **0** | 1,808 | **0** | −57 |
| AArch64 | 3,079 | 1 | 1,225 | **0** | 1,853 | **0** | −11 |

The file count is one higher than the previous round's because this directory
adds `bit-toggle.c` to `engineering/`. The six x86-64 wins are
`tests/regression/simd_avx2_defer_chain.c` (178 → 164),
`tests/benchmark/programs/glibc_memcmp.c` (158 → 156),
`tests/benchmark/programs/linux_find_bit.c` (123 → 122), `bit-toggle.c` and the
two probes in the sibling directory — the three real files are the reason the
sweep is worth running: they spell the idiom in code nobody wrote for this
compiler.

## Reproduce

```sh
source ~/.cache/lccc/agent-toolchains/env.sh   # gcc 16.2.0, clang 23.1.4
./scripts/build_lccc_fast.sh                   # target/fastbuild/lccc (+ per-target)

bash tests/regression/check_bit_select_fold.sh  # the gate, including this shape

C=target/fastbuild/lccc
CCC_NO_BIT_SELECT=1 $C -S -O2 bit-toggle.c -o toggle-before.s
$C -S -O2 bit-toggle.c -o toggle-after.s
gcc-16 -S -O2 bit-toggle.c -o toggle-gcc.s
clang-23 -S -O2 bit-toggle.c -o toggle-clang.s
python3 ../2026-11-01-bit-selection/count.py toggle-before.s toggle-after.s \
    toggle-gcc.s toggle-clang.s $(cat names.txt)
./cross-target.sh
```
