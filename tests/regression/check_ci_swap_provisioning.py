#!/usr/bin/env python3
"""Swap helper contracts with no privileged commands or /proc writes.

The temporary script differs only at the EUID branch condition. Bash's EUID
is readonly; substituting that one expression tests both branches without
adding a production override that could falsely certify active swap.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
BASH = shutil.which("bash")
MOCK = r'''
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['MOCK_LOG'], 'a') as f:
    f.write(json.dumps([name, *args]) + '\n')
if os.environ.get('FAIL_COMMAND') == name: sys.exit(9)
state = pathlib.Path(os.environ['MOCK_STATE'])
if name == 'sudo':
    assert args.pop(0) == '--'
    os.execvp(args[0], args)
elif name == 'uname': print(os.environ.get('MOCK_OS', 'Linux'))
elif name == 'awk':
    assert args[0] == '-v' and args[1].startswith('path=') and args[-1] == '/proc/swaps'
    sys.exit(0 if state.exists() else 1)
elif name == 'swapon':
    if args == ['--show']: print('mock active swap')
    elif os.environ.get('NO_ACTIVATE') != '1': state.touch()
'''


class SwapProvisioningTests(unittest.TestCase):
    def invoke(self, *, uid=0, active=False, stale=False, repeats=1, missing=(), **overrides):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            bin_dir = directory / 'bin'
            bin_dir.mkdir()
            mock = bin_dir / 'mock'
            mock.write_text(f'#!{sys.executable}\n' + MOCK)
            mock.chmod(0o755)
            for command in ('uname', 'awk', 'sudo', 'rm', 'fallocate', 'chown',
                            'chmod', 'mkswap', 'swapon', 'free'):
                if command not in missing:
                    (bin_dir / command).symlink_to(mock)
            swap_dir = directory / 'runner temp with spaces'
            swap_dir.mkdir()
            swap_file = swap_dir / 'lccc-ci.swap'
            if stale:
                swap_file.touch()
            state = directory / 'active'
            if active:
                state.touch()
            text = (ROOT / 'scripts/ci_enable_swap.sh').read_text()
            self.assertEqual(text.count('(( EUID == 0 ))'), 1)
            script = directory / 'helper.sh'
            script.write_text(text.replace('(( EUID == 0 ))', '(( MOCK_EUID == 0 ))'))
            log = directory / 'log'
            env = {**os.environ, 'PATH': str(bin_dir), 'MOCK_LOG': str(log),
                   'MOCK_STATE': str(state), 'MOCK_EUID': str(uid),
                   'RUNNER_TEMP': str(swap_dir), 'LCCC_CI_SWAP_SIZE': '4G', **overrides}
            results = [subprocess.run([BASH, str(script)], env=env, text=True,
                                      capture_output=True, timeout=15) for _ in range(repeats)]
            commands = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            return results, commands, str(swap_file)

    def test_root_sequence_and_fixed_quoted_path(self):
        results, commands, path = self.invoke(stale=True)
        self.assertEqual(results[0].returncode, 0, results[0].stderr)
        self.assertFalse(any(c[0] == 'sudo' for c in commands))
        self.assertEqual([c for c in commands if c[0] not in ('uname', 'awk', 'free')], [
            ['rm', '-f', '--', path], ['fallocate', '-l', '4G', path],
            ['chown', 'root:root', path], ['chmod', '600', path],
            ['mkswap', path], ['swapon', path], ['swapon', '--show'],
        ])

    def test_nonroot_uses_sudo_for_every_privileged_command(self):
        results, commands, _ = self.invoke(uid=1000, stale=True)
        self.assertEqual(results[0].returncode, 0, results[0].stderr)
        privileged = [c for c in commands if c[0] in ('rm', 'fallocate', 'chown', 'chmod', 'mkswap', 'swapon')]
        self.assertEqual([c[2:] for c in commands if c[0] == 'sudo'], privileged)
        self.assertTrue(all(c[1] == '--' for c in commands if c[0] == 'sudo'))

    def test_second_invocation_does_not_reallocate_or_activate(self):
        results, commands, path = self.invoke(repeats=2)
        self.assertTrue(all(r.returncode == 0 for r in results))
        self.assertEqual(commands.count(['fallocate', '-l', '4G', path]), 1)
        self.assertEqual(commands.count(['swapon', path]), 1)
        self.assertEqual(commands.count(['swapon', '--show']), 2)

    def test_already_active_swap_never_removes_stale_file(self):
        results, commands, _ = self.invoke(active=True, stale=True)
        self.assertEqual(results[0].returncode, 0, results[0].stderr)
        self.assertFalse(any(c[0] in ('rm', 'fallocate', 'chown', 'chmod', 'mkswap') for c in commands))
        self.assertEqual([c for c in commands if c[0] == 'swapon'], [['swapon', '--show']])

    def test_command_failures_propagate(self):
        for command in ('sudo', 'rm', 'fallocate', 'chown', 'chmod', 'mkswap', 'swapon', 'free'):
            with self.subTest(command=command):
                results, _, _ = self.invoke(uid=1000, stale=True, FAIL_COMMAND=command)
                self.assertNotEqual(results[0].returncode, 0)

    def test_activation_requires_independent_proc_check(self):
        results, _, _ = self.invoke(NO_ACTIVATE='1')
        self.assertNotEqual(results[0].returncode, 0)
        self.assertIn('swap file is not active', results[0].stderr)
        self.assertNotIn('Active CI swap file:', results[0].stdout)

    def test_missing_tools_and_nonlinux_fail_before_provisioning(self):
        for options, message in (({'uid': 1000, 'missing': ('sudo',)}, 'sudo is required'),
                                 ({'missing': ('fallocate',)}, 'fallocate is required'),
                                 ({'MOCK_OS': 'Darwin'}, 'Linux is required')):
            with self.subTest(options=options):
                results, commands, _ = self.invoke(**options)
                self.assertNotEqual(results[0].returncode, 0)
                self.assertIn(message, results[0].stderr)
                self.assertFalse(any(c[0] in ('rm', 'fallocate', 'mkswap', 'swapon') for c in commands))


if __name__ == '__main__':
    unittest.main()
