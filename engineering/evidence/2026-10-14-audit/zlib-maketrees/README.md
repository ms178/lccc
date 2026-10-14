# A14-30 — real zlib-ng Huffman-table miscompile

Baseline LCCC `3b66cd1d73743a7a79f912c01df48ac15cc646ef`; compiler itself built
fastbuild/-O1/-j2. This is a **known failing audit reproducer**, not a fixed test.

`maketrees-small.c` / `gencodes-small.c` are altered, standalone extractions
from zlib-ng 2.3.3. `provenance.json` pins upstream, the archpkgbuilds patch and
post-patch selected source hashes. `LICENSE.txt` retains upstream's notice.
Only dependency declarations, constants and the required type layout were
substituted for large headers; disabled assertion/tracing code was removed.
Separate translation units retain the original call boundary. Do not rename
these as LCCC-authored benchmark algorithms.

## Replay without fetching any package

From this directory (use an absolute LCCC path):

```sh
work=$(mktemp -d)
LCCC=/absolute/path/to/lccc/target/fastbuild/lccc
gcc -O2 maketrees-small.c gencodes-small.c -o "$work/ref"
"$work/ref" > "$work/ref.txt"
cmp expected.txt "$work/ref.txt"
"$LCCC" -O2 maketrees-small.c gencodes-small.c -o "$work/bad"
"$work/bad" > "$work/bad.txt"
diff -u expected.txt "$work/bad.txt"  # expected failure on audit baseline
CCC_NO_IV_WIDEN=1 "$LCCC" -O2 maketrees-small.c gencodes-small.c -o "$work/control"
"$work/control" > "$work/control.txt"
cmp expected.txt "$work/control.txt"
gcc -O1 -fsanitize=address,undefined maketrees-small.c gencodes-small.c -o "$work/san"
ASAN_OPTIONS=detect_leaks=0 UBSAN_OPTIONS=halt_on_error=1 "$work/san" > "$work/san.txt"
cmp expected.txt "$work/san.txt"
rm -rf "$work"
```

Actual results are in `reduced-matrix.json`: GCC/Clang -O2 and GCC ASan+UBSan
match; LCCC -O0/-O1 match; -O2/-O3 mismatch; -O2 with widening disabled matches.
Expected SHA-256:
`022736ff3362ef3591d1f8fc3b7fede5927c0d25ab0813d7ef8cd95efd6ea242`.
Wrong SHA-256:
`531118a2c07a32090b063ce1fb96e713f0716273c7924f114c49c78e8619cd32`.

First visible wrong literal code is 28 rather than 19 at tree index 144;
the first distance code is 144 rather than 0. These are value differences,
not whitespace or non-deterministic file headers.

## Full project and TU isolation

The existing workload runner reproduced 68/69 CTests twice; only `maketrees`
failed. All four independent compression/decompression round trips passed.
To preserve the worktree for investigation use KEEP_WORK=1 and **a dedicated,
disposable WORK_DIR** (A14-22: this runner deletes a caller-supplied directory
otherwise). `tests/workloads/zlib-ng-2.3.3/isolate_maketrees.py` accepts that
source/build pair and replays Ninja's actual flags, three TU mixes, -O0..-O3,
and the widening control, always with the same reference linker. Its return 1
is expected while this defect remains open; inspect the JSON, never suppress it.

`replay-isolation.json` is the validated script's receipt. Only LCCC-generated
maketrees.c is needed to reproduce; trees.c/zutil.c may be GCC-generated.
`pass-triage.json` is an exploratory screen. Only verified real controls such
as CCC_NO_IV_WIDEN and disabling licm/gvn support conclusions; unrecognized
pass names in exploratory probes may be no-ops and are **not negative proof**.

No compiler implementation was changed in this audit. Disabling a pass only
identifies an interaction to investigate. It does not prove the exact faulty
rewrite, justify shipping all widening disabled, or establish a performance win.
