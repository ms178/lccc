/* Block and for-initializer typedefs must leave outer ordinary identifiers intact. */
typedef int scope_alias;

int main(void) {
    for (typedef long scope_alias; 0; ) {
    }
    scope_alias after_for = 0;
    if (sizeof(after_for) != sizeof(int))
        return 1;

    {
        typedef long scope_alias;
        scope_alias inner = 0;
        if (sizeof(inner) != sizeof(long))
            return 2;
    }

    scope_alias after_block = 0;
    return sizeof(after_block) == sizeof(int) ? 0 : 3;
}
