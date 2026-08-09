# Differential fuzzing

`differential_fuzz.py` emits deterministic, defined-behavior GNU C translation
units and compares LCCC's stdout/exit status against a reference compiler.  It
covers fixed-width arithmetic, CFG joins, calls, global state, arrays,
structs/bitfields, `volatile` locals, postfix increment/decrement, and limited
`__int128` paths.

Example:

```bash
python3 tests/fuzz/differential_fuzz.py \
  --ccc ./target/release/lccc --gcc gcc \
  --seeds 0:300 --levels O0,O1,O2,O3,Os,Oz --jobs 2 \
  --out /tmp/lccc-diff-fuzz
```

The generator avoids signed overflow, invalid shifts, division by zero,
out-of-bounds accesses, invalid pointers, and unspecified argument evaluation
order.  A failure retains its source under `--out` for reduction.

## CFG / phi-web stress

`phi_cfg_fuzz.py` specifically creates loop-carried values, branch diamonds,
switch joins, `continue` paths, and postfix increment/decrement operations to
exercise CFG-aware copy coalescing. For example:

```bash
python3 tests/fuzz/phi_cfg_fuzz.py \
  --ccc ./target/release/lccc --gcc gcc \
  --seeds 0:1000 --levels O3,Os --jobs 2 --out /tmp/lccc-phi-fuzz
```
