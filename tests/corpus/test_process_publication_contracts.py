#!/usr/bin/env python3
"""Compiler-free real Python subprocess budgets and publication fault tests."""
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest import mock

REPO=Path(__file__).resolve().parents[2];sys.path.insert(0,str(REPO))
from tools.corpus import process,publication


class ProcessContract(unittest.TestCase):
    def test_closed_stdin_and_locale(self):
        r=process.run([sys.executable,'-c','import os,sys; print(os.environ["LC_ALL"]); print(len(sys.stdin.read()))'],2)
        self.assertEqual(r['stdout'],b'C\n0\n');self.assertEqual(r['returncode'],0)
    def test_timeout_retains_prefix_and_raw_code(self):
        r=process.run([sys.executable,'-c','import time; print("prefix",flush=True); time.sleep(9)'],.2)
        self.assertEqual(r['failure'],'timeout');self.assertIn(b'prefix',r['stdout']);self.assertLess(r['returncode'],0)
    def test_combined_output_budget(self):
        r=process.run([sys.executable,'-c','import os; os.write(1,b"x"*10000); os.write(2,b"y"*10000)'],2,output_limit=256)
        self.assertEqual(r['failure'],'output-limit');self.assertLessEqual(len(r['stdout'])+len(r['stderr']),256)
    def test_descendant_pipe_cannot_strand_reader(self):
        start=time.monotonic()
        r=process.run([sys.executable,'-c','import subprocess,sys; subprocess.Popen([sys.executable,"-c","import time; time.sleep(10)"])'],.2)
        self.assertEqual(r['failure'],'timeout');self.assertLess(time.monotonic()-start,2)
    def test_invalid_budgets(self):
        for value in (0,-1,float('inf'),float('nan')):
            with self.assertRaises(ValueError):process.run(['not-launched'],value)
    def test_spawn_failure_preserved_as_exception(self):
        with self.assertRaises(OSError):process.run(['/definitely/no/executable'],1)
    def test_hashes_evidence(self):
        import hashlib
        r=process.run([sys.executable,'-c','print("evidence")'],2)
        self.assertEqual(r['stdout_sha256'],hashlib.sha256(r['stdout']).hexdigest())


class PublicationContract(unittest.TestCase):
    def test_atomic_file_failure_preserves_destination(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/'index.json';p.write_bytes(b'old')
            with mock.patch.object(publication.os,'replace',side_effect=OSError('injected')),self.assertRaises(OSError):
                publication.atomic_bytes(p,b'new')
            self.assertEqual(p.read_bytes(),b'old');self.assertEqual(list(Path(td).glob('*.tmp')),[])
    def test_existing_directory_no_force_preserved(self):
        with tempfile.TemporaryDirectory() as td:
            live=Path(td)/'live';stage=Path(td)/'stage';live.mkdir();stage.mkdir()
            (live/'sentinel').write_text('old')
            with self.assertRaises(FileExistsError):publication.publish_directory(stage,live,force=False)
            self.assertEqual((live/'sentinel').read_text(),'old')
    def test_unavailable_exchange_preserves_old_and_stage(self):
        with tempfile.TemporaryDirectory() as td:
            live=Path(td)/'live';stage=Path(td)/'stage';live.mkdir();stage.mkdir()
            (live/'sentinel').write_text('old');(stage/'sentinel').write_text('new')
            with mock.patch.object(publication,'exchange',side_effect=OSError('unsupported')),self.assertRaises(OSError):
                publication.publish_directory(stage,live,force=True)
            self.assertEqual((live/'sentinel').read_text(),'old');self.assertEqual((stage/'sentinel').read_text(),'new')
    def test_real_exchange_no_missing_live_path(self):
        with tempfile.TemporaryDirectory() as td:
            live=Path(td)/'live';stage=Path(td)/'stage';live.mkdir();stage.mkdir()
            (live/'sentinel').write_text('old');(stage/'sentinel').write_text('new')
            publication.publish_directory(stage,live,force=True)
            self.assertEqual((live/'sentinel').read_text(),'new');self.assertEqual((stage/'sentinel').read_text(),'old')
    def test_symlink_live_refused(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);(root/'original').mkdir();(root/'stage').mkdir();(root/'live').symlink_to('original')
            with self.assertRaises(ValueError):publication.publish_directory(root/'stage',root/'live',force=True)

if __name__=='__main__':unittest.main()
