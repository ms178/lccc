#!/usr/bin/env python3
"""Run the real CI functions with stub Cargo/free; do not build or trigger OOM."""
from pathlib import Path
import json
import subprocess
import tempfile

root = Path(__file__).resolve().parents[4]
source = (root / 'scripts/ci_local.sh').read_text()
functions = []
for name in ['cargo_test_repeated', 'clippy_gate']:
    start = source.index(name + '() {')
    functions.append(source[start:source.index('\n}', start) + 2])
setup = '''unset CARGO_PROFILE_FASTBUILD_DEBUG CARGO_INCREMENTAL CI_LOCAL_JOBS
free() { printf 'Mem: 1982 0 0 0 0 0\\n'; }
cargo() { printf 'cargo %s\\n' "$*"; }
python3() {
    case "$1" in
        scripts/ci_test_report.py)
            if [ "${2:-}" = "--self-test" ]; then return 0; fi
            shift; [ "$1" = "--" ] && shift; "$@" ;;
        scripts/check_clippy_policy.py)
            printf 'clippy-policy jobs=%s\\n' "${CARGO_BUILD_JOBS:-unset}" ;;
        *) return 2 ;;
    esac
}
'''
records = []
with tempfile.TemporaryDirectory(prefix='ci-memory-audit-') as temp:
    for label, call in [('fresh', 'clippy_gate'),
                        ('after-test', 'cargo_test_repeated; clippy_gate')]:
        result = subprocess.run(['bash', '-c', setup + '\n'.join(functions) + '\n' + call],
                                cwd=temp, capture_output=True, text=True, timeout=10)
        records.append(dict(arm=label, returncode=result.returncode,
                            stdout=result.stdout, stderr=result.stderr))
print(json.dumps(records, indent=2))
# Desired contract: running an earlier gate must not weaken automatic memory
# protection in a later gate. Explicit caller settings require separate tests.
clippy_commands = [next((line for line in row['stdout'].splitlines()
                        if line.startswith(('cargo clippy ', 'clippy-policy jobs='))), None) for row in records]
raise SystemExit(int(any(row['returncode'] for row in records)
                     or None in clippy_commands or clippy_commands[0] != clippy_commands[1]))
