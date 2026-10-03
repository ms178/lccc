"""Adversarial scalar, lifecycle and matcher tests added after native audit."""
from pathlib import Path
import math
import sys
import tempfile
import time
import unittest
from unittest import mock

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO))
from tools.corpus import diagnostics, process, publication, schema


class ScalarBoundaryTests(unittest.TestCase):
    def test_exponent_overflow_not_only_named_constants(self):
        for text in ('1e999', '-1e999', '9.9e9999'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                schema.loads('{"nested":{"value":' + text + '}}')

    def test_finite_float_and_integer_precision_preserved(self):
        self.assertEqual(schema.loads('{"n":9007199254740993,"s":0.25}'),
                         {'n': 9007199254740993, 's': 0.25})

    def test_count_rejects_falsey_nontext_values(self):
        for value in (False, 0, [], {}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                schema.count(value)
        self.assertEqual(schema.count(None), (1, 1))
        self.assertEqual(schema.count(''), (1, 1))

    def test_empty_relative_components_not_directory_records(self):
        for path in ('.', './', 'a/..', 'a//b'):
            with self.subTest(path=path), self.assertRaises(ValueError):
                schema.relative(path)


class ProcessLifecycleTests(unittest.TestCase):
    def test_closed_pipes_do_not_shorten_timeout(self):
        started = time.monotonic()
        result = process.run([sys.executable, '-c',
                              'import os,time; os.close(1); os.close(2); time.sleep(2.1)'], 3)
        self.assertEqual(result['returncode'], 0)
        self.assertIsNone(result['failure'])
        self.assertGreaterEqual(time.monotonic() - started, 2)

    def test_closed_pipes_hanging_process_still_times_out(self):
        result = process.run([sys.executable, '-c',
                              'import os,time; os.close(1); os.close(2); time.sleep(10)'], .15)
        self.assertEqual(result['failure'], 'timeout')
        self.assertLess(result['elapsed'], 2)

    def test_output_failure_grace_not_original_large_timeout(self):
        import os
        import signal
        with tempfile.TemporaryDirectory() as td:
            pidfile = Path(td) / 'child.pid'
            child = ('import os,time; os.setsid(); '
                     f'open({str(pidfile)!r},"w").write(str(os.getpid())); '
                     'os.write(1,b"x"*10000); time.sleep(10)')
            parent = 'import subprocess,sys; subprocess.Popen([sys.executable,"-c",' + repr(child) + '])'
            try:
                result = process.run([sys.executable, '-c', parent], 8, output_limit=256)
                self.assertEqual(result['failure'], 'output-limit')
                self.assertLess(result['elapsed'], 2)
                self.assertEqual(result['retained_bytes'], 256)
            finally:
                if pidfile.exists():
                    try:
                        os.kill(int(pidfile.read_text()), signal.SIGKILL)
                    except ProcessLookupError:
                        pass


class PublicationCommitTests(unittest.TestCase):
    def test_parent_fsync_failure_reports_committed_visibility(self):
        with tempfile.TemporaryDirectory() as td:
            destination = Path(td) / 'report.json'
            destination.write_bytes(b'old')
            with mock.patch.object(publication, 'sync_dir', side_effect=OSError('durability fault')):
                with self.assertRaises(publication.PublicationError) as caught:
                    publication.atomic_bytes(destination, b'new-complete')
            self.assertTrue(caught.exception.committed)
            self.assertEqual(destination.read_bytes(), b'new-complete')
            self.assertIn('durability unconfirmed', str(caught.exception))

    def test_exchange_parent_fsync_failure_preserves_both_complete_generations(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td); staged = root / 'stage'; live = root / 'live'
            staged.mkdir(); live.mkdir()
            (staged / 'x').write_text('new'); (live / 'x').write_text('old')
            with mock.patch.object(publication, 'sync_dir', side_effect=OSError('durability fault')):
                with self.assertRaises(publication.PublicationError) as caught:
                    publication.publish_directory(staged, live, force=True)
            self.assertTrue(caught.exception.committed)
            self.assertEqual((live / 'x').read_text(), 'new')
            self.assertEqual((staged / 'x').read_text(), 'old')

    def test_symlink_file_and_lock_refused(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td); target = root / 'valuable'; target.write_text('preserve')
            link = root / 'link'; link.symlink_to(target)
            with self.assertRaises(ValueError):
                publication.atomic_bytes(link, b'not permitted')
            with self.assertRaises(OSError):
                with publication.lock(link):
                    self.fail('lock symlink followed')
            self.assertEqual(target.read_text(), 'preserve')

    def test_writer_lock_deadline_is_bounded(self):
        with tempfile.TemporaryDirectory() as td:
            with publication.lock(Path(td) / 'lock'):
                start = time.monotonic()
                with self.assertRaises(TimeoutError):
                    with publication.lock(Path(td) / 'lock', timeout=.05):
                        self.fail('competing writer acquired lock')
                self.assertLess(time.monotonic() - start, .5)

    def test_published_mode_is_durable_before_rename(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / 'file'
            publication.atomic_bytes(path, b'data')
            self.assertEqual(path.stat().st_mode & 0o777, 0o644)


if __name__ == '__main__':
    unittest.main()
