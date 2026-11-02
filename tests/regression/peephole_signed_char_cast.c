#include <stdio.h>

__attribute__((noinline))
static int unicode_byte_type(char hi, char lo) {
  switch ((unsigned char)hi) {
  case 0xD8:
  case 0xD9:
  case 0xDA:
  case 0xDB:
    return 4;
  case 0xDC:
  case 0xDD:
  case 0xDE:
  case 0xDF:
    return 2;
  case 0xFF:
    switch ((unsigned char)lo) {
    case 0xFF:
    case 0xFE:
      return 1;
    }
    break;
  }
  return 3;
}

int main(void) {
  static volatile const signed char tests[][3] = {
    {0x00, 0x00, 3}, {(signed char)0xd7, 0x00, 3}, {(signed char)0xd8, 0x00, 4},
    {(signed char)0xdb, (signed char)0xff, 4}, {(signed char)0xdc, 0x00, 2}, {(signed char)0xdf, (signed char)0xff, 2},
    {(signed char)0xe0, 0x00, 3}, {(signed char)0xff, (signed char)0xfd, 3}, {(signed char)0xff, (signed char)0xfe, 1}, {(signed char)0xff, (signed char)0xff, 1}
  };
  int failed = 0;
  for (unsigned i = 0; i < sizeof(tests) / sizeof(tests[0]); ++i) {
    int got = unicode_byte_type((char)tests[i][0], (char)tests[i][1]);
    if (got != tests[i][2]) {
      printf("case %u: hi=%02x lo=%02x got=%d expected=%u\n",
             i, tests[i][0], tests[i][1], got, tests[i][2]);
      failed = 1;
    }
  }
  return failed;
}
