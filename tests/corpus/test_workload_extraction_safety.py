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

import contextlib
import hashlib
import importlib.util
import io
import os
import pathlib
import stat
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

# The sibling helper, loaded the same way, so the two extraction policies can be
# compared against each other rather than only against their own expectations.
_rspec = importlib.util.spec_from_file_location(
    'recover_contract_parity', REPO / 'scripts' / 'lccc_recover.py')
recovery = importlib.util.module_from_spec(_rspec)
_rspec.loader.exec_module(recovery)


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

    def test_escaping_symlink_refused_by_the_member_rules(self):
        with tempfile.TemporaryDirectory() as dest:
            with self.assertRaises(ValueError) as caught:
                runner.extract_untrusted(
                    self.archive([self.sym('gzip-1.14/evil', '../../../../etc/passwd')]),
                    Path(dest))
        self.assertIn('escapes the destination', str(caught.exception))

    def test_escaping_symlink_refused_on_the_legacy_path(self):
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

    def test_fifo_member_refused_by_the_member_rules(self):
        # Measured: filter='data' rejects it with SpecialFileError, so the
        # hand-applied rules must reject it too or the fallback is weaker.
        with tempfile.TemporaryDirectory() as dest:
            with self.assertRaises(ValueError) as caught:
                runner.extract_untrusted(
                    self.archive([self.special(tarfile.FIFOTYPE)]), Path(dest))
        self.assertIn('unsupported entry type', str(caught.exception))

    def test_fifo_member_refused_on_the_legacy_path(self):
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


def _build(root, extra):
    """tar.gz bytes: one dir member for `root`, then `extra`."""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode='w:gz') as t:
        head = tarfile.TarInfo(root)
        head.type = tarfile.DIRTYPE
        head.mode = 0o755
        t.addfile(head)
        for member in extra:
            t.addfile(member)
    return buf.getvalue()


def _open(data):
    return tarfile.open(fileobj=io.BytesIO(data), mode='r:gz')


def _reg(root, name, mode=0o644, uid=0, gid=0, uname='root', gname='root'):
    m = tarfile.TarInfo(f'{root}/{name}')
    m.type = tarfile.REGTYPE
    m.size = 0
    m.mode = mode
    m.uid, m.gid, m.uname, m.gname = uid, gid, uname, gname
    return m


@contextlib.contextmanager
def pre_pep706(module):
    """Emulate CPython 3.9-3.11.3 for `module`: no filter keyword, metadata applied.

    Disabling the probe on its own is not enough.  On 3.14 a bare extractall
    already defaults to the `data` filter, so a "legacy" run there would
    silently get the very protection these tests exist to show is absent;
    forcing fully_trusted pins pre-3.12 semantics on every interpreter.
    """
    real = tarfile.TarFile.extractall

    def legacy(self, path='.', members=None, **kw):
        kw.pop('filter', None)
        return real(self, path, members=members, filter='fully_trusted')

    with mock.patch.object(module, 'extractall_takes_filter', lambda: False), \
            mock.patch.object(tarfile.TarFile, 'extractall', legacy):
        yield


def _snapshot(root):
    """Everything an archive could have planted: type, mode, owner, link target."""
    out = {}
    for p in sorted(Path(root).rglob('*')):
        st = p.lstat()
        kind = ('link' if stat.S_ISLNK(st.st_mode) else
                'file' if stat.S_ISREG(st.st_mode) else
                'dir' if stat.S_ISDIR(st.st_mode) else 'other')
        out[p.relative_to(root).as_posix()] = (
            kind, st.st_mode & 0o7777, st.st_uid, st.st_gid,
            os.readlink(p) if kind == 'link' else '')
    return out


class DataFilterKeywordTests(unittest.TestCase):
    """Assert which code path ran, instead of inferring it from a rejection.

    Every ValueError asserted in ExtractUntrustedTests is raised by the
    hand-applied rules BEFORE extractall is reached, so those tests stay green
    with the filter forced off -- measured: patching extractall_takes_filter to
    return False leaves the escaping-symlink test passing.  A rejection
    therefore evidences the member rules, not PEP 706, and the suite had
    nothing that observed the filter actually being used.  These do.
    """

    def test_supported_path_passes_filter_data(self):
        seen = {}
        real = tarfile.TarFile.extractall

        # `filter` must stay in the signature: extractall_takes_filter probes
        # inspect.signature(tarfile.TarFile.extractall) at call time, so a
        # `**kw` wrapper reads as unsupported and silently flips the helper to
        # the legacy branch.  Verified -- a `**kw` spy made this test observe
        # ABSENT.  The probe reads the live object rather than a cached flag on
        # purpose, so a patched extractall is a patched probe.
        def spy(self, path='.', members=None, *, filter=None, **kw):
            seen['filter'] = filter if filter is not None else 'ABSENT'
            return real(self, path, members=members, filter=filter)

        data = _build('gzip-1.14', [_reg('gzip-1.14', 'ok.txt')])
        with tempfile.TemporaryDirectory() as dest:
            with mock.patch.object(tarfile.TarFile, 'extractall', spy):
                runner.extract_untrusted(_open(data), Path(dest))
        self.assertEqual(seen.get('filter'), 'data')

    def test_legacy_path_never_passes_a_filter_keyword(self):
        seen = {}
        real = tarfile.TarFile.extractall

        def spy(self, path='.', members=None, **kw):
            seen['filter'] = kw.get('filter', 'ABSENT')
            return real(self, path, members=members, filter='fully_trusted')

        data = _build('gzip-1.14', [_reg('gzip-1.14', 'ok.txt')])
        with tempfile.TemporaryDirectory() as dest:
            with mock.patch.object(runner, 'extractall_takes_filter', lambda: False), \
                    mock.patch.object(tarfile.TarFile, 'extractall', spy):
                runner.extract_untrusted(_open(data), Path(dest))
        self.assertEqual(seen['filter'], 'ABSENT')

    def test_legacy_path_still_neutralizes_what_extractall_would_apply(self):
        # The keyword is absent, so the guard has to come from the helper.
        seen = {}
        real = tarfile.TarFile.extractall

        def spy(self, path='.', members=None, *, filter=None, **kw):
            seen['members'] = [(m.mode, m.uid, m.gid, m.uname, m.gname)
                               for m in (members if members is not None else self.getmembers())]
            return real(self, path, members=members, filter='fully_trusted')

        data = _build('gzip-1.14', [_reg('gzip-1.14', 'x', mode=0o777,
                                         uid=1234, gid=4321, uname='mallory', gname='mallory')])
        with tempfile.TemporaryDirectory() as dest:
            with mock.patch.object(runner, 'extractall_takes_filter', lambda: False), \
                    mock.patch.object(tarfile.TarFile, 'extractall', spy):
                runner.extract_untrusted(_open(data), Path(dest))
        self.assertTrue(seen['members'], 'the guard never reached extractall')
        for mode, uid, gid, uname, gname in seen['members']:
            self.assertIsNone(uid)
            self.assertIsNone(gid)
            self.assertIsNone(uname)
            self.assertIsNone(gname)
            # None means "leave the mode to the OS", which is what the data
            # filter does for directories and symlinks.
            self.assertTrue(mode is None or not mode & 0o022,
                            f'group/other write survived: {mode!r}')


class LegacyMetadataTests(unittest.TestCase):
    """The pre-3.12 path must neutralize ownership and permissions.

    An unfiltered extractall chowns and chmods with the archive's own values,
    and this tarball arrives over the network, so those are attacker-controlled
    inputs rather than trusted metadata.
    """

    def test_benign_archive_extracts_in_full_on_the_legacy_path(self):
        sub = tarfile.TarInfo('gzip-1.14/src/sub')
        sub.type = tarfile.DIRTYPE
        sub.mode = 0o755
        entries = [sub, _reg('gzip-1.14', 'src/a.c'), _reg('gzip-1.14', 'src/sub/b.h')]
        with tempfile.TemporaryDirectory() as dest:
            with pre_pep706(runner):
                runner.extract_untrusted(_open(_build('gzip-1.14', entries)), Path(dest))
            self.assertTrue((Path(dest) / 'gzip-1.14' / 'src' / 'a.c').is_file())
            self.assertTrue((Path(dest) / 'gzip-1.14' / 'src' / 'sub' / 'b.h').is_file())

    def test_legacy_path_matches_the_data_filter_attribute_for_attribute(self):
        # The strongest form of the claim: same archive through both paths,
        # identical type/mode/owner for every member.  It derives its
        # expectation from the real filter instead of a hand-written table, so
        # it cannot drift when CPython's rules do.
        sub = tarfile.TarInfo('gzip-1.14/sub')
        sub.type = tarfile.DIRTYPE
        sub.mode = 0o707
        entries = [sub,
                   _reg('gzip-1.14', 'a', mode=0o777),
                   _reg('gzip-1.14', 'b', mode=0o600),
                   _reg('gzip-1.14', 'c', mode=0o000),
                   _reg('gzip-1.14', 'd', mode=0o4755, uid=1234, gid=4321)]
        # 0o4755 is refused by the member loop before extraction, so build the
        # setuid case only for the neutralizer comparison below.
        entries = entries[:-1]
        with tempfile.TemporaryDirectory() as filtered, \
                tempfile.TemporaryDirectory() as legacy_dir:
            runner.extract_untrusted(_open(_build('gzip-1.14', entries)), Path(filtered))
            with pre_pep706(runner):
                runner.extract_untrusted(_open(_build('gzip-1.14', entries)), Path(legacy_dir))
            self.assertEqual(_snapshot(filtered), _snapshot(legacy_dir))

    def test_legacy_path_clamps_group_and_other_write(self):
        with tempfile.TemporaryDirectory() as dest:
            with pre_pep706(runner):
                runner.extract_untrusted(
                    _open(_build('gzip-1.14', [_reg('gzip-1.14', 'x', mode=0o777)])), Path(dest))
            got = (Path(dest) / 'gzip-1.14' / 'x').stat().st_mode & 0o7777
        self.assertEqual(got, 0o755)

    def test_legacy_path_forces_owner_readwrite(self):
        with tempfile.TemporaryDirectory() as dest:
            with pre_pep706(runner):
                runner.extract_untrusted(
                    _open(_build('gzip-1.14', [_reg('gzip-1.14', 'x', mode=0o000)])), Path(dest))
            got = (Path(dest) / 'gzip-1.14' / 'x').stat().st_mode & 0o7777
        self.assertEqual(got, 0o600)

    def test_rejected_archive_creates_nothing_outside_the_destination(self):
        def sym(name, target):
            m = tarfile.TarInfo(f'gzip-1.14/{name}')
            m.type = tarfile.SYMTYPE
            m.linkname = target
            m.mode = 0o777
            return m

        hostile = [('escaping symlink', sym('evil', '../../../../tmp/pwned-by-archive')),
                   ('absolute link', sym('abs', '/tmp/pwned-by-archive'))]
        for label, member in hostile:
            with self.subTest(case=label):
                with tempfile.TemporaryDirectory() as dest:
                    with self.assertRaises(ValueError):
                        runner.extract_untrusted(_open(_build('gzip-1.14', [member])), Path(dest))
                    self.assertEqual(_snapshot(dest), {})
                    self.assertFalse(Path('/tmp/pwned-by-archive').exists())


class NeutralizerParityTests(unittest.TestCase):
    """_neutralize_archive_attrs must BE the data filter's metadata half."""

    def test_matches_tarfile_data_filter_over_the_mode_matrix(self):
        if not hasattr(tarfile, 'data_filter'):
            self.skipTest('tarfile.data_filter unavailable on this interpreter')
        import copy
        kinds = ((tarfile.REGTYPE, 'reg'), (tarfile.DIRTYPE, 'dir'), (tarfile.SYMTYPE, 'sym'))
        modes = (0o777, 0o755, 0o644, 0o600, 0o400, 0o000, 0o4755, 0o2755, 0o1777, 0o707)
        for mode in modes:
            for kind, label in kinds:
                with self.subTest(mode=oct(mode), type=label):
                    base = tarfile.TarInfo('gzip-1.14/x')
                    base.type = kind
                    base.mode = mode
                    base.uid, base.gid = 1234, 4321
                    base.uname, base.gname = 'mallory', 'mallory'
                    if kind == tarfile.SYMTYPE:
                        base.linkname = 'target'
                    ref = tarfile.data_filter(copy.deepcopy(base), '/tmp/dest')
                    got = runner._neutralize_archive_attrs(copy.deepcopy(base))
                    self.assertEqual(
                        (got.mode, got.uid, got.gid, got.uname, got.gname),
                        (ref.mode, ref.uid, ref.gid, ref.uname, ref.gname))


class HelperParityTests(unittest.TestCase):
    """The two extraction policies must not drift apart.

    They are mirrored copies on purpose -- tests/workloads/ imports nothing from
    scripts/, and check_script_imports.py resolves imports against real modules
    -- so nothing but a test stops one helper gaining a rule the other lacks.
    Each is fed the same hostile member under its own archive root and both
    must reach the same verdict.
    """

    CASES = ('escaping symlink', 'absolute link target', 'fifo', 'char device',
             'hardlink', 'setuid mode', 'escaping member name', 'backslash name',
             'benign file')

    def members(self, root, case):
        def mk(name, kind, mode=0o644, linkname=None):
            m = tarfile.TarInfo(f'{root}/{name}')
            m.type = kind
            m.mode = mode
            m.size = 0
            if linkname is not None:
                m.linkname = linkname
            if kind == tarfile.CHRTYPE:
                m.devmajor, m.devminor = 1, 3
            return m

        table = {
            'escaping symlink': mk('evil', tarfile.SYMTYPE, 0o777, '../../../../etc/passwd'),
            'absolute link target': mk('abs', tarfile.SYMTYPE, 0o777, '/etc/passwd'),
            'fifo': mk('f', tarfile.FIFOTYPE),
            'char device': mk('c', tarfile.CHRTYPE),
            'hardlink': mk('h', tarfile.LNKTYPE, 0o644, f'{root}/other'),
            'setuid mode': mk('s', tarfile.REGTYPE, 0o4755),
            'escaping member name': mk('../escape', tarfile.REGTYPE),
            'backslash name': mk('..\\..\\escape', tarfile.REGTYPE),
            'benign file': mk('ok.txt', tarfile.REGTYPE),
        }
        return [table[case]]

    def gzip_verdict(self, root, case, dest):
        try:
            runner.extract_untrusted(_open(_build(root, self.members(root, case))), Path(dest))
            return 'accept'
        except ValueError:
            return 'reject'

    def recovery_verdict(self, root, case, dest):
        data = _build(root, self.members(root, case))
        archive = Path(dest) / 'src.tar.gz'
        archive.write_bytes(data)
        stage = Path(dest) / 'stage'
        stage.mkdir()
        try:
            recovery.extract(archive, stage,
                             {'archive_sha256': hashlib.sha256(data).hexdigest()})
            return 'accept'
        except ValueError:
            return 'reject'

    def test_both_helpers_reach_the_same_verdict(self):
        for case in self.CASES:
            with self.subTest(case=case):
                with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
                    left = self.gzip_verdict('gzip-1.14', case, a)
                    right = self.recovery_verdict('lccc', case, b)
                self.assertEqual(left, right)
                if case == 'benign file':
                    self.assertEqual(left, 'accept', 'a benign archive must still extract')
                else:
                    self.assertEqual(left, 'reject', f'{case} must be refused')


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


class FailedCommandEvidenceTests(unittest.TestCase):
    def test_failed_build_retains_stdout_and_stderr(self):
        from subprocess import CompletedProcess
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / 'build.log'
            result = CompletedProcess(['make'], 2, 'compile context\n', 'link error\n')
            with mock.patch.object(runner.subprocess, 'run', return_value=result):
                with self.assertRaisesRegex(RuntimeError, 'command failed'):
                    runner.run(['make'], log=log)
            self.assertIn('compile context', log.read_text())
            self.assertIn('link error', log.read_text())


if __name__ == '__main__':
    unittest.main()
