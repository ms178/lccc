# Pinned GNU gzip 1.14 end-to-end workload

This workload graduates the standalone `gzip_crc32` kernel to a complete
upstream build and execution test. It is selected from
`/home/user/archpkgbuilds/packages/gzip/PKGBUILD` (`pkgver=1.14`) and uses the
GNU gzip 1.14 release archive.

## Pin and license

- Upstream: <https://ftp.gnu.org/pub/gnu/gzip/gzip-1.14.tar.xz>
- SHA-256 used here:
  `01a7b881bd220bfdf615f97b8718f80bdfd3f6add385b993dcf6efd14e8c0ac6`
- License: GPL-3.0-or-later (the runner does not vendor or relicense gzip)
- Retrieved and validated: 2026-08-18

The current package recipe records a different archive checksum,
`7454eb6935db17c6655576c2e1b0fabefd38b4d0936e0f87f48cd062ce91a057`.
Two earlier fetches and the fetch used for this end-to-end run were identical
to the pin above. This discrepancy and the lack of a signature verification in
the current VM remain explicit supply-chain limitations; do not silently treat
the recipe checksum as validated.

## What the runner proves

`run.py`:

1. verifies the archive SHA-256 before extraction;
2. builds the complete project with LCCC and GCC using `-O3
   -march=x86-64-v3` and exactly `make -j2`;
3. runs gzip's complete upstream test suite (30/30 required for each build);
4. generates two deterministic 8 MiB corpora: gzip's pinned source material
   and a reproducible structured-text/binary mixture adapted from the package
   recipe's PGO workload;
5. requires bit-identical gzip streams across compilers at levels 1, 6, and 9,
   and byte-identical decompression;
6. captures binaries, `size`, full `objdump`, build/test logs, raw randomized
   timing samples, individual best/worst/median cases, and arithmetic/geometric
   aggregate ratios;
7. optionally builds a same-LCCC kill-switch control (`--lccc-control-env
   NAME=VALUE`) and reports every treatment/control case, arithmetic/geometric
   means, best gain, and worst regression without hiding an outlier; and
8. pins timing children to one available CPU when `taskset` is present.

Example:

```sh
python3 tests/workloads/gzip-1.14/run.py \
  --archive /path/to/gzip-1.14.tar.xz \
  --lccc "$PWD/target/fastbuild/lccc" \
  --artifact-dir /path/to/results \
  --lccc-control-env CCC_NO_GLOBAL_ADDR_REMAT=1 \
  --rounds 15 --warmups 2
```

The generated report is **VM wall-clock screening only**. It does not provide
PMU evidence or establish bare-metal superiority on Raptor Lake. The script
intentionally does not use the package recipe's nondeterministic
`/dev/urandom`, `$RANDOM`, or timestamp-bearing corpus generation.
