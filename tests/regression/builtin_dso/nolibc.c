/* -nostdlib / -nodefaultlibs (check_builtin_dso_link.sh): no default
   library is searched, so puts stays undefined and the link fails, as with
   GCC.  (lccc-ld used to resolve it from libc.so.6 regardless.) */
extern int puts(const char *);
int main(void) { return puts("unreachable"); }
