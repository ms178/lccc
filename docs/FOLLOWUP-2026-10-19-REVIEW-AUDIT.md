# Review-AI follow-up audit — 2026-10-19

This note records which review suggestions survived source inspection and
measurement. It intentionally distinguishes a real defect from a plausible
but incorrect proposed implementation.

## 1. Ordinary-identifier lexical depth — finding valid, proposed file frame rejected

`SymbolTable` already exposed `lookup_with_depth` and `lookup_depth`; adding a
second `depth_of` API would duplicate an `Option<usize>` API and make absence
ambiguous. Its initial map is file scope (depth 0). `TypeContext` instead stores
only disposable lexical undo frames: frame index 0 is function scope (depth 1),
so `enum_constant_with_depth` correctly maps index N to N + 1 and uses 0 when
no frame recorded the file-scope enumerator.

Initializing `TypeContext::scope_stack` with a file frame was rejected. The
frames record changes to typedef, tag, layout, alias, cache, and enumerator
maps and `pop_scope` undoes them. Making file scope an undo frame also made
queries such as `is_struct_key_shadowed` treat ordinary file-scope completion
as lexical shadowing. The retained design keeps file scope canonical without
conflating identity with undo ownership. A cross-map unit test now checks depths
0 through 3 and unwind behavior; the C regression adds a nested object over a
function-scope enumerator.

## 2. binary128 zero — matcher complete, sign regression strengthened

The actual `IrConst` integer variants are `I8`, `I16`, `I32`, `I64`, and
`I128`; all five zero forms plus `Zero` are already matched explicitly. There
are no unsigned integer variants. Floating constants are deliberately excluded
(the broad `IrConst::is_zero` helper would incorrectly include negative zero).
The runtime regression now sends both integer zero and `-0.0F128` through the
same SysV argument path and checks all 128 payload bits, including the negative
zero sign bit.

## 3. TLS model CLI — accepted and narrowed

`local-exec` and `initial-exec` remain implemented. `global-dynamic`,
`local-dynamic`, an empty value, and unknown values now fail explicitly instead
of entering the generic warning/ignore policy. Documentation states the actual
limitation: Initial-Exec is a compilation-wide per-object choice; per-symbol
TLS model attributes are not implemented.

## 4. pointer-width spill claim — measured, no benefit claimed

The deterministic SysV generator gained `--emit-only` so the exact 186-shape
source grid can be reproduced without an oracle compiler. Assembly from
`532669b7` and `95bbb4cd`, at `-O2 -w -S`, was byte-identical for both generated
translation units: zero assembly-byte delta and therefore zero spill-byte
delta. The primary glibc follow-up records commands and SHA-256 values. This
grid did not exercise the original glibc pointer-return/load witness, so the
correctness fix is retained without a performance claim.

## 5. glibc workload script — accepted

Host C and C++ compilers must now be supplied together and report the same
target. Their resolved paths, binary hashes, and versions are logged and stored
in the reuse manifest. Patch provenance is captured from the checkout that
actually contains it and checked against the pinned recipe revision when Git
metadata exists. Destructive cleanup is confined to named children of a
canonical, non-root, non-home work directory. Failure handling prints useful
configure/build/install log tails.

## 6. peephole operand classes — accepted

A focused unit test covers XMM-to-GPR, GPR-to-XMM, and memory-source `movq`
forms around the extension fusion, plus a positive plain-GPR control. Non-GPR
forms remain unfused and cannot index GPR alias tables.

## 7. provenance — accepted

The primary follow-up now names the actual validated base (`59f061d2`), merged
commit range (`532669b7..20078a5f`), merge commit (`d8f79a6b`), and PR #868.
Patch applicability is described with the command actually used,
`git apply --check`. It explicitly does not claim a nonexistent
`git am --fuzz` option; `--fuzz=0` is used only with GNU `patch` for the
pinned glibc patch.

## 8. census — review command rejected, real interface verified

There is no repository `cargo run --bin audit` interface. The authoritative
commands are:

```sh
python3 scripts/audit_census.py --selftest
python3 scripts/audit_census.py --check
```

Both passed. No metric grew, so regenerating the baseline would add churn and
weaken the ratchet rather than improve it.
