/* The macros are defined before use, so the alias expansions happen here, at
   level 1: a frontend that remembered the level inside the macro body would
   report 0 for `alias_level` (the level where the body was written), and one
   that expanded `__INCLUDE_LEVEL__` only at the top level would fail to expand
   it inside a body at all. */
#define LEVEL_ALIAS __INCLUDE_LEVEL__
int header_level = __INCLUDE_LEVEL__;
int alias_level = LEVEL_ALIAS;
const char *timestamp_here = __TIMESTAMP__;
