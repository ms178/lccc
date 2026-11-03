import hashlib
import gzip
import io
import subprocess
import importlib.util
import json
import re
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('transport', Path(__file__).resolve().parents[2]/'scripts/export_ci_diagnostics.py')
transport = importlib.util.module_from_spec(spec)
spec.loader.exec_module(transport)


class TransportTests(unittest.TestCase):
    def test_workflow_has_minimal_transport_permissions(self):
        workflow = (Path(__file__).resolve().parents[2] /
                    '.github/workflows/kernel-61858.yml').read_text()
        job = workflow.split('  diagnostic-transport:\n', 1)[1].split(
            '\n  host-prerequisites:', 1)[0]
        block = re.search(r'(?m)^    permissions:\n((?:      [^\n]*\n)+)', job)
        self.assertIsNotNone(block)
        permissions = dict(line.strip().split(': ', 1)
                           for line in block.group(1).splitlines()
                           if not line.strip().startswith('#'))
        # Job-level permissions reset every omitted scope to none. Downloading
        # Actions logs/artifacts needs read; publishing blobs needs contents write.
        self.assertEqual(permissions, {'actions': 'read', 'contents': 'write'})

    def test_api_failure_is_visible_without_credentials(self):
        error = subprocess.CalledProcessError(1, ['gh'], stderr=
            b'gh: HTTP 403 token-secret https://storage.invalid/log?sig=secret\n')
        with patch.dict(transport.os.environ, {'GH_TOKEN': 'token-secret'}), \
                patch.object(transport.subprocess, 'run', side_effect=error):
            with self.assertRaises(RuntimeError) as caught:
                transport.gh('repos/a/b/actions/runs')
        message = str(caught.exception)
        self.assertIn('HTTP 403', message)
        self.assertNotIn('token-secret', message)
        self.assertNotIn('sig=secret', message)
        self.assertNotIn('storage.invalid', message)

    def test_transient_reads_and_blob_creation_retry_identically(self):
        for status in (429, 500, 502, 503, 504):
            for payload, path in ((None, 'repos/a/b/actions/jobs/7/logs'),
                                  (b'blob-payload', 'repos/a/b/git/blobs')):
                error = subprocess.CalledProcessError(1, ['gh'],
                            stderr=f'gh: Server Error (HTTP {status})'.encode())
                success = subprocess.CompletedProcess([], 0, stdout=b'raw\x00\xff')
                with self.subTest(status=status, payload=payload), \
                        patch.object(transport.subprocess, 'run', side_effect=[error, success]) as call, \
                        patch.object(transport.time, 'sleep') as sleep:
                    self.assertEqual(transport.gh(path, payload, allow_binary=True), b'raw\x00\xff')
                    self.assertEqual(call.call_count, 2)
                    self.assertEqual(call.call_args_list[0], call.call_args_list[1])
                    sleep.assert_called_once_with(1)

    def test_timeout_retries_but_auth_and_other_posts_do_not(self):
        error = subprocess.TimeoutExpired(['gh'], 180)
        success = subprocess.CompletedProcess([], 0, stdout=b'done')
        with patch.object(transport.subprocess, 'run', side_effect=[error, success]) as call, \
                patch.object(transport.time, 'sleep') as sleep:
            self.assertEqual(transport.gh('repos/a/b/actions/runs'), b'done')
            self.assertEqual(call.call_count, 2)
            sleep.assert_called_once_with(1)
        for status, payload, path in [(401, None, 'read'), (403, None, 'read'),
                                      (404, None, 'read'), (422, None, 'read'),
                                      (502, b'payload', 'repos/a/b/issues')]:
            error = subprocess.CalledProcessError(1, ['gh'], stderr=f'HTTP {status}'.encode())
            with self.subTest(status=status, path=path), \
                    patch.object(transport.subprocess, 'run', side_effect=error) as call, \
                    patch.object(transport.time, 'sleep') as sleep:
                with self.assertRaises(RuntimeError):
                    transport.gh(path, payload)
                self.assertEqual(call.call_count, 1)
                sleep.assert_not_called()

    def test_exhausted_retries_still_fail_and_redact_secrets(self):
        error = subprocess.CalledProcessError(1, ['gh'], stderr=
            b'HTTP 502 token-secret https://storage.invalid/log?sig=secret')
        with patch.dict(transport.os.environ, {'GITHUB_TOKEN': 'token-secret'}), \
                patch.object(transport.subprocess, 'run', side_effect=error) as call, \
                patch.object(transport.time, 'sleep') as sleep:
            with self.assertRaises(RuntimeError) as caught:
                transport.gh('repos/a/b/actions/runs')
        self.assertEqual(call.call_count, 4)
        self.assertEqual([c.args[0] for c in sleep.call_args_list], [1, 2, 4])
        self.assertIn('HTTP 502', str(caught.exception))
        for secret in ('token-secret', 'sig=secret', 'storage.invalid'):
            self.assertNotIn(secret, str(caught.exception))

    def test_cli_failure_is_annotated_and_still_fails(self):
        output = io.StringIO()
        with patch.object(transport, 'main', side_effect=RuntimeError('bad%\nline\r')), \
                patch('sys.stdout', output):
            with self.assertRaises(RuntimeError):
                transport.run_cli()
        self.assertIn('::error title=CI diagnostic transport failed::bad%25%0Aline%0D',
                      output.getvalue())

    def test_binary_opt_in_preserves_bytes_without_terminal_output(self):
        raw = b'PK\x03\x04\x1b[31m\x00\xff'
        result = subprocess.CompletedProcess([], 0, stdout=raw)
        for binary in (False, True):
            with self.subTest(binary=binary), \
                    patch.object(transport.subprocess, 'run', return_value=result) as run:
                self.assertEqual(transport.gh('endpoint', allow_binary=binary), raw)
                self.assertEqual('--allow-escape-sequences' in run.call_args.args[0], binary)
                self.assertIs(run.call_args.kwargs['capture_output'], True)

    def test_only_evidence_downloads_opt_in_to_binary(self):
        log = b'compiler: \x1b[31merror\x1b[0m\n'
        archive = b'PK\x03\x04\x1b\x00\xff'
        def api(path, payload=None, *, allow_binary=False):
            if path.endswith('/logs'):
                self.assertTrue(allow_binary)
                return log
            if path.endswith('/zip'):
                self.assertTrue(allow_binary)
                return archive
            self.assertFalse(allow_binary)
            if '/jobs?' in path:
                result = {'jobs': [{'id': 7, 'conclusion': 'failure'}]}
            elif '/artifacts?' in path:
                result = {'artifacts': [{'id': 8, 'expired': False,
                                        'name': 'kernel-61858-test'}]}
            else:
                result = {'repository': {'full_name': 'a/b'}, 'status': 'completed',
                          'head_sha': 'abc', 'conclusion': 'failure'}
            return json.dumps(result).encode()
        output = io.StringIO()
        with patch.dict(transport.os.environ, {'GITHUB_REPOSITORY': 'a/b', 'SOURCE_RUN_IDS': '42'}), \
                patch.object(transport, 'gh', api), \
                patch.object(transport, 'export_file', return_value=[]) as export, \
                patch('sys.stdout', output):
            transport.main()
        self.assertEqual(gzip.decompress(export.call_args_list[0].args[2]), log)
        self.assertEqual(export.call_args_list[1].args[2], archive)
        self.assertNotIn('\x1b', output.getvalue())

    def test_ids(self):
        self.assertEqual(transport.parse_run_ids('1,42'), [1, 42])
        for invalid in ('', '0', '-1', '1,1', '1, 2', '1;echo x', '1,2,3,4,5', 'a', '1\n', '9'*21):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                transport.parse_run_ids(invalid)

    def test_parts(self):
        seen = []
        def gh(path, payload):
            import base64
            data = base64.b64decode(json.loads(payload)['content'], validate=True)
            seen.append(data)
            return json.dumps({'sha': hashlib.sha1(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest()}).encode()
        with patch.object(transport, 'PART_BYTES', 3), patch.object(transport, 'gh', gh):
            rows = transport.export_file('a/b', 'test', b'1234567')
        self.assertEqual(seen, [b'123', b'456', b'7'])
        self.assertEqual(len(rows), 4)
        self.assertIn(hashlib.sha256(b'1234567').hexdigest(), rows[0])

    def test_reject_wrong_blob_identity(self):
        with patch.object(transport, 'gh', return_value=b'{"sha":"wrong"}'):
            with self.assertRaises(RuntimeError):
                transport.export_file('a/b', 'test', b'payload')


if __name__ == '__main__':
    unittest.main()
