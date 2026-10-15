"""Real synchronization tests, not elapsed-time guesses about parallelism."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class SchedulerTests(unittest.TestCase):
    def run_shell(self, body, jobs=2):
        with tempfile.TemporaryDirectory() as td:
            return subprocess.run(['bash', '-c', f'''
set -u -o pipefail
PASSED=0; FAILED=()
export CI_LOCAL_GATE_JOBS={jobs}
source "{ROOT}/scripts/ci_gate_scheduler.sh"
cd "$1"
{body}
''', 'test', td], capture_output=True, text=True, timeout=15)

    def test_actual_overlap_and_exclusive_barrier(self):
        p = self.run_shell('''
a() { touch a; for ((i=0;i<100;i++)); do [ -e b ] && return 0; sleep .01; done; return 9; }
b() { touch b; for ((i=0;i<100;i++)); do [ -e a ] && return 0; sleep .01; done; return 9; }
barrier() { test ${#CI_GATE_PIDS[@]} = 0 && test "$PASSED" = 2; }
ci_gate_submit no-conflict-markers a
ci_gate_submit ci-workflow-shell b
ci_gate_submit build barrier
ci_gate_drain
[ "$PASSED" = 3 ] && [ ${#FAILED[@]} = 0 ]
''')
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertEqual(p.stderr, '')

    def test_bound_and_every_failure_recorded_once(self):
        p = self.run_shell('''
for name in no-conflict-markers ci-workflow-shell script-imports volatile-destructuring; do
    ci_gate_submit "$name" bash -c 'exit 7'
    [ ${#CI_GATE_PIDS[@]} -le 2 ] || exit 9
done
ci_gate_submit unknown-gate true
ci_gate_drain
[ "$PASSED" = 1 ] && [ ${#FAILED[@]} = 4 ] && [ "${FAILED[3]}" = volatile-destructuring ]
''')
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertEqual(p.stdout.count('FAIL  '), 4)
        self.assertEqual(p.stderr, '')

    def test_serial_reference(self):
        p = self.run_shell('''
ci_gate_submit no-conflict-markers true
[ "$PASSED" = 1 ] && [ ${#CI_GATE_PIDS[@]} = 0 ] || exit 3
ci_gate_submit arbitrary false
[ "${FAILED[*]}" = arbitrary ]
''', jobs=1)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)

    def test_invalid_resource_limit_rejected(self):
        self.assertEqual(self.run_shell('true', jobs=3).returncode, 2)
