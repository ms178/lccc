/* TAG-ID-1: nested same-tag struct definitions shared one type identity.
 *
 * Status: FIXED in this tree.  This file is documentation only -- nothing in
 * the build or the test suite consumes tests/bugs/.  The behaviour is pinned
 * by tests/regression/check_record_tag_identity.sh, a 16-row differential
 * against the host GCC run from `ci_local.sh --fast` and hosted CI; the
 * mechanism and the differential matrix live in backlog.md under
 * "TAG-ID-1 - FIXED".  Kept short on purpose: a long narrative here drifted
 * out of date the moment the fix landed and started contradicting the code.
 *
 * The defect, in one line: struct/union types were keyed by tag alone while
 * struct LAYOUTS were scope-aware, so an inner `struct S { int c; }` and an
 * outer `struct S { char c; }` were the same CType.  The callee read the
 * argument with the outer 1-byte layout while the caller built it with the
 * inner 4-byte one -- 300 is 0x12C and only the low byte was read, so this
 * printed 44.  A silent miscompile, not a missing diagnostic.
 *
 * It took three changes, because two downstream gaps each masked the one
 * above: a distinct key per shadowing definition, a scoped record_alias map so
 * every tag->CType site resolves to the live variant, and a record-type
 * comparison in check_call_arguments, which previously compared only arity and
 * pointer/float mixing.  The third is the lesson: a type-identity fix is inert
 * until something consumes the identity.
 *
 * Expected now, and what the gate asserts:
 *
 *     $ lccc -std=c23 tests/bugs/nested_tag_struct_identity.c -o t
 *     error: incompatible type for argument to 'f': 'struct S' here is a
 *            different type from the 'struct S' of the parameter, ...
 *     $ gcc -std=c23 -c tests/bugs/nested_tag_struct_identity.c
 *     error: incompatible type for argument 1 of 'f'
 *
 * Both reject, and lccc produces no binary.
 *
 * Residual, asserted by the gate so it cannot widen silently: a positive N3037
 * case under -std=c17 is accepted by lccc and rejected by GCC.  Definitions
 * with corresponding members share a key, which is what C23 wants and is too
 * permissive pre-C23.  It cannot miscompile -- corresponding members means
 * identical layouts -- and closing it needs the C standard threaded into sema,
 * which has no notion of -std today.
 */
#include <stdio.h>

struct S { char c; };
unsigned f(struct S p) { return (unsigned)(unsigned char)p.c; }

int main(void) {
    struct S { int c; } s = { 300 };   /* C 6.7.2.3: a NEW type, incompatible */
    printf("%u\n", f(s));              /* correct answer: 300; lccc prints 44 */
    return 0;
}
