#!/usr/bin/env python3
"""Compact recovery in actual miniature Git repositories; no compiler builds."""
import base64
import hashlib
import importlib.util
import inspect
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from unittest import mock

REPO=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('recover_contract',REPO/'scripts/lccc_recover.py')
recovery=importlib.util.module_from_spec(spec);spec.loader.exec_module(recovery)
bs=importlib.util.spec_from_file_location('bundle_contract',REPO/'scripts/session_bundle.py')
bundle=importlib.util.module_from_spec(bs);bs.loader.exec_module(bundle)


def git(repo,*args):return subprocess.check_output(['git','-C',str(repo),*args],stderr=subprocess.DEVNULL).decode().strip()


class RecoveryContract(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
        self.repo=self.root/'original';self.repo.mkdir();self.art=self.root/'artifacts';self.art.mkdir()
        git(self.repo,'init','-q');git(self.repo,'config','user.name','test');git(self.repo,'config','user.email','test@example.invalid')
        git(self.repo,'checkout','-qb','arena/synthetic')
        (self.repo/'a.txt').write_text('base\n');(self.repo/'b.txt').write_text('unchanged\n');git(self.repo,'add','.');git(self.repo,'commit','-qm','base')
        self.base=git(self.repo,'rev-parse','HEAD')
        (self.repo/'a.txt').write_text('new\n');(self.repo/'c.txt').write_text('added\n');git(self.repo,'add','.');git(self.repo,'commit','-qm','session')
        self.head=git(self.repo,'rev-parse','HEAD');self.tree=git(self.repo,'rev-parse','HEAD^{tree}')
        meta=bundle.build(self.repo,self.base,self.art/'lccc-session.bundle');(self.art/'SESSION_BUNDLE.json').write_text(json.dumps(meta))
        patch=subprocess.check_output(['git','-C',str(self.repo),'diff','--binary',self.base,self.head]);(self.root/'ms178-1.patch').write_bytes(patch)
        archive=subprocess.check_output(['git','-C',str(self.repo),'archive','--format=tar','--prefix=lccc/',self.head])
        import gzip
        (self.art/'lccc-src.tar.gz').write_bytes(gzip.compress(archive,mtime=0))
        raw=subprocess.check_output(['git','-C',str(self.repo),'cat-file','commit',self.base])
        self.recipe=dict(format='lccc-source-archive-v2',archive='lccc-src.tar.gz',archive_sha256=recovery.sha(self.art/'lccc-src.tar.gz'),
                         patch='ms178-1.patch',patch_sha256=recovery.sha(self.root/'ms178-1.patch'),prefix='lccc/',
                         head=self.head,tree=self.tree,branch='arena/synthetic',base=self.base,base_commit_base64=base64.b64encode(raw).decode())
        self.write_recipe()
    def write_recipe(self):(self.art/'SOURCE_ARCHIVE.json').write_text(json.dumps(self.recipe))
    def test_online_local_upstream_exact_history(self):
        dest=self.root/'restored';r=recovery.recover(dest,self.art,upstream=str(self.repo))
        self.assertTrue(r['exact_history']);self.assertEqual(r['head'],self.head);self.assertEqual(git(dest,'branch','--show-current'),'arena/synthetic')
    def test_OFFLINE_exact_history_original_commitIDs(self):
        dest=self.root/'restored';r=recovery.recover(dest,self.art,offline=True,allow_source_fallback=False)
        self.assertEqual(r['mode'],'offline-reconstructed-base');self.assertTrue(r['exact_history'])
        self.assertEqual(git(dest,'rev-parse','HEAD'),self.head);self.assertEqual(git(dest,'rev-parse',self.base+'^{tree}'),git(self.repo,'rev-parse',self.base+'^{tree}'))
        self.assertEqual((dest/'a.txt').read_text(),'new\n');self.assertEqual((dest/'b.txt').read_text(),'unchanged\n')
    def test_xz_source_restores_exact_history_offline(self):
        import gzip
        import lzma
        original=self.art/'lccc-src.tar.gz'
        compact=self.art/'lccc-src.tar.xz'
        compact.write_bytes(lzma.compress(gzip.decompress(original.read_bytes())))
        self.recipe.update(archive=compact.name,archive_sha256=recovery.sha(compact))
        self.write_recipe();original.unlink()
        dest=self.root/'restored'
        result=recovery.recover(dest,self.art,offline=True,allow_source_fallback=False)
        self.assertTrue(result['exact_history'])
        self.assertEqual(git(dest,'rev-parse','HEAD'),self.head)
        self.assertEqual(git(dest,'rev-parse','HEAD^{tree}'),self.tree)

    def pack_bundle(self,kind="valid"):
        bundle_path=self.art/'lccc-session.bundle'
        data=bundle_path.read_bytes()
        archive=self.art/'session-bundles.tar.xz'
        with tarfile.open(archive,'w:xz') as tar:
            if kind=="corrupt":data=b'not a Git bundle'
            item=tarfile.TarInfo('other.bundle' if kind=="missing" else bundle_path.name);item.size=len(data)
            if kind=="link":item.type=tarfile.SYMTYPE;item.linkname='/etc/passwd';item.size=0
            tar.addfile(item,io.BytesIO(data) if item.isfile() else None)
            if kind=="duplicate":tar.addfile(item,io.BytesIO(data))
            if kind=="escape":
                other=tarfile.TarInfo('../escape');other.size=0;tar.addfile(other,io.BytesIO())
        metadata=json.loads((self.art/'SESSION_BUNDLE.json').read_text())
        metadata.update(bundle_archive=archive.name,bundle_archive_sha256=recovery.sha(archive))
        (self.art/'SESSION_BUNDLE.json').write_text(json.dumps(metadata));bundle_path.unlink()
        return archive

    def test_retained_bundle_count_and_independent_byte_budgets(self):
        for count in (16, 17, recovery.BUNDLE_ARCHIVE_MAX_MEMBERS):
            self.assertTrue(recovery.bundle_archive_within_budget(count, 1, count))
        self.assertFalse(recovery.bundle_archive_within_budget(65, 1, 65))
        self.assertFalse(recovery.bundle_archive_within_budget(1, 64*1024*1024+1, 64*1024*1024+1))
        self.assertFalse(recovery.bundle_archive_within_budget(17, 1, 256*1024*1024+1))
        self.assertFalse(recovery.bundle_archive_within_budget(1, -1, 1))

    def test_seventeen_retained_bundles_restore_exact_history(self):
        archive = self.pack_bundle()
        with tarfile.open(archive, 'r:xz') as tar:
            data = tar.extractfile('lccc-session.bundle').read()
        with tarfile.open(archive, 'w:xz', preset=0) as tar:
            for index in range(17):
                name = 'lccc-session.bundle' if index == 16 else f'old-{index}.bundle'
                item = tarfile.TarInfo(name); item.size = len(data)
                tar.addfile(item, io.BytesIO(data))
        meta = json.loads((self.art/'SESSION_BUNDLE.json').read_text())
        meta['bundle_archive_sha256'] = recovery.sha(archive)
        (self.art/'SESSION_BUNDLE.json').write_text(json.dumps(meta))
        result = recovery.recover(self.root/'restored', self.art, offline=True, allow_source_fallback=False)
        self.assertTrue(result['exact_history'])
        self.assertEqual(result['head'], self.head)

    def test_packed_bundle_offline_exact_history_without_persistent_extraction(self):
        self.pack_bundle()
        dest=self.root/'restored'
        result=recovery.recover(dest,self.art,offline=True,allow_source_fallback=False)
        self.assertTrue(result['exact_history']);self.assertEqual(git(dest,'rev-parse','HEAD'),self.head)
        self.assertFalse((self.art/'lccc-session.bundle').exists())

    def test_packed_bundle_rejects_invalid_members(self):
        original=(self.art/'lccc-session.bundle').read_bytes()
        for kind in ['link','duplicate','escape','corrupt','missing']:
            with self.subTest(kind=kind):
                (self.art/'lccc-session.bundle').write_bytes(original);self.pack_bundle(kind)
                dest=self.root/('bad-'+kind)
                with self.assertRaises(ValueError):
                    recovery.recover(dest,self.art,offline=True,allow_source_fallback=False)
                self.assertFalse(dest.exists());self.assertFalse((self.root/'escape').exists())

    def test_packed_bundle_hash_and_standalone_precedence(self):
        archive=self.pack_bundle();good=archive.read_bytes();archive.write_bytes(b'corrupt')
        with self.assertRaises(ValueError):
            recovery.recover(self.root/'bad-archive',self.art,offline=True,allow_source_fallback=False)
        archive.write_bytes(good);(self.art/'lccc-session.bundle').write_bytes(b'corrupt')
        with self.assertRaises(ValueError):
            recovery.recover(self.root/'bad-standalone',self.art,offline=True,allow_source_fallback=False)
        self.assertFalse((self.root/'bad-archive').exists());self.assertFalse((self.root/'bad-standalone').exists())

    def test_failed_network_uses_offline_recipe(self):
        dest=self.root/'restored';r=recovery.recover(dest,self.art,upstream='/definitely/missing/upstream',allow_source_fallback=False)
        self.assertEqual(r['mode'],'offline-reconstructed-base');self.assertEqual(r['head'],self.head)
    def test_corrupt_bundle_honest_source_fallback(self):
        (self.art/'lccc-session.bundle').write_bytes(b'corrupt')
        dest=self.root/'restored';r=recovery.recover(dest,self.art,offline=True)
        self.assertFalse(r['exact_history']);self.assertEqual(r['mode'],'verified-source-fallback');self.assertEqual(r['original_head'],self.head)
        self.assertEqual(git(dest,'rev-parse','HEAD^{tree}'),self.tree);self.assertEqual((dest/'a.txt').read_text(),'new\n')
    def test_corrupt_patch_exact_requirement_preserves_destination(self):
        (self.root/'ms178-1.patch').write_bytes(b'corrupt');dest=self.root/'restored'
        with self.assertRaises(ValueError):recovery.recover(dest,self.art,offline=True,allow_source_fallback=False)
        self.assertFalse(dest.exists())
    def test_corrupt_archive_no_false_offline_fallback(self):
        (self.art/'lccc-src.tar.gz').write_bytes(b'corrupt');dest=self.root/'restored'
        with self.assertRaises(ValueError):recovery.recover(dest,self.art,offline=True)
        self.assertFalse(dest.exists())
    def test_BASE_raw_identity_mismatch_rejected(self):
        self.recipe['base_commit_base64']=base64.b64encode(b'wrong commit').decode();self.write_recipe()
        with self.assertRaises(ValueError):recovery.recover(self.root/'restored',self.art,offline=True)
    def test_preserve_existing_worktree_modifications_and_deletions(self):
        dest=self.root/'existing';dest.mkdir();(dest/'a.txt').write_text('uncommitted work\n');(dest/'untracked').write_text('valuable')
        r=recovery.recover(dest,self.art,offline=True,preserve_worktree=True)
        self.assertTrue(r['worktree_preserved']);self.assertEqual((dest/'a.txt').read_text(),'uncommitted work\n')
        self.assertEqual((dest/'untracked').read_text(),'valuable');self.assertFalse((dest/'b.txt').exists())
        self.assertIn(' M a.txt',subprocess.check_output(['git','-C',str(dest),'status','--porcelain']).decode())
    def test_existing_git_never_overwritten(self):
        with self.assertRaises(ValueError):recovery.recover(self.repo,self.art,offline=True,preserve_worktree=True)
        self.assertEqual(git(self.repo,'rev-parse','HEAD'),self.head)
    def test_nonempty_destination_requires_preserve_mode(self):
        dest=self.root/'existing';dest.mkdir();(dest/'valuable').write_text('keep')
        with self.assertRaises(ValueError):recovery.recover(dest,self.art,offline=True)
        self.assertEqual((dest/'valuable').read_text(),'keep')
    def test_source_generation_mismatch_never_restores_old_HEAD_silently(self):
        b=json.loads((self.art/'SESSION_BUNDLE.json').read_text());b['head']=self.base;(self.art/'SESSION_BUNDLE.json').write_text(json.dumps(b))
        dest=self.root/'restored';r=recovery.recover(dest,self.art,offline=True)
        self.assertFalse(r['exact_history']);self.assertEqual(r['original_head'],self.head);self.assertEqual(git(dest,'rev-parse','HEAD^{tree}'),self.tree)
    def test_unsafe_archive_path_rejected_even_with_matching_hash(self):
        data=io.BytesIO()
        with tarfile.open(fileobj=data,mode='w:gz') as tar:
            item=tarfile.TarInfo('lccc/../../escape');item.size=4;tar.addfile(item,io.BytesIO(b'evil'))
        (self.art/'lccc-src.tar.gz').write_bytes(data.getvalue());self.recipe['archive_sha256']=recovery.sha(self.art/'lccc-src.tar.gz');self.write_recipe()
        with self.assertRaises(ValueError):recovery.recover(self.root/'restored',self.art,offline=True)
        self.assertFalse((self.root/'escape').exists())
    def test_duplicate_or_nonfinite_metadata_rejected(self):
        for text in ('{"head":1,"head":2}','{"bad":Infinity}'):
            p=self.root/'bad.json';p.write_text(text)
            with self.assertRaises(ValueError):recovery.read_json(p)
    def test_bootstrap_source_only_no_install_or_compiler_queries(self):
        dest=self.root/'bootstrap';env=dict(os.environ,LCCC_REPO=str(dest),LCCC_ARTIFACTS=str(self.art),LCCC_RECOVERY_OFFLINE='1')
        p=subprocess.run(['bash',str(REPO/'scripts/lccc-bootstrap.sh'),'--source-only'],env=env,capture_output=True,text=True,timeout=30)
        self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(git(dest,'rev-parse','HEAD'),self.head);self.assertIn('compiler builds were not requested',p.stdout)
    def test_restore_source_only_preserves_worktree_without_toolchains(self):
        dest=self.root/'arena';(dest/'scripts').mkdir(parents=True)
        for filename in ('arena_session_restore.sh','lccc_recover.py'):shutil.copy2(REPO/'scripts'/filename,dest/'scripts'/filename)
        (dest/'a.txt').write_text('valuable work')
        env=dict(os.environ,LCCC_ARTIFACTS=str(self.art),LCCC_RECOVERY_OFFLINE='1')
        p=subprocess.run(['bash',str(dest/'scripts/arena_session_restore.sh'),'--source-only'],env=env,capture_output=True,text=True,timeout=30)
        self.assertEqual(p.returncode,0,p.stderr);self.assertEqual((dest/'a.txt').read_text(),'valuable work');self.assertEqual(git(dest,'rev-parse','HEAD'),self.head)
        self.assertIn('no swap/toolchain/packages/compiler queries/builds',p.stdout)

class ExtractSafetyTests(unittest.TestCase):
    """`extract` refuses an escaping link on EVERY interpreter, not just 3.12+.

    The member loop validates member NAMES and never the symlink TARGET, so the
    only thing that ever stopped `lccc/evil -> ../../../../etc/passwd` was
    tarfile's PEP 706 data filter -- 3.12+/3.11.4+, against the Python 3.9+
    floor docs/getting-started.md declares.  A portability fallback that drops
    the filter must not drop the guarantee with it, so the loop now rejects the
    escaping target itself and these pin both paths.
    """

    def setUp(self):
        self._td=tempfile.TemporaryDirectory();self.addCleanup(self._td.cleanup)

    def archive(self,extra):
        buf=io.BytesIO()
        with tarfile.open(fileobj=buf,mode='w:gz') as t:
            root=tarfile.TarInfo('lccc');root.type=tarfile.DIRTYPE;root.mode=0o775;t.addfile(root)
            for member in extra:t.addfile(member)
        data=buf.getvalue()
        path=Path(self._td.name)/'src.tar.gz';path.write_bytes(data)
        return path,hashlib.sha256(data).hexdigest()

    def sym(self,name,target):
        member=tarfile.TarInfo(name);member.type=tarfile.SYMTYPE;member.linkname=target;member.mode=0o775
        return member

    def test_escaping_symlink_refused_by_the_member_rules(self):
        path,digest=self.archive([self.sym('lccc/evil','../../../../etc/passwd')])
        with tempfile.TemporaryDirectory() as stage:
            with self.assertRaises(ValueError) as caught:
                recovery.extract(path,Path(stage),{'archive_sha256':digest})
        self.assertIn('escaping archive link target',str(caught.exception))

    def test_escaping_symlink_refused_on_the_legacy_path(self):
        # A pre-3.12 interpreter: extractall accepts no `filter` keyword, so the
        # TypeError fallback runs -- the loop must still be what refuses it.
        path,digest=self.archive([self.sym('lccc/evil','../../../../etc/passwd')])
        real=tarfile.TarFile.extractall
        def legacy(self,dest,**kw):
            kw.pop('filter',None);return real(self,dest,**kw)
        with tempfile.TemporaryDirectory() as stage:
            with mock.patch.object(tarfile.TarFile,'extractall',legacy):
                with self.assertRaises(ValueError) as caught:
                    recovery.extract(path,Path(stage),{'archive_sha256':digest})
        self.assertIn('escaping archive link target',str(caught.exception))

    def test_special_mode_bits_refused(self):
        member=tarfile.TarInfo('lccc/x');member.type=tarfile.REGTYPE;member.size=0;member.mode=0o4755
        path,digest=self.archive([member])
        with tempfile.TemporaryDirectory() as stage:
            with self.assertRaises(ValueError) as caught:
                recovery.extract(path,Path(stage),{'archive_sha256':digest})
        self.assertIn('setuid/setgid/sticky',str(caught.exception))

    def test_in_tree_relative_symlink_still_extracted(self):
        # The rule is strict about escaping targets, not about links as such.
        path,digest=self.archive([self.sym('lccc/link','README.md')])
        with tempfile.TemporaryDirectory() as stage:
            repo=recovery.extract(path,Path(stage),{'archive_sha256':digest})
            self.assertTrue((repo/'link').is_symlink())

    def test_filter_support_is_probed_from_the_signature(self):
        supported = 'filter' in inspect.signature(tarfile.TarFile.extractall).parameters
        self.assertEqual(recovery.extractall_takes_filter(), supported)
        def legacy(self,path='.',members=None,*,numeric_owner=False):pass
        with mock.patch.object(tarfile.TarFile,'extractall',legacy):
            self.assertFalse(recovery.extractall_takes_filter())

    def test_an_unrelated_typeerror_is_not_swallowed_into_an_unfiltered_retry(self):
        # The exact failure mode the probe replaces.  `except TypeError` around
        # the filtered call catches an error raised from INSIDE it and silently
        # retries without the filter, extracting content the filter refused.
        # The stub keeps `filter` in its signature so the probe still reports
        # support -- patching it away would test the fallback instead.
        path,digest=self.archive([self.sym('lccc/link','README.md')])
        real=tarfile.TarFile.extractall
        def filtered_call_fails(self,dest,members=None,*,filter=None,numeric_owner=False):
            if filter is not None:raise TypeError('unrelated internal failure')
            return real(self,dest,members=members,numeric_owner=numeric_owner)
        with tempfile.TemporaryDirectory() as stage:
            with mock.patch.object(tarfile.TarFile,'extractall',filtered_call_fails):
                with self.assertRaises(TypeError) as caught:
                    recovery.extract(path,Path(stage),{'archive_sha256':digest})
        self.assertIn('unrelated internal failure',str(caught.exception))


def _bytes(root,extra):
    buf=io.BytesIO()
    with tarfile.open(fileobj=buf,mode='w:gz') as t:
        head=tarfile.TarInfo(root);head.type=tarfile.DIRTYPE;head.mode=0o775;t.addfile(head)
        for member in extra:t.addfile(member)
    return buf.getvalue()


def _reg(name,mode=0o644,uid=0,gid=0,uname='root',gname='root'):
    member=tarfile.TarInfo(name);member.type=tarfile.REGTYPE;member.size=0;member.mode=mode
    member.uid,member.gid,member.uname,member.gname=uid,gid,uname,gname
    return member


def _snapshot(root):
    out={}
    for path in sorted(Path(root).rglob('*')):
        st=path.lstat()
        out[path.relative_to(root).as_posix()]=(
            'dir' if os.path.isdir(path) and not os.path.islink(path) else 'file',
            st.st_mode&0o7777,st.st_uid,st.st_gid)
    return out


class MetadataContractTests(unittest.TestCase):
    """The pre-3.12 fallback must neutralize ownership and mode, not just names.

    The member loop validates names, types and link targets.  It cannot cover
    metadata: an unfiltered extractall chowns and chmods with the archive's own
    values, and this helper restores LOCAL artifacts whose neighbouring
    metadata is not a signature.  tarfile's data filter closes that gap on
    3.12+/3.11.4+; neutralize_archive_attrs is its metadata half on the floor.
    """

    def setUp(self):
        self._td=tempfile.TemporaryDirectory();self.addCleanup(self._td.cleanup)
        self.root=Path(self._td.name)

    def write(self,data):
        path=self.root/'src.tar.gz';path.write_bytes(data)
        return path,{'archive_sha256':hashlib.sha256(data).hexdigest()}

    def test_supported_path_passes_filter_data(self):
        # `filter` has to stay in the stub signature: extractall_takes_filter
        # probes inspect.signature(tarfile.TarFile.extractall) at call time, so
        # a `**kw` wrapper reads as unsupported and flips the helper to the
        # legacy branch.  Measured -- a `**kw` spy made this observe ABSENT.
        #
        # The probe reads the PATCHED function, so on an interpreter without
        # PEP 706 (the documented 3.9+ floor, e.g. 3.11.2) the spy makes the
        # helper report support and the forwarded `filter=` then fails inside
        # the real extractall.  The claim under test -- "the supported path
        # passes filter='data'" -- has no subject there: skip it instead of
        # asserting a capability the interpreter does not have.
        real=tarfile.TarFile.extractall
        if 'filter' not in inspect.signature(real).parameters:
            self.skipTest('interpreter extractall has no PEP 706 filter keyword')
        seen={}
        def spy(self,path='.',members=None,*,filter=None,**kw):
            seen['filter']=filter if filter is not None else 'ABSENT'
            return real(self,path,members=members,filter=filter)
        archive,record=self.write(_bytes('lccc',[_reg('lccc/ok.txt')]))
        with tempfile.TemporaryDirectory() as stage:
            with mock.patch.object(tarfile.TarFile,'extractall',spy):
                recovery.extract(archive,Path(stage),record)
        self.assertEqual(seen.get('filter'),'data')

    def test_legacy_path_neutralizes_ownership_and_mode(self):
        real=tarfile.TarFile.extractall
        # `fully_trusted` pins the legacy behaviour on interpreters that have
        # PEP 706, and does not exist before it (3.11.2 and older reject the
        # keyword outright) -- forward it only where it is accepted.
        forward=(lambda kw: kw) if 'filter' in inspect.signature(real).parameters else (lambda kw: {})
        seen={}
        def spy(self,path='.',members=None,*,filter=None,**kw):
            seen['members']=[(m.mode,m.uid,m.gid,m.uname,m.gname)
                             for m in (members if members is not None else self.getmembers())]
            return real(self,path,members=members,**forward({'filter':'fully_trusted'}))
        archive,record=self.write(_bytes('lccc',[
            _reg('lccc/x',mode=0o777,uid=1234,gid=4321,uname='mallory',gname='mallory')]))
        with tempfile.TemporaryDirectory() as stage:
            with mock.patch.object(recovery,'extractall_takes_filter',lambda: False), \
                    mock.patch.object(tarfile.TarFile,'extractall',spy):
                recovery.extract(archive,Path(stage),record)
        self.assertTrue(seen['members'],'the guard never reached extractall')
        for mode,uid,gid,uname,gname in seen['members']:
            self.assertIsNone(uid);self.assertIsNone(gid)
            self.assertIsNone(uname);self.assertIsNone(gname)
            self.assertTrue(mode is None or not mode&0o022,
                            f'group/other write survived: {mode!r}')

    def test_legacy_path_matches_the_data_filter_attribute_for_attribute(self):
        # Expectation derived from the real filter rather than a hand-written
        # table, so it tracks CPython instead of drifting from it.
        sub=tarfile.TarInfo('lccc/sub');sub.type=tarfile.DIRTYPE;sub.mode=0o707
        entries=[sub,_reg('lccc/a',mode=0o777),_reg('lccc/b',mode=0o600),_reg('lccc/c',mode=0o000)]
        real=tarfile.TarFile.extractall
        forward=(lambda kw: kw) if 'filter' in inspect.signature(real).parameters else (lambda kw: {})
        def legacy(self,path='.',members=None,**kw):
            kw.pop('filter',None)
            return real(self,path,members=members,**forward({'filter':'fully_trusted'}))
        with tempfile.TemporaryDirectory() as filtered,tempfile.TemporaryDirectory() as legacy_dir:
            archive,record=self.write(_bytes('lccc',entries))
            recovery.extract(archive,Path(filtered),record)
            with mock.patch.object(recovery,'extractall_takes_filter',lambda: False), \
                    mock.patch.object(tarfile.TarFile,'extractall',legacy):
                recovery.extract(archive,Path(legacy_dir),record)
            self.assertEqual(_snapshot(Path(filtered)/'lccc'),_snapshot(Path(legacy_dir)/'lccc'))

    def test_neutralizer_matches_tarfile_data_filter_over_the_mode_matrix(self):
        if not hasattr(tarfile,'data_filter'):
            self.skipTest('tarfile.data_filter unavailable on this interpreter')
        import copy
        kinds=((tarfile.REGTYPE,'reg'),(tarfile.DIRTYPE,'dir'),(tarfile.SYMTYPE,'sym'))
        for mode in (0o777,0o755,0o644,0o600,0o400,0o000,0o4755,0o2755,0o1777,0o707):
            for kind,label in kinds:
                with self.subTest(mode=oct(mode),type=label):
                    base=tarfile.TarInfo('lccc/x');base.type=kind;base.mode=mode
                    base.uid,base.gid=1234,4321;base.uname,base.gname='mallory','mallory'
                    if kind==tarfile.SYMTYPE:base.linkname='target'
                    ref=tarfile.data_filter(copy.deepcopy(base),'/tmp/dest')
                    got=recovery.neutralize_archive_attrs(copy.deepcopy(base))
                    self.assertEqual(
                        (got.mode,got.uid,got.gid,got.uname,got.gname),
                        (ref.mode,ref.uid,ref.gid,ref.uname,ref.gname))


class MergedRangeBundleContract(unittest.TestCase):
    """A session range that merged upstream must still prove against BASE.

    2026-10-13, found while snapshotting the branch right after it had been
    merged with upstream main and re-anchored onto it: `build` failed with
    "Repository lacks these prerequisite commits: <old fork point>".  The proof
    fetched BASE with --depth=1, but git records the OLD fork point as a second
    prerequisite of a merged range, and a depth-1 BASE cannot satisfy it, so a
    bundle that restores perfectly was rejected.  This is that shape, in
    miniature: session branch, an upstream line, a merge of the two, and the
    bundle proved from the new BASE.
    """

    def test_merged_range_verifies_against_base_ancestry(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td); repo = root / 'repo'; repo.mkdir()
            git(repo, 'init', '-q'); git(repo, 'config', 'user.name', 'test')
            git(repo, 'config', 'user.email', 'test@example.invalid')
            (repo / 'a.txt').write_text('base\n'); git(repo, 'add', '.')
            git(repo, 'commit', '-qm', 'base'); fork = git(repo, 'rev-parse', 'HEAD')
            git(repo, 'checkout', '-qb', 'arena/synthetic')
            (repo / 'a.txt').write_text('session\n'); git(repo, 'commit', '-qam', 'session')
            # the upstream line, and the merge of it into the session branch
            git(repo, 'checkout', '-q', '-b', 'main', fork)
            (repo / 'b.txt').write_text('upstream\n'); git(repo, 'add', '.')
            git(repo, 'commit', '-qm', 'upstream'); upstream = git(repo, 'rev-parse', 'HEAD')
            git(repo, 'checkout', '-q', 'arena/synthetic')
            git(repo, 'merge', '-q', '--no-ff', '-m', 'Merge upstream', 'main')
            head = git(repo, 'rev-parse', 'HEAD')

            meta = bundle.build(repo, upstream, root / 'lccc-session.bundle')

            self.assertEqual(meta['base'], upstream); self.assertEqual(meta['head'], head)
            self.assertTrue(meta['isolated_restore_verified'])
            # The cheap BASE-only proof cannot satisfy the merged range's second
            # prerequisite; the receipt has to say which proof actually ran.
            self.assertEqual(meta['base_fetch'], 'base-with-ancestry')
            restored = root / 'restored'; restored.mkdir()
            git(restored, 'init', '-q')
            git(restored, 'fetch', '-q', '--no-tags', str(repo), upstream)
            git(restored, 'fetch', '-q', str(root / 'lccc-session.bundle'), 'arena/synthetic')
            git(restored, 'checkout', '-q', 'FETCH_HEAD')
            self.assertEqual(git(restored, 'rev-parse', 'HEAD'), head)
            self.assertEqual(git(restored, 'rev-parse', 'HEAD^{tree}'),
                             git(repo, 'rev-parse', 'HEAD^{tree}'))

    def test_recovery_of_merged_range_bundle_is_exact(self):
        """The delivered bundle of a merged session must restore exactly.

        Same shape as above, but through the user-facing entry point: the
        recovery tool fetches BASE `--depth=1`, so it needs the same fallback
        (deepen BASE) that the bundle proof needed, and it must not silently
        degrade to the source fallback while an exact restore is possible.
        """
        with tempfile.TemporaryDirectory() as td:
            root = Path(td); repo = root / 'repo'; repo.mkdir()
            art = root / 'art'; art.mkdir()
            git(repo, 'init', '-q'); git(repo, 'config', 'user.name', 'test')
            git(repo, 'config', 'user.email', 'test@example.invalid')
            (repo / 'a.txt').write_text('base\n'); git(repo, 'add', '.')
            git(repo, 'commit', '-qm', 'base'); fork = git(repo, 'rev-parse', 'HEAD')
            git(repo, 'checkout', '-qb', 'arena/synthetic')
            (repo / 'a.txt').write_text('session\n'); git(repo, 'commit', '-qam', 'session')
            git(repo, 'checkout', '-q', '-b', 'main', fork)
            (repo / 'b.txt').write_text('upstream\n'); git(repo, 'add', '.')
            git(repo, 'commit', '-qm', 'upstream'); upstream = git(repo, 'rev-parse', 'HEAD')
            git(repo, 'checkout', '-q', 'arena/synthetic')
            git(repo, 'merge', '-q', '--no-ff', '-m', 'Merge upstream', 'main')
            head = git(repo, 'rev-parse', 'HEAD'); tree = git(repo, 'rev-parse', 'HEAD^{tree}')
            meta = bundle.build(repo, upstream, art / 'lccc-session.bundle')
            (art / 'SESSION_BUNDLE.json').write_text(json.dumps(meta))

            dest = root / 'restored'
            result = recovery.recover(dest, art, upstream=str(repo))
            self.assertTrue(result['exact_history'], result)
            self.assertEqual(result['head'], head); self.assertEqual(result['tree'], tree)
            self.assertEqual((dest / 'a.txt').read_text(), 'session\n')
            self.assertEqual((dest / 'b.txt').read_text(), 'upstream\n')

            # Offline has BASE alone by construction, so the honest answer is
            # the verified source fallback -- never an invented exact history.
            # (No source recipe here, so the offline attempt must fail loudly.)
            with self.assertRaises(ValueError):
                recovery.recover(root / 'offline', art, upstream=str(repo), offline=True)


if __name__ == '__main__':
    unittest.main()
