/* C11 6.2.3: objects and enumerators share the ordinary-identifier
 * namespace, and the innermost declaration wins.  glibc's fnmatch.c has the
 * first shape: an enum diagnostic_t enumerator named `wstring` and a local
 * `struct scratch_buffer wstring`. */
enum diagnostic_t { shadowed = 7 };

struct payload {
    int value;
};

static int local_object_shadows_file_enumerator(void)
{
    struct payload shadowed = { 35 };
    return shadowed.value;
}

static int file_object = 11;

static int local_enumerator_shadows_file_object(void)
{
    enum { file_object = 31 };
    return file_object;
}

static int nested_object_shadows_function_enumerator(void)
{
    enum { name = 3 };
    {
        struct payload name = { 23 };
        return name.value;
    }
}

int main(void)
{
    return local_object_shadows_file_enumerator()
                + local_enumerator_shadows_file_object()
                + nested_object_shadows_function_enumerator()
            == 89
        ? 0
        : 1;
}
