#define MAJOR 1
#define MINOR 5
#define PATCH 7
#define QUOTE(x) #x
#define EXPAND(x) QUOTE(x)
#define VERSION(x) x.MINOR.PATCH
#define PLUS +
#define I 13
static int equal(const char *a, const char *b) {
    while (*a && *a == *b) { ++a; ++b; }
    return *a == *b;
}
int main(void) {
    return !equal(EXPAND(MAJOR.MINOR.PATCH), "1.5.7")
        || !equal(EXPAND(VERSION(MAJOR)), "1.5.7")
        || !equal(EXPAND(PLUS+), "++")
        || !equal(EXPAND(PLUS +), "+ +")
        || !equal(EXPAND(1.I), "1.I");
}
