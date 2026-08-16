/* Verify __builtin_object_size folds string literals (fortify high-ROI path). */
#include <stddef.h>

int main(void) {
    if (__builtin_object_size("hello", 0) != 6) return 1;
    if (__builtin_object_size("hello", 1) != 6) return 2;
    if (__builtin_object_size("hello", 2) != 6) return 3;
    if (__builtin_object_size("hello", 3) != 6) return 4;
    if (__builtin_object_size("", 0) != 1) return 5;
    if (__builtin_object_size("x", 0) != 2) return 6;
    if (__builtin_object_size("abcdef", 0) != (long)sizeof("abcdef")) return 7;

    char *p = (char *)0;
    if (__builtin_object_size(p, 0) != (size_t)-1) return 9;
    if (__builtin_object_size(p, 2) != 0) return 10;
    return 0;
}
