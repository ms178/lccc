/* GNU __auto_type's name enters scope AFTER its initializer (GCC manual,
   "Referring to a Type with typeof"). This is a negative control for the
   ordinary C declarator-scope fix. GCC 16.2 accepts it; Clang 23.1.5 rejects
   the typedef-shaped initializer. Do not silently relabel that disagreement
   as a three-compiler pass; run this fixture against GCC and LCCC. */
typedef long long Inferred;
int main(void) {
    __auto_type Inferred = (Inferred)7;
    return !(Inferred == 7 && sizeof(Inferred) == sizeof(long long));
}
