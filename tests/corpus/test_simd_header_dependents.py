"""`gen_lcccsimd.py --check` answers for the tables recorded from the header.

`include/lcccsimd.h` is generated, and three tables are recorded from it:
the public API inventory, the feature permissions of the helpers, and the
intrinsic census.  Each has a gate in a lane of its own.  A header that
gains names and leaves a table as it was passed `--check` and failed two
of those lanes; the check now runs the two that need no compiler.

The tests run the generator on the tree as it is, and on a copy of what it
reads with one table made stale at a time.
"""
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GENERATOR = 'scripts/gen_lcccsimd.py'
INVENTORY = 'engineering/isa/sse-avx-public-api.json'
PERMISSIONS = 'tests/simd/isa_permissions_gcc16.json'
# Everything the generator's check and the two checks behind it read.
READ = (
    GENERATOR,
    'scripts/audit_sse_avx_public_api.py',
    'tests/simd/check_isa_permissions.py',
    'tests/simd/probe_utils.py',
    INVENTORY,
    'engineering/isa/gcc16-sse-avx-reference.json',
    PERMISSIONS,
    'src/frontend/sema/builtins.rs',
    'src/ir/lowering/expr_builtins_lcccsimd.rs',
)


def check(root, *arguments):
    return subprocess.run(
        [sys.executable, str(root / GENERATOR), '--check', *arguments],
        cwd=root, capture_output=True, text=True, check=False, timeout=120,
    )


class HeaderDependents(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(prefix='lccc-simd-dependents-')
        cls.copy = Path(cls.directory.name)
        for name in READ:
            target = cls.copy / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        shutil.copytree(ROOT / 'include', cls.copy / 'include')

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def stale(self, table, change, *arguments):
        """Run the check with `table` changed by `change`, then put it back."""
        path = self.copy / table
        kept = path.read_text()
        data = json.loads(kept)
        change(data)
        path.write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')
        try:
            return check(self.copy, *arguments)
        finally:
            path.write_text(kept)

    def test_the_tree_agrees_with_its_header(self):
        run = check(ROOT)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn('matches the generator', run.stdout)
        self.assertIn('the 2 tables recorded from it that need no compiler agree with it', run.stdout)

    def test_the_copy_is_complete(self):
        # The control of the two tests below: unchanged, the copy passes, so
        # a failure there is the change and not a file the copy lacks.
        run = check(self.copy)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn('the 2 tables recorded from it', run.stdout)

    def test_a_stale_inventory_fails_and_is_named(self):
        def drop_a_name(data):
            # What a header with one more name than the inventory looks like.
            row = next(r for r in data['apis'] if r['name'] == '_mm512_mask_cmplt_epi32_mask')
            data['apis'].remove(row)
            data['declarations'] -= 1
            data['unique_names'] -= 1

        run = self.stale(INVENTORY, drop_a_name)
        self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
        self.assertIn(f'{INVENTORY} is recorded from the header and fails its check', run.stderr)
        self.assertIn('python3 scripts/audit_sse_avx_public_api.py --write', run.stderr)
        self.assertNotIn(PERMISSIONS, run.stderr)

    def test_stale_permissions_fail_and_are_named(self):
        def earlier_representative(data):
            # The table names one intrinsic per helper, the last the header
            # defines; the masked twins moved it for every `vpcmp`.
            self.assertEqual(data['rows']['vpcmpd512']['intrinsic'], '_mm512_mask_cmp_epi32_mask')
            data['rows']['vpcmpd512']['intrinsic'] = '_mm512_cmp_epi32_mask'

        run = self.stale(PERMISSIONS, earlier_representative)
        self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
        self.assertIn(f'{PERMISSIONS} is recorded from the header and fails its check', run.stderr)
        self.assertIn('vpcmpd512', run.stderr)
        self.assertIn('tests/simd/gen_isa_permissions.py --headers', run.stderr)
        self.assertNotIn(f'{INVENTORY} is recorded', run.stderr)

    def test_a_header_that_differs_is_reported_before_any_table(self):
        path = self.copy / 'include' / 'lcccsimd.h'
        kept = path.read_text()
        path.write_text(kept + '/* edited by hand */\n')
        try:
            run = check(self.copy)
        finally:
            path.write_text(kept)
        self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
        self.assertIn('does not match the generator', run.stderr)
        self.assertNotIn('is recorded from the header', run.stderr)

    def test_a_header_written_elsewhere_has_no_tables(self):
        # `--out` compares a header at another path; the tables belong to the
        # repository's own, and a stale one is not that header's business.
        elsewhere = self.copy / 'elsewhere.h'
        shutil.copyfile(self.copy / 'include' / 'lcccsimd.h', elsewhere)

        def emptied(data):
            data['declarations'] = 0

        self.assertEqual(self.stale(INVENTORY, emptied).returncode, 1)
        run = self.stale(INVENTORY, emptied, '--out', str(elsewhere))
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn('matches the generator', run.stdout)
        self.assertNotIn('tables recorded from it', run.stdout)


if __name__ == '__main__':
    unittest.main()
