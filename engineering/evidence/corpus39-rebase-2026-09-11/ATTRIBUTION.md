# Which of the 39-benchmark corpus results are attributable, and which are noise

`perf-ab-mine-vs-base.{json,md}` in this directory is the project's own screening
engine (`scripts/perf_ab.py`) comparing the rebased build (A) against pristine
upstream `d03ca818` (B) over `tests/benchmark/programs`, `-O2`, 11 interleaved
AB/BA rounds, primary metric = minimum, startup-floor benchmarks excluded.

Its aggregate verdict is **"NO MEASURABLE DIFFERENCE (+0.19% geomean over 34
benchmarks)"**, and it lists A as faster on five benchmarks and slower on five.
Read naively that is a wash with a couple of regressions. It is not: the harness
compares *timings*, and for a change with a 10-translation-unit blast radius most
of those timings compare **byte-identical machine code**.

## The noise floor, measured from the harness's own output

`hash_table` reports **B/A = 1.047 (+4.7 %)** — the largest "win" in the table —
and its assembly is **byte-identical** between the two arms. So on this corpus, in
this VM, with this harness, ±4.7 % is achievable from pure noise on identical
binaries. That single row calibrates every other row.

## Attribution

Each benchmark below was recompiled with both binaries at `-O2 -S` and the
assembly compared byte for byte (`cmp`).

| benchmark | asm | harness B/A | attributable? |
|---|---|---|---|
| `sha256_transform` | **DIFFERS** | 1.047 | **YES — win** |
| `linux_rbtree` | **DIFFERS** | 1.016 | **YES — win** |
| `strlen_bench` | **DIFFERS** | 1.013 | **YES — win** |
| `hash_table` | identical | 1.047 | no — noise (this row *is* the noise floor) |
| `sieve` | identical | 1.015 | no — noise |
| `chacha20_block` | identical | 1.008 | no — noise |
| `zlib_ng_adler32` | identical | 0.997 | no — noise |
| `matmul` | identical | 0.988 | no — noise |
| `lz4_compress` | identical | 0.985 | no — noise |
| `bitops` | identical | 0.985 | no — noise |
| `spectral_norm` | identical | 0.973 | no — noise |
| `arith_loop` | identical | 0.969 | no — noise |

The remaining 22 measured benchmarks and the 17 startup-floor exclusions are all
byte-identical between arms as well: `scripts/differential_corpus.sh` over all 807
`.c` files under `tests/` reports exactly **10** changed translation units, of
which only three are in `tests/benchmark/programs` — the three wins above.

## Conclusion

* **31 of 34 measurable benchmarks cannot regress, because their machine code is
  bit-for-bit unchanged.** That is a stronger guarantee than any timing run.
* **All three benchmarks whose code did change improved**: `sha256_transform`
  +4.7 %, `linux_rbtree` +1.6 %, `strlen_bench` +1.3 %.
* Every "slower" row in the harness output is a byte-identical arm and therefore
  noise, bounded by the ±4.7 % that `hash_table` demonstrates on identical code.

The three wins are independently corroborated by amplified paired A/Bs at sizes
chosen to clear the noise floor (`paired_ab.py`, which refuses to report on
identical arms): `sha256_transform` +3.63 % median / +3.81 % min over 31 rounds
(p=0.0000), `linux_rbtree` +1.05 % / +1.06 % over 61 rounds (p=0.0000),
`strlen_bench` +1.18 % / +2.89 % over 31 rounds (p=0.0012, sd ≈ 9 % so the
magnitude is soft).

## Method consequence

A corpus geomean is the wrong statistic for a narrowly-targeted change: 31
identical arms contribute noise and one of them swings ±4.7 %, which is enough to
bury a real +4.7 % win in a "+0.19 %, no measurable difference" verdict. Screen
for *which translation units changed* first (`scripts/differential_corpus.sh`,
~70 s for 807 files), then time only those, at an amplified size. That is the
procedure this directory documents, and it is the procedure that found the
`lz4_compress` −40.53 % regression two earlier review rounds had missed.
