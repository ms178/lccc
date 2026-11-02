import hashlib
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

    def test_cli_failure_is_annotated_and_still_fails(self):
        output = io.StringIO()
        with patch.object(transport, 'main', side_effect=RuntimeError('bad%\nline\r')), \
                patch('sys.stdout', output):
            with self.assertRaises(RuntimeError):
                transport.run_cli()
        self.assertIn('::error title=CI diagnostic transport failed::bad%25%0Aline%0D',
                      output.getvalue())

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
