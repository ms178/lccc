#!/usr/bin/env python3
"""Provisioning diagnostics preserve failures; no apt, root or network calls."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import yaml
import ci_annotate_log

ROOT = Path(__file__).resolve().parents[1]


class OracleDiagnostics(unittest.TestCase):
    def run_step(self, gcc_status, qemu_status):
        workflow = yaml.safe_load((ROOT / '.github/workflows/ci.yml').read_text())
        steps = [step['run'] for job in workflow['jobs'].values() for step in job.get('steps', [])
                 if step.get('name') == 'Install GCC 16 and i686 C runtime/development files']
        self.assertEqual(len(steps), 6)
        self.assertEqual(len(set(steps)), 1)
        with tempfile.TemporaryDirectory() as name:
            p = Path(name)
            (p / 'scripts').mkdir()
            (p / 'bin').mkdir()
            # The helper owns annotation encoding now; the workflow must
            # stream it unchanged and preserve its failure status.
            helper = f'echo "GCC provisioning"; exit {gcc_status}\n'
            if gcc_status:
                helper = ('echo "::error title=GCC 16 provisioning failed (mode --multilib)::resolver %25 failure"; '
                          f'exit {gcc_status}\n')
            (p / 'scripts/ci_install_gcc16.sh').write_text(helper)
            sudo = p / 'bin/sudo'
            sudo.write_text(f'#!/bin/sh\ntouch qemu-called\necho "E: qemu resolver"\nexit {qemu_status}\n')
            sudo.chmod(0o755)
            result = subprocess.run(['bash', '-e', '-o', 'pipefail', '-c', steps[0]], cwd=p,
                                    env=os.environ | {'PATH': f'{p / "bin"}:{os.environ["PATH"]}', 'RUNNER_TEMP': str(p)},
                                    text=True, capture_output=True, timeout=10)
            return result, (p / 'qemu-called').exists()

    def test_gcc_failure_stops_qemu_and_is_encoded(self):
        result, called = self.run_step(9, 0)
        self.assertEqual(result.returncode, 9)
        self.assertFalse(called)
        self.assertIn('::error title=GCC 16 provisioning failed (mode --multilib)::', result.stdout)
        self.assertIn('%25', result.stdout)

    def test_qemu_failure_cannot_turn_green(self):
        result, called = self.run_step(0, 13)
        self.assertTrue(called)
        self.assertEqual(result.returncode, 13)

    def test_fetch_failure_survives_long_resolver_diagnostics(self):
        lines = ['Err:4 https://ppa.example.invalid noble InRelease',
                 '  503 Service Unavailable',
                 'W: Failed to fetch repository index']
        lines += ['package policy detail'] * 90
        lines += ['E: Unable to locate package gcc-16']
        chosen = ci_annotate_log.select(lines, context=3, tail=40)
        self.assertIn(lines[0], chosen)
        self.assertIn(lines[1], chosen)
        self.assertIn(lines[-1], chosen)

    def test_success_never_emits_failure_annotation(self):
        result, called = self.run_step(0, 0)
        self.assertTrue(called)
        self.assertEqual(result.returncode, 0)
        self.assertNotIn('::error', result.stdout)


if __name__ == '__main__':
    unittest.main()
