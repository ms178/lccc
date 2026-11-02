"""The object-name table is shared by ob and cob, not the cost context."""
import io
import unittest
from scripts.callgrind_transfers import count_transfers


class TransferCounts(unittest.TestCase):
    def count(self, text):
        return count_transfers(io.StringIO(text), '/owned program')

    def test_callee_can_introduce_own_name_before_current_object(self):
        self.assertEqual(self.count('''ob=(1) /lib/libc.so
cob=(5) /owned program
jump=999
ob=(5)
jcnd=7/10 0x1234
jump=3 0x1235
'''), 10)

    def test_callee_does_not_redirect_the_current_cost_context(self):
        self.assertEqual(self.count('''ob=(5) /owned program
cob=(1) /lib/libc.so
jump=3 0x1234
ob=(1)
jump=999 0x1234
ob=(5)
jcnd=7/10 0x1234
'''), 10)

    def test_uncompressed_and_large_integer_counts(self):
        self.assertEqual(self.count('''ob=/owned program
jump=9007199254740993 0x1234
jcnd=2/4 0x1234
'''), 9007199254740995)

    def test_missing_or_undefined_object_is_not_zero_measurement(self):
        for profile in ('ob=(1)\n', 'ob=(1) /other\njump=2\n'):
            with self.subTest(profile=profile), self.assertRaises(ValueError):
                self.count(profile)

    def test_malformed_owned_transfer_is_not_silently_ignored(self):
        with self.assertRaises(ValueError):
            self.count('ob=(5) /owned program\njump=not-a-count\n')
