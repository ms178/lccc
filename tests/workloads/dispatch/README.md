# Dispatch-shaped micro-workloads (AUD-CPU-1 evidence)

Two small programs whose purpose is to make the *cost of the runtime
`__builtin_cpu_supports` model* measurable with a plain instruction counter
(valgrind/callgrind), because this VM has no PMU and wall-clock numbers on a
shared 2-vCPU box are not evidence.

* `two_way_dispatch.c` — one dispatch site, called 50 M times from a
  `noinline` callee, so the loop-invariant probe cannot be hoisted out of the
  loop.  `-DINIT_ONLY` compiles the single-call variant that exposes the
  one-time CPUID initialisation instead.
* `crc32_dispatch.c` — the idiom every runtime-dispatched library uses: one
  `__builtin_cpu_supports` query chooses between a narrow and a wide integer
  kernel over a deterministic 64 KiB buffer.  It is byte-for-byte
  output-identical across compilers, so the instruction counts are comparable.

`run.sh` builds each program with LCCC (and, when present, GCC and Clang),
runs it under `callgrind`, and prints `PROGRAM TOTALS` per binary.  The
numbers are instruction counts, i.e. independent of the host's load and
frequency — the only honest measurement available here.

No wall-clock claims are made from this directory: the target hardware is an
i7-14700KF, and this VM cannot stand in for it.
