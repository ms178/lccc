# S101: fresh latest-main, five-compiler and patched-zstd screen

Freshly fetched main remains84a62201. Its independent fastbuild/-O1/-j2 build
passed (3m10s first build). Candidate production compiler is the immutable
S97-fixed compiler built by diagnostic FAST; S99/S100 change evidence/reporting,
not Rust production code. Compiler hashes and actual measured ELF hashes are
retained. No build, CI, provisioning or checkpoint compression overlaps the
completed timing run. An earlier4-case interrupted run is NOT this evidence.

## Native45, nine paired rounds, two excluded warm-ups

All45 programs passed for candidate, latest main, GCC16.2.0, Clang23.1.3 and
ICX2026.1.1. Common C flags: -O2 -march=x86-64-v3 -fwrapv -fno-fast-math
-ffp-contract=off; explicit libm linking. This FP policy differs from historical
S93 timings, which therefore cannot be used as this run's baseline.

The S96 report checker recomputed statistics from raw paired rounds. Ratios
are candidate/reference elapsed time: below1 is better. Only rows with both
arms at least20ms enter these aggregates; all45 rows remain visible.

| Reference | Eligible | Geometric mean | Arithmetic mean |
|---|---:|---:|---:|
|main84|28|1.002269154|1.004321397|
|GCC16.2|28|1.025237135|1.037186046|
|Clang23.1.3|27|1.020838480|1.038989716|
|ICX2026.1.1|28|1.091282688|1.113498172|

Main comparison best: nbody0.788114252,95% paired bootstrap interval
[0.765321635,0.795318899]. Worst: fannkuch1.257396512,
[1.236766345,1.270887140]. Seven main point estimates exceed1%,five exceed2%,
one exceeds5%. Other apparent >2% regressions have intervals crossing1 in
this screen; they are not established persistent regressions. The fannkuch
regression is clearly incompatible with claiming a universal speedup.
GCC comparison range0.645112856(bitops)–1.463227705(fannkuch);20/16/13 eligible
point estimates exceed1/2/5%. Full per-case/vendor details are in the raw and
qualified reports, not hidden by aggregate means or launch-floor wins.

## Real patched zstd1.5.7/pkgrel16,96 timing cells

Refreshed recipe2e62d34d uses unchanged pinned upstream source and81,489-byte
user patch; actual source/patch SHA checks passed before application. Three
fresh builds use uniform-O3/x86-64-v3/fwrapv/no-fast-math/FP-contract-off,
multithreading, legacy and zlib/lzma/lz4 CLI support. They do NOT reproduce the
recipe's LLVM24 Full-LTO/PGO/BOLT packaging pipeline. API drivers share GCC16
compiled timing/validation code and link independently compiled static libs.

Six1MiB corpora × eight options × compression/decompression =96 cells, each
nine paired rounds; pilot-calibrated iteration counts, excluded warm-ups,
per-iteration byte checks and independent GCC CLI decoding. All96 complete,
zero failures. Profiles: fast5,L1,L3,L9,L19,long20,two threads,no checksum.

| Comparison | Direction | GM | AM | Best–worst |
|---|---|---:|---:|---|
|candidate/GCC|compress|1.620623120|1.645531431|1.053215187–3.227042197|
|candidate/GCC|decompress|1.402463129|1.414778093|0.992203036–1.710468871|
|candidate/main|compress|1.000976725|1.001445977|0.899959220–1.067424294|
|candidate/main|decompress|0.999609269|0.999717764|0.960646820–1.050560737|

The requested broad zstd superiority against GCC is emphatically NOT met.
Main-relative >5% screen outliers include ELF-L19/source-L3/random-two-thread
compression, RLE-long20 compression and random-long20 decompression. These
require repetition, not a speedup/regression explanation invented from one run.

## Active tools and retention

Fresh source/L3/four-call profiles: candidate/main/GCC whole-program simulated
Callgrind Ir289235944/288804074/158856414. Those include shared driver and
validation work and are NOT hardware instructions/cycles or timed-region-only
counts. Corresponding3 Memcheck runs pass with zero errors. This does not
prove semantic correctness on its own (S97 explicitly demonstrated that limit).

`engineering/evidence/redteam101/` retains complete raw JSON, reports, build
logs, profiles and replay. `artifacts/S101-MEASURED-EXECUTABLES.tar.xz` stores
all225 measured native ELFs,3 zstd drivers,3 real CLIs and the applied user
patch:232 members, each independently rehashed after archive creation.
This external payload avoids multiplying binaries inside cumulative patches;
its manifest and archive digest are in the patch. All measurements are VM
screening, not physical EPYC7763/Raptor Lake results or external approval.

Current disposition: correctness/reporting repairs retained; overall runtime
acceptance NOT met. Longer follow-up pairs and final stable-tree FAST are
separate pending gates at this checkpoint. No full/slow local CI was run.
