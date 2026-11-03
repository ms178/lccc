/* A feature operator must not corrupt non-ASCII bytes elsewhere on its line. */
int main(void) { const unsigned char text[] = "é"; (void)__has_feature(cxx_exceptions); return sizeof(text) == 3 && text[0] == 0xc3 && text[1] == 0xa9 ? 0 : 1; }
