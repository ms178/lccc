# S106 — scoped runtime improvement, not broad zstd superiority

Compiler implementation is unchanged from S104's4855/0/9 unit-validated tree.
Main rechecked at2f477ce0. The archpkgbuilds head advanced to
b4f57d901c56d9be86f986ea8ce860f57638b4ea; its zstd PKGBUILD and81,489-byte patch
are byte-identical to the measured recipe. Latest-recipe provenance is retained.

## Positive runtime result: explicitly a focused helper benchmark

The separate `mask_throughput.c` workload executes **40 million fixed iterations**,
calling outlined32/64-bit mask helpers on unsigned pseudorandom values and
indices. It models the outlined-helper opportunity; it is NOT an added member
of the45-case corpus, a substitute for zstd, or a claim about application speed.
All five compilers produce the same checksum.31 randomized paired rounds,
two excluded warm-ups, CPU0, disabled child-process ASLR, identical GNU C17/
O2/x86-64-v3/wrapping/strict-FP policy. Raw wall and child-CPU times are retained.

| Reference | Candidate/reference wall-time median | 95% paired-bootstrap interval |
|---|---:|---:|
|main2f477ce0|0.800889435|0.717085984–0.896569776|
|GCC16.2|1.191141533|1.126351598–1.253822812|
|Clang23.1.3|1.168991771|1.126483654–1.227891336|
|native ICX2026.1.1|1.170724443|1.094694200–1.215017713|

That is **19.91% less elapsed time than main** in this focused workload
(interval10.34–28.29% less), not a20% zstd speedup. Child-CPU main-relative
median0.796057645, interval[0.716616362,0.896265354], agrees in direction.
The candidate still trails all three vendor compilers in this test.

Across the31 paired elapsed-time ratios, main-relative geometric mean is
0.809386161 and arithmetic mean0.844170227; individual ratios range
0.451306214–1.711274770. These are repeated samples of ONE workload, not
a corpus aggregate. The wide range discloses substantial VM sample variation;
the median interval above is a within-run statistical result, not independent
physical-host replication or a universal20% gain. Vendor-relative GM/AM:
GCC1.238920297/1.280132749, Clang1.184588476/1.239938407,
ICX1.149313794/1.167647719. All samples, including outliers, are retained.

## Application controls: uncertainty is not erased

The complete96-cell/full1MiB zstd screen and all19 longer31-pair follow-ups
remain as reported in S105. All correctness/frame checks pass. Primary screen
GM/AM: compression/main0.995779954/0.996157408; decompression/main
1.001694711/1.001724594. These are descriptive aggregates, not established
population-wide speedups. The45 original corpus ELFs remain byte-identical.

Additional independent31-pair controls,800ms target, ASLR disabled and a
byte-identical baseline executable:

| Case | Candidate/main | Interval | Identical control/main | Interval |
|---|---:|---|---:|---|
|source/L3 compress|0.989245205|0.983135975–1.002685297|1.006577430|0.995645020–1.021387319|
|random/long20 compress|0.968786614|0.935339608–0.998307050|1.017530343|0.963539924–1.054774879|

The first source/L3 repeat was0.989357[0.986813,0.991171]; its sign/magnitude
persist but the independent interval crosses1. The earlier random/long20
regression1.036771[1.006656,1.058528] reverses here. Neither a reliable broad
speedup nor a universally regression-free application result is certified.

A further within-process diagnostic removes separate executable/process setup:
600 global symbols per baseline/candidate library were namespaced, with exact
.text hex identities verified before/after renaming. Both versions share one
GCC driver and buffers; two baseline arms execute the SAME code with separate
contexts. Per-switch warm-ups are excluded and every timed compression is
independently decoded by GCC-built zstd. The harness passes Memcheck with0
errors/0 live bytes.31 randomized rounds per case yield:

| Case | Candidate/main | Interval | Same-code control/main | Interval |
|---|---:|---|---:|---|
|source/L3|1.006327164|0.978005541–1.025630996|0.984684374|0.973732096–1.009235582|
|random/long20|1.045362646|0.993731374–1.133045292|0.993906050|0.972149208–1.070991308|

This changes cache/context conditions and is explicitly a diagnostic, not pooled
with the primary matrix. It does NOT confirm an application improvement. The
random/long20 concern remains a variance/layout investigation, not a defect
silently waived or a proven fix. Whole-program random/long20 Ir differed by6,
whereas source/L3 Ir drops289235575→284770075; simulated instructions are not
PMU cycles. No favorable subset replaces the complete matrix or these controls.

## Acceptance decision and limits

Retain the semantics-proven native selection for its scoped, measured helper
runtime gain and lower executed instruction count. Preserve all legality,
flag, ABI, pinning and fallback guards. Do not unpin FSE's scratch-shaped
prologue save to manufacture a win. Broad zstd/GCC superiority and reliable
application-level speedup remain **unmet**; native performance approval is
limited to the explicitly measured scope above.

Current LCCC builds and new fixture controls have no warnings. GCC's legacy
zstd v0.1 build still emits six disclosed diagnostics. Recorded builtin/ISA debt
and incomplete exhaustive PR-hunk closure remain in backlog.md. No independent
army approval, universal zero defects, or physical EPYC/Raptor Lake result is
claimed. Final FAST/rustfmt/Clippy status belongs to the matching external
delivery stamp; full/slow local CI is not authorized or run.
