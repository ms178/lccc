/* Verify that the same streaming XML input produces the same callback stream.
 * This harness is compiled with GCC for every library arm. */
#include <expat.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

struct Stats {
    uint64_t hash, starts, ends, text;
};

static void add(struct Stats *s, const char *p, size_t n) {
    for (size_t i = 0; i < n; ++i) {
        s->hash ^= (unsigned char)p[i];
        s->hash *= UINT64_C(1099511628211);
    }
}

static void start(void *v, const XML_Char *name, const XML_Char **attrs) {
    struct Stats *s = v;
    ++s->starts;
    add(s, "<", 1);
    add(s, name, strlen(name));
    for (size_t i = 0; attrs[i]; i += 2) {
        add(s, "=", 1);
        add(s, attrs[i], strlen(attrs[i]));
        add(s, attrs[i + 1], strlen(attrs[i + 1]));
    }
}

static void end(void *v, const XML_Char *name) {
    struct Stats *s = v;
    ++s->ends;
    add(s, ">", 1);
    add(s, name, strlen(name));
}

static void text(void *v, const XML_Char *data, int len) {
    struct Stats *s = v;
    s->text += (uint64_t)len;
    add(s, data, (size_t)len);
}

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    FILE *f = fopen(argv[1], "rb");
    if (!f) return 3;
    XML_Parser p = XML_ParserCreateNS(NULL, '!');
    if (!p) { fclose(f); return 4; }
    struct Stats s = {UINT64_C(14695981039346656037), 0, 0, 0};
    XML_SetUserData(p, &s);
    XML_SetElementHandler(p, start, end);
    XML_SetCharacterDataHandler(p, text);
    char buf[8192];
    int done;
    do {
        size_t n = fread(buf, 1, sizeof(buf), f);
        if (ferror(f)) { XML_ParserFree(p); fclose(f); return 5; }
        done = feof(f);
        if (XML_Parse(p, buf, (int)n, done) == XML_STATUS_ERROR) {
            fprintf(stderr, "parse error %d\n", XML_GetErrorCode(p));
            XML_ParserFree(p);
            fclose(f);
            return 6;
        }
    } while (!done);
    XML_ParserFree(p);
    fclose(f);
    printf("%" PRIu64 " %" PRIu64 " %" PRIu64 " %016" PRIx64 "\n",
           s.starts, s.ends, s.text, s.hash);
    return 0;
}
