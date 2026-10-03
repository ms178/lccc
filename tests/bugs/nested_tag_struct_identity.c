/* BUG: nested same-tag struct definitions share one type identity -> miscompile
 *
 * Status: OPEN (frontend/sema). Filed from the EDG Changes-distillation
 * campaign, entry "C23: New tag compatibility rules" (N3037), the highest
 * scored C-relevant entry in docs/edg_changes_c_extract.md (*C-score +10*).
 *
 * WHAT HAPPENS
 * ------------
 * lccc keys struct/union types by tag alone -- CType::Struct("struct.S") --
 * while struct LAYOUTS are scope-aware: TypeContext::insert_struct_layout_
 * scoped_from_ref() records the shadowed layout in the scope frame and
 * restores it on pop_scope(). So an inner-scope definition of `struct S`
 * silently replaces the layout that the outer `struct S` refers to, and the
 * two declarations are not distinguishable as types anywhere downstream.
 * The callee then reads the argument with the outer (1-byte) layout while the
 * caller built it with the inner (4-byte) one.
 *
 * OBSERVED (lccc built from this tree, x86-64, -std=c23):
 *
 *     $ lccc -std=c23 nested_tag_struct_identity.c -o t && ./t
 *     44                  <-- WRONG
 *     $ gcc -std=c23 -c nested_tag_struct_identity.c
 *     error: incompatible type for argument 1 of 'f'   <-- correct: rejected
 *
 * 300 is 0x12C; the callee reads only the low byte, 0x2C = 44. Silent
 * truncation through an incompatible struct type -- a miscompile, not merely
 * a missing diagnostic.
 *
 * DIFFERENTIAL MATRIX vs GCC 16.2 (`-fsyntax-only`, expect = GCC's verdict):
 *
 *     case                              gcc      lccc    agree
 *     positive N3037 (members match)    accept   accept  yes
 *     member TYPE differs               REJECT   accept  NO
 *     member NAME differs               REJECT   accept  NO
 *     member COUNT differs              REJECT   accept  NO
 *     bit-field WIDTH differs           REJECT   accept  NO
 *     bit-field SIGNEDNESS differs      REJECT   accept  NO
 *     enumerator VALUE differs          accept   accept  yes
 *
 * And by language mode, on the positive case alone:
 *
 *     -std=   gcc      lccc
 *     c11     REJECT   accept
 *     c17     REJECT   accept
 *     c23     accept   accept
 *
 * So the defect is broader than C23: C 6.7.2.3 makes an inner-scope definition
 * a NEW type that is incompatible with the outer one, and lccc accepts the
 * mixture in every mode. It is only accidentally right in C23, and only when
 * the members happen to correspond.
 *
 * ROOT CAUSE (exact sites)
 * ------------------------
 * - src/frontend/sema/analysis.rs  resolve_struct_or_union():
 *       let key = format!("{}.{}", prefix, tag);
 *   one key per tag, no per-definition identity. Anonymous structs already get
 *   a unique id via next_anon_struct_id(), so the mechanism exists.
 * - src/frontend/sema/analysis.rs  pointee_types_compatible():
 *       (CType::Struct(ka), CType::Struct(kb)) => ka == kb,
 *   compatibility decided by tag equality alone.
 * - src/frontend/sema/type_context.rs  struct_layouts + TypeScopeFrame::
 *   struct_layouts_shadowed: layouts are scoped, identities are not.
 *
 * WHY IT IS NOT A ONE-LINE FIX
 * ----------------------------
 * The tag-keyed form is assumed across layers. src/ir/lowering/ rebuilds keys
 * by hand from the AST tag -- expr_access.rs, stmt.rs, structs.rs,
 * const_eval.rs all do format!("struct.{}", tag) -- and TypeSpecifier carries
 * the raw tag while CType carries the prefixed key. Introducing a
 * per-definition discriminator therefore needs a single resolver used by every
 * one of those sites, not just a change in sema; done piecemeal it would make
 * lowering fail to find layouts. There are three resolve_struct_or_union
 * implementations (common/type_builder.rs trait, ir/lowering/types_ctype.rs,
 * frontend/sema/analysis.rs) reached through the TypeBuilder trait.
 *
 * PLAN
 * ----
 * 1. TypeContext: map base key -> stack of per-definition keys, with a
 *    resolve_struct_key() returning the innermost live one; push/pop with the
 *    existing scope frames so shadowing still unwinds.
 * 2. Replace every hand-built format!("struct.{}", tag) / "union." lookup with
 *    that resolver (one helper, ~10 call sites in src/ir/lowering/).
 * 3. sema: mint a distinct key for a definition that shadows a visible one;
 *    keep the bare tag key for file-scope declarations and forward
 *    declarations so 6.7.2.3p2 (same tag at file scope = same type) is
 *    preserved.
 * 4. C23 only: add the N3037 compatibility relation -- same tag, members
 *    correspond by name, type, bit-field width and signedness, enumerators by
 *    value -- and treat corresponding types as compatible. Pre-C23 they stay
 *    incompatible, which makes cases 2-6 above diagnose.
 * 5. Turn each row of the matrix into a tests/regression/ case with the GCC
 *    verdict as the expectation, so the differential is pinned and cannot
 *    regress silently.
 *
 * Interim mitigation to consider (NOT yet implemented): diagnosing an
 * inner-scope definition whose layout differs from the visible one would trade
 * a rare valid C17 pattern for the loss of a silent miscompile. Deliberately
 * not done here -- step 1-4 is the correct fix and does not reject valid code.
 */
#include <stdio.h>

struct S { char c; };
unsigned f(struct S p) { return (unsigned)(unsigned char)p.c; }

int main(void) {
    struct S { int c; } s = { 300 };   /* C 6.7.2.3: a NEW type, incompatible */
    printf("%u\n", f(s));              /* correct answer: 300; lccc prints 44 */
    return 0;
}
