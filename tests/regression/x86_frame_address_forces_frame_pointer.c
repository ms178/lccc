/* __builtin_frame_address(0) requires a real frame record even under
 * -fomit-frame-pointer. A stale caller-owned rbp value is not sufficient. */
#include <stdlib.h>

__attribute__((noinline))
static int frame_is_local(const char *caller_local)
{
    char local;
    const char *frame = __builtin_frame_address(0);
    if (!frame)
        return 0;
    if (caller_local < &local)
        return caller_local <= frame && frame <= &local;
    return &local <= frame && frame <= caller_local;
}

int main(void)
{
    char anchor;
    if (!frame_is_local(&anchor))
        abort();
    return 0;
}
