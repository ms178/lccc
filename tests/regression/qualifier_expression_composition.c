/* Legal qualifier composition must preserve values and pointer stepping.
 * The standalone diagnostic matrix covers rejections; this executable pins
 * the corresponding code paths at every optimization level and target. */
static const int values[3] = {11, 23, 37};
struct record { int value; };
static const struct record object = {41};

static const int *offset(void) { return values + 1; }
static const int *member(void) { return &object.value + 0; }
static const int *callback(void) { return values; }
static int sum(const int *p) { return p[0] + p[1]; }

int main(void) {
    const int *p = ((void)0, values);
    const int *q = values[0] ? &values[1] : &values[2];
    const int *(*f)(void) = callback;
    const int (*rows)[3] = &values;
    const int *const pointers[2] = {values, values + 1};
    const int *const *pp = pointers;
    if (p[2] != 37 || q[0] != 23 || offset()[1] != 37 || member()[0] != 41)
        return 1;
    if (f()[0] != 11 || (*rows)[2] != 37 || pp[1][1] != 37 || sum(p) != 34)
        return 2;
    return 0;
}
