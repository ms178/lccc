/* Raw control bytes inside literals below are intentional regression input.
 * Do not replace them with C escapes: that would stop testing marker collisions.
 * Expected strings use escapes, independently of the raw input spelling. */
#define Q(x) #x
#define S(x) Q(x)
#define CAT(x,y) x ## y
static int eq(const char *a, const char *b) {
  while (*a && *a == *b) { ++a; ++b; }
  return *a == *b;
}
int main(void) {
  if (!eq("", "\001")) return 1;
  if (!eq(CAT(,""), "\001")) return 2;
  if (!eq(Q(""), "\"\001\"")) return 3;
  if (!eq(S(""), "\"\001\"")) return 4;
  if ('' != 1) return 5;
  if (CAT(,'') != 1) return 6;
  if (!eq("", "\002")) return 7;
  if (!eq(CAT(,""), "\002")) return 8;
  if (!eq(Q(""), "\"\002\"")) return 9;
  if (!eq(S(""), "\"\002\"")) return 10;
  if ('' != 2) return 11;
  if (CAT(,'') != 2) return 12;
  if (!eq("", "\003")) return 13;
  if (!eq(CAT(,""), "\003")) return 14;
  if (!eq(Q(""), "\"\003\"")) return 15;
  if (!eq(S(""), "\"\003\"")) return 16;
  if ('' != 3) return 17;
  if (CAT(,'') != 3) return 18;
  if (!eq("", "\004")) return 19;
  if (!eq(CAT(,""), "\004")) return 20;
  if (!eq(Q(""), "\"\004\"")) return 21;
  if (!eq(S(""), "\"\004\"")) return 22;
  if ('' != 4) return 23;
  if (CAT(,'') != 4) return 24;
  if (!eq(Q("éλ😀"), "\"éλ😀\"")) return 25;
  if (!eq(S("éλ😀"), "\"éλ😀\"")) return 26;
  if (!eq(Q("a  b\"c\\d"), "\"a  b\\\"c\\\\d\"")) return 27;
  if (!eq(S("a  b\"c\\d"), "\"a  b\\\"c\\\\d\"")) return 28;
  return 0;
}
