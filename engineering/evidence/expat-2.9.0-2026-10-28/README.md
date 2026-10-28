# Expat 2.9.0: qualifier regression and bounded workload evidence

## Source and configuration

This is the upstream Expat tag source, not a reduced lookalike: tag `R_2_9_0`
resolves to commit `a76b1174c1ea52abb639a4c69d52b630a8a106a2`. The downloaded
GitHub tag archive had SHA-256
`6e8ce7b52ebbaa423c59becc7cf1e6875c42e49228c062ac8123c10e700cec27`.
That archive is distinct from the published release tarball; its redirected
release asset was not accessible here. The checked-in `expat_config.h` is a
**manual Linux x86-64 configuration**, not configure or CMake output. It uses
`HAVE_GETRANDOM`, namespaces, DTDs and general entities, 1,024 context bytes,
`XML_STATIC`, and `XML_TESTING`; both compilers receive identical options.
No CMake, autotools or generated release `configure` was available, so this is
not an upstream configure/CTest build. Source and generated build products stay
outside the checkout.

GCC 16.2.0, Clang 23.1.4 and the local LCCC compiler built the upstream library
files `xcs.c`, `xmlparse.c`, `xmltok.c`, `xmlrole.c` and `random_getrandom.c`
with `-O2 -std=gnu11`. GCC 16.2 compiled the 17 upstream test translation
units and linked the runner separately against each library. Each runner passed
**5,196 checks, zero failures**. LCCC also compiled all 17 test translation
units; the GCC-linked runner with LCCC-compiled tests and library passed the
same 5,196 checks. This is the default test suite under this manual
configuration, not every Expat configuration or platform.

## Bug and scope

Before the fix, `typedef int *P; void f(const P p) { *p = 1; }` was rejected
as a read-only write, although only `p` is const. Conversely,
`typedef int *P; void f(const P *p) { *p = 0; }` was incorrectly accepted.
The old compiler (SHA-256
`955dc9b920aa4ead9bc3557cbb775f948814840229bb8ad706759e52eb3eb42c`)
also chose the wrong `_Generic` arm for a const-qualified pointer-typedef
field. The original Expat `xmlparse.c` failed with 16 false read-only errors
on `const XML_Parser` variables. Qualifying the aliased pointer level instead
of its base, retaining the level across derived declarators and using the same
base-const interpretation in sema and lowering fixes those cases. Tests cover
alias chains, members, arrays, casts, compound literals, function returns and
pointer-to-array nesting, including both legal writes and required errors.

This is not complete qualifier support. In particular, GCC accepts
`void f(int *x) { const __typeof__(x) p=x; *p=1; }`, while LCCC still rejects
it; LCCC also accepts the invalid `const __typeof__(x) *p=&x; *p=0;`.
Some typedef-based diagnostic spellings still differ from GCC. Those gaps and
the remaining `typeof`/qualifier representation issues remain open.

## Workload and measurements

The upstream `tests/benchmark/benchmark.c` was compiled once by GCC for all
arms; only the library compiler differs. Its namespace-enabled workload uses a
deterministic synthetic 1,400,832-byte XML document, 8,192-byte chunks and
five parses per timing sample. The document has SHA-256
`ad11dae6d1a4661377dc3531105fe6b177876186197f1721b3c7fb38eb3e944e`.
Legal trailing XML whitespace pads it to a multiple of the chunk size: the
upstream benchmark advances its pointer by a whole chunk after the last parse,
so this keeps the final pointer at one-past-end. The independent GCC-compiled
callback harness produced the same result for all three libraries:
`40001 40001 217784 7f21649baacdab7c` (start/end events, text bytes, FNV-1a
stream hash). Equal output is a useful screen, not proof of all behavior.

On this two-vCPU virtualized Intel Xeon environment, 40 randomized paired
rounds after two warmups used the upstream `clock()` CPU-time measurement.
The table gives the median of within-round ratios and a 10,000-resample
percentile bootstrap interval; raw samples and executable hashes are in
`paired-observed.json`. Higher means more CPU time than GCC.

| Library compiler / GCC | Median ratio | 95% bootstrap interval |
|---|---:|---:|
| LCCC / GCC 16.2 | 1.3161 | [1.3077, 1.3206] |
| Clang 23.1.4 / GCC 16.2 | 0.9387 | [0.9352, 0.9427] |

This measured configuration is slower with LCCC; it is not evidence of a
speedup. It does not establish performance on Raptor Lake or other physical
targets, other XML inputs, other builds, or stable causal attribution. VM
scheduling, frequency, input and randomized parser hash salts can matter.

Valgrind 3.19.0 Callgrind, with I1/D1 `32768,8,64`, LL `33554432,16,64`,
cache and branch simulation, ran the same upstream benchmark for two parses:

| Library compiler | Whole-process simulated Ir |
|---|---:|
| GCC 16.2 | 260,948,419 |
| LCCC | 320,759,808 |
| Clang 23.1.4 | 248,531,002 |

These are single-run simulated instruction counts, including startup and I/O;
they are not PMU counts, cycles, target-hardware timings or the parse-only
`clock()` region. Hash salts can change counts across runs. The LCCC compiler
used for the measured library was SHA-256
`ef48c7007228476ede0088510857eb5878889c170568ad1fcab855e9d3c5bbfa`.

## Reproduction

Unpack the verified tag source into a private directory. With the compilers on
`PATH`, the following compiles the libraries and test runners, generates the
same input, checks callbacks, and writes paired samples under the chosen
private output directory. It never installs or extracts packages.

```sh
GCC=gcc-16 LCCC=/path/to/lccc CLANG=clang \
  ./engineering/evidence/expat-2.9.0-2026-10-28/run.sh \
  /private/path/libexpat-R_2_9_0/expat /private/output/expat
```

Set `VALGRIND` to a Callgrind-capable executable and, for an isolated Debian
Valgrind, `VALGRIND_LIB` to its private `usr/libexec/valgrind` directory to
repeat the simulated-count stage. `ROUNDS` may reduce the timing sample count
for a smoke test; fewer samples are not the 40-round result above. The script
checks four source-file hashes, the generated input and callback result; verify
the complete archive separately before unpacking. Outputs may differ across
compilers, machines and environments.
