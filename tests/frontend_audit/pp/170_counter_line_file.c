/* The compiler's own builtins, which are not replacement lists.
   `__INCLUDE_LEVEL__` and `__TIMESTAMP__` were the two conventions lccc did not
   implement (this case used to say so); both oracles implement them and so does
   lccc now: the level of the main file is 0, and the timestamp is the file's
   modification time in ctime form, which is the *same string* for all three
   compilers here because they read the same file and the same zone. */
__LINE__ __FILE__ __COUNTER__ __COUNTER__ __COUNTER__ __BASE_FILE__
__INCLUDE_LEVEL__ __TIMESTAMP__
#define L __LINE__
L
#define LVL __INCLUDE_LEVEL__
LVL
#line 500
__LINE__ L
#line 600 "renamed.c"
__FILE__ __LINE__ __INCLUDE_LEVEL__
__STDC_VERSION__ __STDC_HOSTED__
/* __DATE__ / __TIME__ expand to the wall clock, so a case that compared
   them would fail whenever the three compilers ran in different seconds.
   Their contents are covered by the predefined-macro unit tests and by
   tests/regression/check_preprocessor_builtin_scope.sh, which re-samples
   across a second boundary instead of assuming it never happens. */
__STDC_HOSTED__
