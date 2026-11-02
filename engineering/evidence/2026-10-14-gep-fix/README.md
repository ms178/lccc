# Constant-GEP / CTest repair evidence

Implementation: S14 `bc626f2c86af4b9cf4295a342777f9d4e847343d`, upstream base
`e047aa1eb2ae59b3943a64ab7ec5e5dddb89a3ff`. The current follow-up is
`engineering/FOLLOWUP-2026-10-14-constant-gep-fix.md`.

- `initial-validation.json`: the first targeted receipt; its CI-pending status
  describes that checkpoint, not the final delivered snapshot.
- `backend-controls-before.json`: all eight peephole controls still fail.
- `backend-controls-after.json`: all 14 default/backend/pass arms pass.
- `small-before.s` / `small-after.s`: raw local assembly, not a throughput score.
- `extended-validation.json`, `memcheck-*.txt`: sanitizer/target/Memcheck checks.
- `kernel-matrix.json`: bounded O0/O1/O2/O3 native output differentials, 28/0/0.
- `zlib/`, `expat/`, `gzip/`: fresh full-project evidence, not old results relabelled.
  Expat's 4932 internal checks are in `upstream-{gcc,lccc}.txt`; one CTest wrapper
  is not one internal test. gzip timing samples are rejected for inference:
  one round, no warmup, other work active. Its correctness results are usable.
- `callgrind/`: initial GCC context plus three equal-path-length paired rounds;
  summary and 30 full manifests. `raw-profiles.tar.xz` retains all 60 profiles;
  their hashes were checked against the individual manifests. Extract with
  `tar -xJf raw-profiles.tar.xz -C EMPTY_DIRECTORY`. No executables are archived.
- `oracle/`: active v3/Raptor-Lake CE probes, including ICC's refusal and fresh
  ICX effective version. `oracle-icc-compat/` is a separately labelled target.
- `rust-tests.txt`: completed Rust-test phase of an intentionally interrupted
  first fast run, NOT a final CI pass. Final certificate/log lives in the
  delivery artifacts because embedding that receipt would change its own tree.

Replay primary gates from the repository root:

```sh
CCC=target/fastbuild/lccc bash tests/regression/check_late_const_gep.sh
python3 tests/workloads/expat-2.8.5/junit_check.py --self-test
```

Replay Callgrind with the existing tools (the saved baseline compiler must be
rebuilt from the pre-fix base if not available):

```sh
python3 scripts/callgrind_corpus.py --mine target/fastbuild/lccc \
  --ref /absolute/path/to/pre-fix-lccc --out /owned/disposable/paired \
  --only gzip_crc32,zlib_ng_adler32,expat_siphash24,sqlite_varint,linux_find_bit,glibc_memcmp
```

Native/CE tool versions are explicit. Older local tools are independent
semantic controls, not claims that the requested latest local toolchain matrix
was provisioned. Original baseline failure evidence remains in the dated audit
directory. Diagnostics copied as text have trailing whitespace normalized for
patch hygiene; hashed raw Callgrind profiles are unchanged.
