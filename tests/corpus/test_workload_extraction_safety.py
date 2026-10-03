"""Contract tests for tests/workloads/gzip-1.14/run.py's tar extraction.

That runner calls extractall on a tarball it downloaded over the network, so it
is the site in the tree where an escaping member matters most -- and it passed
`filter="data"` unconditionally, which is CPython 3.12+/3.11.4+ against the
Python 3.9+ floor docs/getting-started.md declares.  On 3.9-3.11.3 the workload
dies with a TypeError before extracting anything; the naive portability fix,
falling back to a bare extractall, would trade that for silently losing the
only check that refuses a symlink pointing outside the destination.

The runner is loaded straight off disk: the directory name carries a hyphen, so
it is not importable as a package, and importing anything else would test a
copy rather than the shipped code.
"""
from __future__ import annotations

import importlib.util
import io
import pathlib
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest import mock

REPO = pathlib.Path(__file__).resolve().parents[2]
RUNNER = REPO / 'tests' / 'workloads' / 'gzip-1.14' / 'run.py'

_spec = importlib.util.spec_from_file_location('gzip_workload_runner', RUNNER)
runner = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(runner)


class ExtractUntrustedTests(unittest.TestCase):
    """The guard must hold with and without tarfile's data filter."""

    def archive(self, extra):
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode='w:gz') as t:
            root = tarfile.TarInfo('gzip-1.14')
            root.type = tarfile.DIRTYPE
            root.mode = 0o755
            t.addfile(root)
            for member in extra:
                t.addfile(member)
        buf.seek(0)
        return tarfile.open(fileobj=buf, mode='r:gz')

    def sym(self, name, target):
        m = tarfile.TarInfo(name)
        m.type = tarfile.SYMTYPE
        m.linkname = target
        m.mode = 0o777
        return m

    def test_escaping_symlink_refused_with_the_data_filter(self):
        with tempfile.TemporaryDirectory() as dest:
            with self.assertRaises(ValueError) as caught:
                runner.extract_untrusted(
                    self.archive([self.sym('gzip-1.14/evil', '../../../../etc/passwd')]),
                    Path(dest))
        self.assertIn('escapes the destination', str(caught.exception))

    def test_escaping_symlink_refused_without_the_data_filter(self):
        # A pre-3.12 interpreter: extractall takes no `filter`, so the probe
        # selects the unfiltered call and the hand-applied rules must be what
        # refuses the member.
        real = tarfile.TarFile.extractall

        def legacy(self, path='.', members=None, *, numeric_owner=False):
            return real(self, path, members=members, numeric_owner=numeric_owner)

        with tempfile.TemporaryDirectory() as dest:
            with mock.patch.object(tarfile.TarFile, 'extractall', legacy):
                self.assertFalse(runner.extractall_takes_filter())
                with self.assertRaises(ValueError) as caught:
                    runner.extract_untrusted(
                        self.archive([self.sym('gzip-1.14/evil', '../../../../etc/passwd')]),
                        Path(dest))
        self.assertIn('escapes the destination', str(caught.exception))

    def test_absolute_link_target_refused(self):
        with tempfile.TemporaryDirectory() as dest:
            with self.assertRaises(ValueError):
                runner.extract_untrusted(
                    self.archive([self.sym('gzip-1.14/evil', '/etc/passwd')]), Path(dest))

    def test_escaping_member_name_refused(self):
        m = tarfile.TarInfo('../outside')
        m.type = tarfile.REGTYPE
        m.size = 0
        m.mode = 0o644
        with tempfile.TemporaryDirectory() as dest:
            with self.assertRaises(ValueError):
                runner.extract_untrusted(self.archive([m]), Path(dest))

    def test_special_mode_bits_refused(self):
        m = tarfile.TarInfo('gzip-1.14/x')
        m.type = tarfile.REGTYPE
        m.size = 0
        m.mode = 0o4755
        with tempfile.TemporaryDirectory() as dest:
            with self.assertRaises(ValueError) as caught:
                runner.extract_untrusted(self.archive([m]), Path(dest))
        self.assertIn('setuid/setgid/sticky', str(caught.exception))

    def test_an_unrelated_typeerror_is_not_swallowed_into_an_unfiltered_retry(self):
        # `except TypeError` around the filtered call would catch an error from
        # inside it and silently retry unfiltered.  The stub keeps `filter` in
        # its signature so the probe still reports support.
        real = tarfile.TarFile.extractall

        def filtered_call_fails(self, dest, members=None, *, filter=None, numeric_owner=False):
            if filter is not None:
                raise TypeError('unrelated internal failure')
            return real(self, dest, members=members, numeric_owner=numeric_owner)

        good = tarfile.TarInfo('gzip-1.14/ok.txt')
        good.type = tarfile.REGTYPE
        good.size = 0
        good.mode = 0o644
        with tempfile.TemporaryDirectory() as dest:
            with mock.patch.object(tarfile.TarFile, 'extractall', filtered_call_fails):
                with self.assertRaises(TypeError) as caught:
                    runner.extract_untrusted(self.archive([good]), Path(dest))
        self.assertIn('unrelated internal failure', str(caught.exception))

    def test_benign_archive_extracts_in_full(self):
        # The guard costs nothing on the shape the real archive has: measured
        # gzip-1.14.tar.xz has 525 members, all under gzip-1.14/, no links and
        # no special-bit modes.
        entries = []
        for name in ('gzip-1.14/src/a.c', 'gzip-1.14/src/sub/b.h'):
            m = tarfile.TarInfo(name)
            m.type = tarfile.REGTYPE
            m.size = 0
            m.mode = 0o644
            entries.append(m)
        d = tarfile.TarInfo('gzip-1.14/src/sub')
        d.type = tarfile.DIRTYPE
        d.mode = 0o755
        with tempfile.TemporaryDirectory() as dest:
            runner.extract_untrusted(self.archive([d] + entries), Path(dest))
            self.assertTrue((Path(dest) / 'gzip-1.14' / 'src' / 'a.c').is_file())
            self.assertTrue((Path(dest) / 'gzip-1.14' / 'src' / 'sub' / 'b.h').is_file())

    def test_in_tree_relative_symlink_still_extracted(self):
        # Strict about escaping targets, not about links as such.
        with tempfile.TemporaryDirectory() as dest:
            runner.extract_untrusted(
                self.archive([self.sym('gzip-1.14/link', 'README.md')]), Path(dest))
            self.assertTrue((Path(dest) / 'gzip-1.14' / 'link').is_symlink())

    def special(self, kind, mode=0o644, linkname=None):
        m = tarfile.TarInfo('gzip-1.14/x')
        m.type = kind
        m.mode = mode
        m.size = 0
        if linkname is not None:
            m.linkname = linkname
        if kind == tarfile.CHRTYPE:
            m.devmajor, m.devminor = 1, 3
        return m

    def test_fifo_member_refused_with_the_data_filter(self):
        # Measured: filter='data' rejects it with SpecialFileError, so the
        # hand-applied rules must reject it too or the fallback is weaker.
        with tempfile.TemporaryDirectory() as dest:
            with self.assertRaises(ValueError) as caught:
                runner.extract_untrusted(
                    self.archive([self.special(tarfile.FIFOTYPE)]), Path(dest))
        self.assertIn('unsupported entry type', str(caught.exception))

    def test_fifo_member_refused_without_the_data_filter(self):
        real = tarfile.TarFile.extractall

        def legacy(self, path='.', members=None, *, numeric_owner=False):
            return real(self, path, members=members, numeric_owner=numeric_owner)

        with tempfile.TemporaryDirectory() as dest:
            with mock.patch.object(tarfile.TarFile, 'extractall', legacy):
                with self.assertRaises(ValueError) as caught:
                    runner.extract_untrusted(
                        self.archive([self.special(tarfile.FIFOTYPE)]), Path(dest))
        self.assertIn('unsupported entry type', str(caught.exception))
        # An unfiltered extractall would otherwise have CREATED the fifo.
        self.assertFalse((Path(dest) / 'gzip-1.14' / 'x').exists())

    def test_device_members_refused(self):
        for kind in (tarfile.CHRTYPE, tarfile.BLKTYPE):
            with self.subTest(kind=kind):
                with tempfile.TemporaryDirectory() as dest:
                    with self.assertRaises(ValueError):
                        runner.extract_untrusted(self.archive([self.special(kind)]), Path(dest))

    def test_hardlink_refused(self):
        # Stricter than the data filter, which permits an in-tree hardlink.
        with tempfile.TemporaryDirectory() as dest:
            with self.assertRaises(ValueError) as caught:
                runner.extract_untrusted(
                    self.archive([self.special(tarfile.LNKTYPE, 0o644, 'gzip-1.14/other')]),
                    Path(dest))
        self.assertIn('unsupported entry type', str(caught.exception))


class CallSiteWiringTests(unittest.TestCase):
    """The guard is worthless unless the runner actually calls it.

    The tests above exercise extract_untrusted directly, so they stayed green
    when the call site was reverted to a bare `extractall(root, filter="data")`
    -- verified: 8/8 OK with the guard unreferenced.  These pin the wiring.
    """

    def ast_of(self, path):
        import ast
        return ast.parse(path.read_text(), str(path.relative_to(REPO)))

    def enclosing_functions(self, tree):
        """Map every call node to the name of the function it sits in."""
        import ast
        owner = {}
        for fn in ast.walk(tree):
            if isinstance(fn, (ast.FunctionDef, ast.AsyncFunctionDef)):
                for node in ast.walk(fn):
                    owner.setdefault(id(node), fn.name)
        return owner

    def test_every_extractall_in_the_runner_is_inside_the_guard(self):
        import ast
        tree = self.ast_of(RUNNER)
        owner = self.enclosing_functions(tree)
        calls = [n for n in ast.walk(tree)
                 if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute)
                 and n.func.attr == 'extractall']
        self.assertTrue(calls, 'the runner no longer extracts anything; rewrite this test')
        for call in calls:
            self.assertEqual(owner.get(id(call)), 'extract_untrusted',
                             'extractall must only be reached through extract_untrusted')

    def test_main_routes_extraction_through_the_guard(self):
        import ast
        tree = self.ast_of(RUNNER)
        main = next(n for n in tree.body
                    if isinstance(n, ast.FunctionDef) and n.name == 'main')
        called = {n.func.id for n in ast.walk(main)
                  if isinstance(n, ast.Call) and isinstance(n.func, ast.Name)}
        self.assertIn('extract_untrusted', called)

    def test_no_unconditional_data_filter_anywhere_in_the_tree(self):
        # The defect class is `extractall(..., filter=...)` passed without a
        # version check: 3.12+/3.11.4+ against the declared 3.9+ floor.  Both
        # remaining sites now sit behind extractall_takes_filter(); a third
        # site must fail here rather than in a workload run on an old Python.
        import ast
        guarded = {'extract_untrusted', 'extract'}
        offenders = []
        for path in sorted(REPO.rglob('*.py')):
            rel = path.relative_to(REPO).as_posix()
            if rel.startswith('tests/corpus/clang-c/') or '/clang-c/' in rel:
                continue
            try:
                tree = ast.parse(path.read_text(), rel)
            except SyntaxError:
                continue
            owner = self.enclosing_functions(tree)
            for node in ast.walk(tree):
                if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)):
                    continue
                if node.func.attr != 'extractall':
                    continue
                if not any(kw.arg == 'filter' for kw in node.keywords):
                    continue
                if owner.get(id(node)) not in guarded:
                    offenders.append(f'{rel}:{node.lineno}')
        self.assertEqual(offenders, [],
                         'these pass filter= to extractall without a version '
                         'probe; route them through a guarded helper: ' + ', '.join(offenders))


if __name__ == '__main__':
    unittest.main()
