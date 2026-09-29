# The cross-vendor oracle

`tools/oracle/godbolt_oracle.py` answers two questions about lccc's code
generation, in this order, and refuses to conflate them:

1. **Is it correct?** Every program in `tests/oracle/programs/` is compiled by
   lccc locally *and* by three independent vendor compilers on Compiler
   Explorer, then **all four are executed** and their stdout and exit status
   compared. A divergence is a miscompilation.
2. **How big is it?** Instruction counts and code size, reported as data.

The order matters, and so does the refusal. `tools/linker/oracle_cmp.py`
already compared lccc-i686 against the local `gcc -m32` by counting
instructions. That uses one reference, on one target, and says nothing about
correctness — **a miscompilation that is perfectly small still shows up as a
win.** Size is the weaker signal and is treated as such here: it is never a
pass/fail gate, because vectorising *reduces* the instruction count while
*raising* throughput, so a size table alone can invert the truth.

## The oracles

| id | compiler | vendor |
|---|---|---|
| `cg162` | GCC 16.2 | GNU |
| `cclang2310` | Clang 23.1.0 | LLVM |
| `cicxlatest` | ICX latest (2026.0) | Intel |

`--oracle-set {default,gnu,llvm,intel,all-vendors}` picks a preset;
`--oracles a,b,c` names ids directly; `--list` prints them.
A set of one vendor is not a cross-vendor oracle, and the self-test asserts
the default is exactly one of each.

## Running it

```sh
python3 tools/oracle/godbolt_oracle.py                 # semantics + size
python3 tools/oracle/godbolt_oracle.py --filter pixmap
python3 tools/oracle/godbolt_oracle.py --oracles cg162,cclang2310
python3 tools/oracle/godbolt_oracle.py --json out.json
python3 tools/oracle/godbolt_oracle.py --bench 15      # + local timing
python3 tools/oracle/godbolt_oracle_selftest.py        # offline, no network
```

Exit status is 0 only when every program that ran agreed with every oracle
that ran **and at least one oracle ran**. Zero oracles is an error, never a
pass.

## Compiler Explorer is evidence, not authority

A remote that rejects a program, times out or rate-limits is a `SKIP` with its
reason printed. It is never allowed to pass silently, and an lccc-side
divergence is a `FAIL` regardless of what the oracle says. This is not
hypothetical caution: §2 of `docs/PR663_CI_REPAIR_AND_FOLLOWUP.md` is a case
where the **reference linker** was the wrong one — GNU ld 2.42 collapses three
local GOT64 references through one section symbol and answers `1 0 0` where
the answer is `1 1 1`. An oracle-agreement gate would have passed that bug and
blamed the compiler for it.

HTTP 429 and 5xx are retried with exponential backoff honouring `Retry-After`,
and results are memoised per `(compiler, flags, source, execute)` so a sweep
does not re-spend the rate limit on identical work.

## What it has already caught

The harness found a bug **in its own test programs**: `rotl(x, 32)` and
`rotl64(x, 64)` shift by the full width, which is undefined even for unsigned
types. Clang 23.1 and GCC folded it differently, the oracle reported a
divergence, and the correct conclusion was *the program has no defined
answer*, not *Clang is wrong*. The programs now mask the count, and the
self-test keeps them masked.

That is the general lesson and the reason the semantics table prints the
program's actual output: an oracle difference is a question to adjudicate, and
the adjudication sometimes lands on the test.

## Limits, stated plainly

* **The size table counts instructions in `-S` output**, not bytes in a
  linked image, so all four numbers are computed the same way but none of
  them is a cycle count. `.rodata` and alignment are invisible to it.
* **`--bench` is best-of-N wall clock on this host.** On these micro-workloads
  a run is ~1 ms, which is mostly `fork`/`exec`: the ratios are near 1.00 and
  that is *noise, not a tie*. Treat the column as a smoke test for gross
  regressions, not as a performance verdict. Making it meaningful needs
  workloads in the tens of milliseconds.
* **A remote binary cannot be timed reliably.** Compiler Explorer's `binary`
  tool returns no `downloads`, and its `asm` view is a *disassembly*: `.comm`,
  `.local`, `.section` and `.extern` are stripped, so it is not always a
  linkable program. The harness therefore assembles the oracle's assembly
  locally and — before trusting the resulting timing — requires it to
  reproduce Compiler Explorer's *own* execution output. A rebuild that does
  not is a `SKIP`, never a reported divergence.
* **Clang cannot be timed at all here**: `-masm=att` is honoured by GCC and
  ICX, but Compiler Explorer's Clang instances answer in Intel syntax and
  ignore the `intel` filter, so the local assembler rejects it. Clang's
  semantics and size are unaffected; only the timing column skips it.
* The oracles are network-dependent and their ids track Compiler Explorer's
  catalogue. A renamed id shows up as a `SKIP`, never as a pass.

## Current result

`-O2`, 8 programs, x86-64, `ratio` = lccc ÷ oracle (lower is smaller):

| program | lccc | GCC 16.2 | Clang 23.1 | ICX |
|---|---:|---:|---:|---:|
| bitops | 122 | 128 | 130 | 213 |
| control | 168 | 116 | 211 | 148 |
| float | 155 | 123 | 154 | 162 |
| int_alu | 92 | 68 | 23 | 28 |
| loops | 130 | 229 | 282 | 229 |
| memory | 242 | 579 | 296 | 449 |
| pixmap | 256 | 283 | 467 | 524 |
| strings | 165 | 197 | 350 | 217 |
| **total** | **1330** | **1723** | **1913** | **1970** |
| **ratio** | — | **0.772** | **0.695** | **0.675** |

**8/8 semantically identical on all three vendors**, and 23–33 % fewer
instructions than any of them.

The one place lccc is clearly behind is `int_alu` (92 vs Clang's 23): Clang
unrolls the fixed-trip-count loops and constant-folds the result away
entirely, so it emits no loop at all. lccc strength-reduces the division by
10 and by 7 into multiply/shift sequences and keeps everything in registers —
a good loop body — but it never does the compile-time evaluation that would
delete it. That is the concrete optimisation this data points at.
