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
import sys
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

    def test_escaping_symlink_refused_with_the_data_filter(self):
        path,digest=self.archive([self.sym('lccc/evil','../../../../etc/passwd')])
        with tempfile.TemporaryDirectory() as stage:
            with self.assertRaises(ValueError) as caught:
                recovery.extract(path,Path(stage),{'archive_sha256':digest})
        self.assertIn('escaping archive link target',str(caught.exception))

    def test_escaping_symlink_refused_without_the_data_filter(self):
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


if __name__=='__main__':unittest.main()
