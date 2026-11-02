/* GNU C nested functions (GCC extension): static chain, captures,
 * trampolines. Pinned runtime behavior for the 7 passing torture shapes:
 * direct calls with captures, address-taken (trampoline) calls, implicit-int
 * definitions, K&R params, multi-level nesting, and sibling calls. */

/* 1. Capture write + argument read through the chain (20000822-1 shape). */
static int f0(int (*fn)(int *), int *p) { return (*fn)(p); }
int capture_write(void) {
    int i = 0;
    int f2(int *p) { i = 1; return *p + 1; }
    return f0(f2, &i);
}

/* 2. Implicit-int definition + VLA-param capture (20010209-1 shape). */
int b_glob = 6;
int vla_param(void) {
    int x[b_glob];
    int bar(int t[b_glob]) {
        int i;
        for (i = 0; i < b_glob; i++)
            t[i] = i + (i > 0 ? t[i - 1] : 0);
        return t[b_glob - 1];
    }
    return bar(x);
}

/* 3. inline nested with enclosing read (20010605-1 shape). */
int inline_nested(void) {
    int v = 42;
    inline int fff(int x) { return x * 10; }
    return fff(v) != 420;
}

/* 4. Enclosing-local write observed after the call (20040520-1 shape). */
int write_then_read(void) {
    int foo = 1;
    int bar(void) {
        int baz = 0;
        if (foo != 45) baz = foo;
        return baz;
    }
    foo = 1;
    return bar() == 1 && foo == 1 ? 0 : 1;
}

/* 5. Loop with counter capture (920612-2 shape). */
int loop_capture(void) {
    int i = 0;
    int a(int x) {
        while (x) i++, x--;
        return x;
    }
    a(2);
    return i;
}

/* 6. Two-level nesting: grandchild captures grandparent's local. */
int two_levels(void) {
    int outer = 10;
    int middle(void) {
        int mid = 5;
        int inner(void) { return outer + mid; } /* both chain levels */
        return inner() + 1;
    }
    return middle(); /* 10 + 5 + 1 */
}

/* 7. Nested function calling a SIBLING nested function. */
int siblings(void) {
    int base = 100;
    int add_base(int x) { return x + base; }
    int twice(int x) { return add_base(add_base(x) - base) - base + x; }
    return twice(3);
}

int main(void) {
    if (capture_write() != 2) return 1;
    if (vla_param() != 15) return 2;
    if (inline_nested() != 0) return 3;
    if (write_then_read() != 0) return 4;
    if (loop_capture() != 2) return 5;
    if (two_levels() != 16) return 6;
    if (siblings() != 6) return 7;
    return 0;
}
