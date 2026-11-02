#!/usr/bin/env bash
# Corpus-wide GNU-as acceptance gate: every regression/benchmark TU must
# assemble with GAS at the ISA its flags permit (no ISA leaks) and with no
# syntax defects the builtin assembler happens to tolerate.  See
# scripts/gas_isa_scan.sh for the two defect classes and the arch table.
set -euo pipefail
repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
command -v "${AS:-as}" >/dev/null || { echo "SKIP: no GNU as"; exit 0; }
LCCC=${CCC:-$repo/target/fastbuild/lccc} exec "$repo/scripts/gas_isa_scan.sh" --all
