/* Pragmas of different kinds in a row, and pragmas inside a record body.
 *
 * Two defects, both older than the option pragmas that exposed them.
 *
 * 1. The parser took the pragma markers kind by kind: every `pack`, then
 *    every `visibility`, then the declaration. Two pragmas in the other
 *    order -- `#pragma GCC visibility push(hidden)` and then
 *    `#pragma pack(push, 1)`, as a header that hides and packs its
 *    structures writes them -- left the second for the declaration parser:
 *    "expected declaration before '#pragma pack'".
 *
 * 2. Inside a record body the member loop skipped a pragma marker as an
 *    unknown token. GCC performs the pragma, and gives the record the
 *    packing in force at its closing brace, for all its members:
 *
 *        struct late { char c;
 *        #pragma pack(push, 1)
 *                      int i; char d; };      // 6 bytes, `i` at offset 1
 *
 *    lccc laid it out in 12 bytes with `i` at offset 4 -- another layout
 *    than the GCC-built code it links with -- and the `pop` after the
 *    record found no `push` ("'#pragma pack (pop)' encountered without
 *    matching '#pragma pack (push)'").
 *
 * The numbers are GCC 16.2's. (Clang 23 packs a record by the pragma in
 * force at its opening brace and prints 12 and 16 for `late` and `outer`;
 * lccc follows GCC, whose objects it has to agree with.) */
#include <stddef.h>
#include <stdio.h>

/* Every order of two different kinds, then all three. */
#pragma GCC visibility push(hidden)
#pragma pack(push, 1)
struct vis_pack { char c; int i; };
#pragma pack(pop)
#pragma GCC visibility pop

#pragma GCC push_options
#pragma pack(push, 2)
struct opt_pack { char c; int i; };
#pragma pack(pop)
#pragma GCC pop_options

#pragma pack(push, 1)
#pragma GCC push_options
#pragma GCC visibility push(default)
#pragma pack(4)
#pragma GCC pop_options
#pragma pack(1)
struct all_three { char c; int i; };
#pragma GCC visibility pop
#pragma pack(pop)

struct natural { char c; int i; };

/* A pragma between members: undone before the brace, it changes nothing. */
struct undone {
    char c;
#pragma pack(push, 1)
    int i;
#pragma pack(pop)
    char d;
};

/* In force at the brace, it packs every member, the earlier ones too. */
struct late {
    char c;
#pragma pack(push, 1)
    int i;
    char d;
};
#pragma pack(pop)

/* A nested record is finished at its own brace, before the pragma; and a
 * pragma of another kind may follow as the last thing in the body. */
struct outer {
    char c;
    struct inner { char c; int i; } inner;
#pragma pack(push, 2)
    int i;
#pragma GCC push_options
};
#pragma GCC pop_options
#pragma pack(pop)

struct after { char c; int i; };

int main(void) {
    printf("kinds %zu %zu %zu %zu\n", sizeof(struct vis_pack), sizeof(struct opt_pack),
           sizeof(struct all_three), sizeof(struct natural));
    printf("undone %zu %zu\n", sizeof(struct undone), offsetof(struct undone, i));
    printf("late %zu %zu %zu\n", sizeof(struct late), offsetof(struct late, i),
           offsetof(struct late, d));
    printf("outer %zu %zu %zu inner %zu\n", sizeof(struct outer), offsetof(struct outer, inner),
           offsetof(struct outer, i), sizeof(struct inner));
    printf("after %zu\n", sizeof(struct after));
    return 0;
}
