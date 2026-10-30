# Workload-derived VM runtime screen, 2026-10-30

This is a user-space compiler-codegen screen, **not a kernel benchmark** and
not a measurement on the target i7-14700KF. It is exploratory: this virtual
Xeon (family 6 model 106) has only 2 vCPUs, and the five paired samples show
substantial dispersion. No speedup/supremacy or kernel-performance claim.

The existing `benchmark_native_batched.py` compiled each original TU once with
`-O2 -march=x86-64 -fwrapv -ffp-contract=off`, directly to the measured object.
The harness verified object identity across symbol renaming, output equality,
and randomized matched rounds. Five samples per compiler run the entry in fresh
forked children, excluding fork/exec/wait while including entry-page faults.
Timed CPU 0 was pinned; the kernel build was suspended and the concurrent CI
process group was pinned to CPU 1 until measurements ended. The VM and host
remain shared; frequency and scheduling are not controlled. The full JSON
contains actual samples, hashes, identities, calibration and reproducibility
metadata. Workload setup and correctness checks run inside the timed entry, so
ratios cannot be ascribed solely to the named hot function.

The neutral driver was GCC 16.2. To link ICX's `_intel_fast_memcpy`, a temporary
GCC wrapper appended `-L<icx-lib> -Wl,-rpath,<icx-lib> -lirc` to links for all
arms, while delegating compilation/version/header queries unchanged. GNU
linker's `--as-needed` determines which program depends on that library. ICX
2026.2.0 was installed locally; Clang is 23.1.4. Each program was compared
against GCC output; the driver was not used to compile kernel code.

Median paired elapsed-time ratio to GCC (lower is faster; not a kernel result):

| workload | LCCC/GCC | Clang/GCC | ICX/GCC | LCCC paired range |
|---|---:|---:|---:|---:|
| gzip longest_match | 1.180 | 0.961 | 0.832 | 1.098–1.208 |
| zlib-ng compare256 | 1.602 | 0.833 | 0.928 | 1.388–2.109 |
| Expat XML scan | 1.125 | 1.003 | 1.542 | 0.874–1.354 |

The zlib-ng outlier is a lead, not a localized diagnosis. In the
`zlib_compare256_64` disassembly, LCCC carries/increments two pointers and a
length separately; GCC uses a single index and indexed loads; Clang also
unrolls by four. Inspection alone does not prove those instructions account for
the whole-entry difference: `check_all_offsets`, input preparation, checksum,
and page faults are timed too. Use a separately validated function-level
benchmark and A/B evidence before changing IV or loop-unrolling transforms.
Baseline x86-64 instructions were requested; `tzcnt` decoded from GCC/Clang's
`F3 0F BC` is architecturally `rep bsf` on non-BMI CPUs, and this workload
executes it only for nonzero differences.

The initial common-GCC link without the Intel library failed with unresolved
`_intel_fast_memcpy`; it was not counted as a result. `-m32` is not supported
by this ICX bundle, so its regression comparison was x86-64 only. GCC, Clang
and LCCC passed the SELinux statement-expression test on both m32 and m64 at
O0/O2; ICX passed m64 O0/O2.

For later reproduction the equivalent portable wrapper is now
`scripts/benchmark_gcc_with_irc.sh`; set `LCCC_BENCH_REAL_GCC` and
`LCCC_BENCH_IRC_LIBDIR`, then pass its absolute path as `--gcc`. The JSON
records the temporary wrapper path used in this particular run. Existing
compiler/source hashes and the raw samples, not the wrapper filename, identify
the measured bodies.
