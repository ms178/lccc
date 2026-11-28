/* A `switch` dispatched through a table, in code that will not move.
 *
 * Where the code is position-fixed (-fno-pic, the kernel code model, and a
 * link the driver knows to be an ordinary executable, which is what
 * `lccc prog.c -o prog` is) the dispatch is one instruction through a table
 * of addresses:
 *
 *         jmpq *.Ljt_0(,%rax,8)
 *     .Ljt_0:
 *         .quad .LBB3
 *         .quad .LBB4
 *
 * The peephole's liveness analysis knew the position-independent form (`leaq
 * .Ljt_0(%rip); movslq; addq; jmpq *%reg` over `.long .LBB3 - .Ljt_0`) and
 * took this one for a jump to an unknown place, which makes a function
 * unanalysable: no liveness-driven peephole ran in any function that held a
 * table, in exactly the builds a benchmark is. `main` of
 * tests/benchmark/programs/sqlite_varint.c kept five register moves that
 * nothing reads in the two loops round its `switch`.
 *
 * Read by check_switch_table_liveness.py: the output against GCC's, built
 * position-fixed and position-independent at every level; and, function by
 * function, the position-fixed code is the position-independent code less
 * the three instructions the dispatch saves. No function here names a
 * global or a string, so the two differ in the dispatch and in nothing else.
 */
#include <stdint.h>
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

/* What the arms hand to and take from: calls the caller's registers live
 * across, and results that are narrowed and widened on the way. */
NOINLINE unsigned char pack(uint64_t v, unsigned shift) {
    return (unsigned char)((v >> (shift & 63)) ^ (v * 0x9e3779b97f4a7c15ull >> 56));
}

NOINLINE int emit(unsigned char *out, uint64_t v) {
    int n = 0;
    do {
        out[n++] = (unsigned char)(v | 0x80);
        v >>= 7;
    } while (v != 0 && n < 9);
    out[n - 1] &= 0x7f;
    return n;
}

/* sqlite_varint's corpus loop: a value built by a `switch` on the index,
 * then a call whose result is narrowed, widened and added in. */
NOINLINE uint64_t dispatch_build_values(unsigned char *out, unsigned count, unsigned state) {
    uint64_t sum = 0;
    unsigned used = 0;
    for (unsigned i = 0; i < count; i++) {
        uint64_t value;
        state = state * 1664525u + 1013904223u;
        switch (i % 9u) {
        case 0: value = state & 0x7fu; break;
        case 1: value = 0x80u | (state & 0x3fffu); break;
        case 2: value = 0x4000u | (state & 0x1fffffu); break;
        case 3: value = 0x200000u | (state & 0xfffffffu); break;
        case 4: value = 0x10000000ull | ((uint64_t)state << 3); break;
        case 5: value = 0x800000000ull | ((uint64_t)state << 10); break;
        case 6: value = 0x40000000000ull | ((uint64_t)state << 17); break;
        case 7: value = 0x2000000000000ull | ((uint64_t)state << 24); break;
        default: value = 0x8000000000000000ull | ((uint64_t)state << 25) | i; break;
        }
        unsigned char tag = pack(value, i);
        used += (unsigned)emit(out + (used & 63u), value);
        sum += value ^ ((uint64_t)tag << (i & 31u));
    }
    return sum + used;
}

/* A bytecode loop: the dispatch is the loop. */
NOINLINE uint64_t dispatch_run_program(const unsigned char *code, unsigned len, uint64_t acc) {
    uint64_t stack[8] = {0};
    unsigned sp = 0;
    for (unsigned pc = 0; pc < len; pc++) {
        unsigned char op = code[pc];
        switch (op & 15u) {
        case 0: acc += op >> 4; break;
        case 1: acc -= op >> 4; break;
        case 2: acc *= 3; break;
        case 3: acc ^= acc >> 7; break;
        case 4: stack[sp++ & 7u] = acc; break;
        case 5: acc += stack[--sp & 7u]; break;
        case 6: acc = 0 - acc; break;
        case 7: acc <<= (op >> 4) & 7; break;
        case 8: acc >>= (op >> 4) & 7; break;
        case 9: acc |= (uint64_t)op << 8; break;
        case 10: acc &= ~(uint64_t)(op >> 4); break;
        case 11: acc += pack(acc, op); break;
        default: acc += pc; break;
        }
    }
    return acc + sp;
}

/* A lexer's state machine: the state is the index, each arm picks the next
 * state from the byte. */
NOINLINE unsigned dispatch_count_tokens(const unsigned char *text, unsigned len) {
    unsigned state = 0, tokens = 0, digits = 0, depth = 0;
    for (unsigned i = 0; i < len; i++) {
        unsigned char c = text[i];
        switch (state) {
        case 0:
            if (c >= '0' && c <= '9') { state = 1; digits++; }
            else if (c >= 'a' && c <= 'z') { state = 2; }
            else if (c == '"') { state = 3; }
            else if (c == '(') { depth++; }
            else if (c == ')') { depth -= depth != 0; }
            break;
        case 1:
            if (c >= '0' && c <= '9') { digits++; }
            else if (c == '.') { state = 4; }
            else { state = 0; tokens++; }
            break;
        case 2:
            if (!((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9'))) { state = 0; tokens++; }
            break;
        case 3:
            if (c == '\\') { state = 5; }
            else if (c == '"') { state = 0; tokens++; }
            break;
        case 4:
            if (c >= '0' && c <= '9') { digits++; }
            else { state = 0; tokens += 2; }
            break;
        case 5:
            state = 3;
            break;
        default:
            state = 0;
            break;
        }
    }
    return tokens * 1000003u + digits * 31u + depth + state;
}

/* A `switch` whose arms return: no loop, values live across the dispatch
 * in each arm's own way. */
NOINLINE uint64_t dispatch_combine(unsigned kind, uint64_t a, uint64_t b, uint64_t c) {
    switch (kind) {
    case 0: return a + b;
    case 1: return a - c;
    case 2: return b * c;
    case 3: return (a ^ b) + c;
    case 4: return a << (b & 31);
    case 5: return c >> (a & 31);
    case 6: return pack(a, (unsigned)b) + c;
    case 7: return a & ~b;
    case 8: return (a | b) ^ c;
    default: return a + b + c;
    }
}

/* Two tables in one function, the second behind a call. */
NOINLINE uint64_t dispatch_twice(unsigned first, unsigned second, uint64_t x) {
    uint64_t y;
    switch (first) {
    case 0: y = x + 1; break;
    case 1: y = x * 5; break;
    case 2: y = x ^ 0x55; break;
    case 3: y = x >> 3; break;
    case 4: y = x << 2; break;
    case 5: y = ~x; break;
    default: y = x; break;
    }
    unsigned char tag = pack(y, second);
    switch (second) {
    case 0: return y + tag;
    case 1: return y - tag;
    case 2: return y ^ tag;
    case 3: return y * (tag | 1u);
    case 4: return y | tag;
    case 5: return y & ~(uint64_t)tag;
    default: return y + x + tag;
    }
}

static uint64_t state = 0x243f6a8885a308d3ull;

static uint32_t next(void) {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    return (uint32_t)(state >> 9);
}

int main(void) {
    static const char alphabet[] = "0123456789abcxyz \"\\().,";
    unsigned char buffer[160];
    unsigned char out[96];
    uint64_t sum = 0xcbf29ce484222325ull;

    for (unsigned round = 0; round < 400; round++) {
        unsigned len = next() % 150u;
        for (unsigned i = 0; i < len; i++) {
            buffer[i] = (unsigned char)next();
        }
        unsigned seed = next();
        sum = (sum ^ dispatch_build_values(out, 1u + len % 40u, seed)) * 0x100000001b3ull;
        uint64_t acc = ((uint64_t)next() << 20) ^ next();
        sum = (sum ^ dispatch_run_program(buffer, len, acc)) * 0x100000001b3ull;
        for (unsigned i = 0; i < len; i++) {
            buffer[i] = (unsigned char)alphabet[next() % (sizeof alphabet - 1)];
        }
        sum = (sum ^ dispatch_count_tokens(buffer, len)) * 0x100000001b3ull;
        for (unsigned kind = 0; kind < 12; kind++) {
            uint64_t a = next();
            uint64_t b = next();
            uint64_t c = ((uint64_t)next() << 32) | next();
            sum = (sum ^ dispatch_combine(kind, a, b, c)) * 0x100000001b3ull;
            sum = (sum ^ dispatch_twice(kind % 8u, (kind * 5u) % 9u, c)) * 0x100000001b3ull;
        }
    }
    printf("switch tables %016llx\n", (unsigned long long)sum);
    return 0;
}
