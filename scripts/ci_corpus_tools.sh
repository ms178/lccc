#!/usr/bin/env bash
# Deliberately compiler-free subset of the local/hosted CI mirror. This is NOT
# ci_local.sh --fast, full compiler CI or performance validation. It installs
# no packages and does not build/query a compiler. Tests use mocks and actual
# miniature Git repositories; process-budget tests run Python only.
set -euo pipefail
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo"
python3 scripts/edg_changes_mine.py selftest
python3 scripts/edg_corpus_mine.py selftest
python3 scripts/edg_corpus_mine.py verify-index
python3 scripts/edg_changes_mine.py verify-artifacts
python3 -m unittest discover -s tests/corpus -p 'test_*.py'
python3 scripts/test_differential_corpus_paths.py
python3 scripts/check_script_imports.py
python3 scripts/check_ci_workflow_shell.py
python3 scripts/check_ci_gate_parity.py
python3 scripts/test_ci_gate_parity.py
python3 scripts/check_doc_links.py
printf '%s\n' 'PASS: compiler-free corpus/tooling checks only; full compiler CI remains UNGATED.'
