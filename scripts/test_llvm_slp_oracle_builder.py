#!/usr/bin/env python3
"""Control-flow/cache tests with mocked tools; never claims to build LLVM."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

BUILDER = Path(__file__).with_name('ci_build_llvm_slp_oracles.sh')
MOCK = r'''#!/usr/bin/python3
import json, os, sys
from pathlib import Path
name=Path(sys.argv[0]).name
a=sys.argv[1:]
with open(os.environ['MOCK_CALLS'], 'a') as f:
    f.write(json.dumps([name, *a])+'\n')
if name=='awk': print(4194304)
elif name in ('gcc-16','g++-16'):
    print(os.environ.get('MOCK_GCC_VERSION','16.2.0') if a==['-dumpfullversion'] else 'mock GCC 16.2.0')
elif name=='git':
    if a[0]=='init': (Path(a[1])/'.git').mkdir(parents=True, exist_ok=True)
    elif a[0]=='-C':
        root=Path(a[1]); a=a[2:]
        if a[:2]==['remote','get-url']: print('https://github.com/llvm/llvm-project.git')
        elif a[:2]==['checkout','--detach']: (root/'.mock-ref').write_text(a[2])
        elif a==['rev-parse','HEAD']: print((root/'.mock-ref').read_text())
elif name=='cmake' and a[0]=='--build':
    root=Path(a[1]); (root/'bin').mkdir(parents=True, exist_ok=True)
    resource=root/'lib/clang/mock/include'; resource.mkdir(parents=True,exist_ok=True)
    (resource/'stddef.h').write_text('/* mock header, not a real compiler */\n')
    p=root/'bin/clang'
    p.write_text('#!/bin/sh\nif [ "$1" = -print-resource-dir ]; then echo "'+str(resource.parent)+'"; else echo "mock Clang control"; fi\n')
    p.chmod(0o755)
'''


class BuilderControls(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        tools = self.root / 'bin'
        tools.mkdir()
        for name in ('awk', 'gcc-16', 'g++-16', 'git', 'cmake'):
            p = tools / name
            p.write_text(MOCK)
            p.chmod(0o755)
        self.prefix = self.root / 'oracles'
        self.log = self.root / 'calls.jsonl'
        self.env = dict(os.environ, PATH=f'{tools}:{os.environ["PATH"]}',
                        MOCK_CALLS=str(self.log))

    def run_builder(self):
        return subprocess.run(['/bin/bash', str(BUILDER), str(self.prefix),
                               str(self.root / 'work')], env=self.env,
                              capture_output=True, text=True, timeout=30)

    def test_two_pinned_serial_builds_and_verified_cache_hit(self):
        result = self.run_builder()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = [json.loads(x) for x in self.log.read_text().splitlines()]
        builds = [x for x in calls if x[:2] == ['cmake', '--build']]
        self.assertEqual(len(builds), 2)
        for build in builds:
            self.assertEqual(build[-2:], ['--parallel', '2'])
            self.assertIn('clang-resource-headers', build)
        configs = [x for x in calls if x[:2] == ['cmake', '-S']]
        self.assertEqual(len(configs), 2)
        for config in configs:
            self.assertIn('-DLLVM_ENABLE_WERROR=ON', config)
            self.assertIn('-DCMAKE_BUILD_TYPE=MinSizeRel', config)
        manifest = json.loads((self.prefix / 'build.json').read_text())
        self.assertEqual(manifest['base'], 'a1d36a0e01751ae7d8e427f667e0a62d2d59e9c5')
        self.assertEqual(manifest['head'], 'a8eea1784d01b2a54b6383a8f7cf87507ce0e4e1')
        before = self.log.read_text()
        result = self.run_builder()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(before, self.log.read_text(), 'cache hit must not rebuild')

    def test_corrupt_binary_is_not_a_cache_hit(self):
        self.assertEqual(self.run_builder().returncode, 0)
        (self.prefix / 'clang-base').write_text('corrupt')
        self.assertNotEqual(self.run_builder().returncode, 0)

    def test_wrong_companion_version_fails_before_source_fetch(self):
        self.env['MOCK_GCC_VERSION'] = '16.0.1'
        self.assertNotEqual(self.run_builder().returncode, 0)
        self.assertNotIn('git', self.log.read_text())


if __name__ == '__main__':
    unittest.main()
