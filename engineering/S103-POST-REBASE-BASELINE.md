# S103 post-rebase baseline — before the S104 selection experiment

Both compilers freshly built from 2f477ce0/main and rebased candidate.
Explicit GNU C17: all45 workloads × five compilers pass, **225/225**. All45
candidate/main ELFs are byte-identical; all180 candidate/vendor ELFs match the
S101 timed programs. Existing vendor timings are therefore code-identity-linked,
not new measurements. The old84a62201 main-relative ratios remain historical.
Fresh semantic controls: candidate/main/GCC-O0 each65/65; no build diagnostics.

Patched zstd was rebuilt with explicit GNU C17. Candidate/main drivers and CLIs
are byte-identical. The shared driver's language selection changes its bytes,
so the entire96-cell, nine-pair, full1MiB matrix was rerun rather than silently
transferring old driver timings. All96 cells pass; three Memcheck runs report
zero errors and three Callgrind profiles are retained. Source/input identities
are unchanged; the six S101 inputs are reused by verified hashes.

Elapsed ratios, below1 is better:

| Reference | Direction | n | GM | AM | Best | Worst |
|---|---|---:|---:|---:|---:|---:|
|GCC16.2|compress|48|1.629649647|1.655834895|1.047143174|3.122508655|
|GCC16.2|decompress|48|1.402704128|1.415186431|1.012762514|1.694372344|
|new main|compress|48|1.000067732|1.001046680|0.805258118|1.080297633|
|new main|decompress|48|0.997394657|0.997484943|0.932704639|1.025012997|

The main/candidate program bytes are identical: apparent individual main-relative
wins/regressions are run noise/confounding, not code improvements. In particular,
no 19% point win can be accepted from this screen. The >60% compression gap to
GCC remains real work, not a delivered success. Candidate/main build warnings0;
GCC legacy v0.1 retains six disclosed warnings. This is not universal warning-free
certification. Timing ran without overlapping builds, CI or compression.

Evidence: `engineering/evidence/redteam103/`; actual new payloads in
`artifacts/S103-ZSTD-GNU17-EXECUTABLES.tar.xz` with a patch-contained manifest.
