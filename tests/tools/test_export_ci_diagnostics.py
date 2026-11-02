import hashlib
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('transport', Path(__file__).resolve().parents[2]/'scripts/export_ci_diagnostics.py')
transport = importlib.util.module_from_spec(spec)
spec.loader.exec_module(transport)


class TransportTests(unittest.TestCase):
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
