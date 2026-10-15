"""Exercise both cache checks in the real provisioner, with tool validation isolated.

No downloads or toolchain builds occur. Existing self-tests cover the validators;
these tests cover orchestration after an incomplete cache reaches the lock.
"""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class GasLockedRecheck(unittest.TestCase):
    def run_provisioner(self, *, complete=False, completed_while_waiting=False):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            prefix = root / 'prefix'
            tools = prefix / 'bin'
            tools.mkdir(parents=True)
            for name in ('as', 'objdump', 'objcopy'):
                p = tools / name
                p.write_text('#!/bin/sh\nexit 0\n')
                p.chmod(0o755)
            consumer = prefix / 'share/opcodes/aarch64-sys-regs.def'
            if complete:
                consumer.parent.mkdir(parents=True)
                consumer.write_text('consumer fixture\n')
            mocks = root / 'mocks'
            mocks.mkdir()
            for name, body in {
                'curl': 'exit 1',
                'flock': '''if [ -n "${FILL_GAS_INPUT:-}" ]; then
mkdir -p "$(dirname "$FILL_GAS_INPUT")"
printf 'consumer fixture\\n' > "$FILL_GAS_INPUT"
fi''',
            }.items():
                p = mocks / name
                p.write_text('#!/bin/sh\n' + body + '\n')
                p.chmod(0o755)
            source = (ROOT / 'scripts/ensure_gas_247.sh').read_text()
            anchor = 'target=${1:-riscv64-linux-gnu}'
            self.assertEqual(source.count(anchor), 1)
            # Only the independent binary/version/provenance layers are mocked.
            # The actual file checks, lock, both early returns and fallback run.
            source = source.replace(anchor, 'validate_pair() { return 0; }\n'
                                    '_pair_provenance_ok() { return 0; }\n' + anchor)
            script = root / 'provision.sh'
            script.write_text(source)
            env = dict(os.environ, PATH=str(mocks) + ':' + os.environ['PATH'],
                       GAS_DL_DIR=str(root/'downloads'), GAS_CACHE=str(root/'cache'))
            env.pop('LCCC_ENSURE_GAS_CHECK_ONLY', None)
            env.pop('FILL_GAS_INPUT', None)
            if completed_while_waiting:
                env['FILL_GAS_INPUT'] = str(consumer)
            return subprocess.run(['bash', str(script), 'aarch64-linux-gnu', str(prefix)],
                                  env=env, capture_output=True, text=True, timeout=15)

    def test_incomplete_consumer_cache_is_not_certified_under_lock(self):
        p = self.run_provisioner()
        self.assertNotEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn('FATAL: no mirror reachable', p.stderr)

    def test_complete_cache_reuses_fast_path(self):
        p = self.run_provisioner(complete=True)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertNotIn('fetch:', p.stderr)

    def test_waiter_reuses_cache_completed_by_lock_owner(self):
        p = self.run_provisioner(completed_while_waiting=True)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertNotIn('fetch:', p.stderr)


if __name__ == '__main__':
    unittest.main()
