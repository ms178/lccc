#!/usr/bin/env bash
# A sub-word load feeding a widening cast (the `strchr` shape) must NOT
# materialize a relay: the load `movsbl (%r),%r2` and the cast `(I32)(I8)*s`
# coalesce into ONE register, so the char compare reads the loaded register
# directly (`cmpl %c, %r2`) instead of `movl %r2,%eax; movl %eax,%r3;
# cmpl %c,%r3`. Regression for the load->widen-cast coalescing.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat >"$td/test.c" <<'EOF'
char *find_c(const char *s, int c)
{
    while (*s && *s != c)
        ++s;
    return (char *)s;
}
EOF
"$CCC" -m32 -Os -fno-pic -S "$td/test.c" -o "$td/test.s"
body=$(sed -n '/^find_c:/,/^\.size find_c/p' "$td/test.s")
# The loop body must be: `movsbl (%r),%R` then `testl %R,%R` and `cmpl %C,%R`
# with the SAME %R. Reject the relay (`movl %R,%R2` between the load and its
# uses). Identify %R from the byte load.
loadreg=$(grep -E 'movsbl[[:space:]]+\(%e[a-z]+\),[[:space:]]*%e[a-z]+' <<<"$body" \
          | head -1 | sed -E 's/.*,[[:space:]]*%([a-z]+)[[:space:]]*$/\1/')
if [ -z "$loadreg" ]; then
    echo "no byte load found"
    echo "--- $body"
    exit 1
fi
# A relay copies %R to another register immediately after the byte load.
if grep -E "movsbl[[:space:]]+\(%e[a-z]+\),[[:space:]]*%${loadreg}" -A2 <<<"$body" \
   | grep -Eq "movl[[:space:]]+%${loadreg},[[:space:]]*%e[a-z]+"; then
    echo "load->widen-cast relay present: byte load %$loadreg copied before compare"
    echo "--- $body"
    exit 1
fi
# The byte must be tested and compared in place.
if ! grep -Eq "testl[[:space:]]+%${loadreg},[[:space:]]*%${loadreg}" <<<"$body"; then
    echo "expected in-place testl of the loaded byte"
    echo "--- $body"
    exit 1
fi
if ! grep -Eq "cmpl[[:space:]]+%e[a-z]+,[[:space:]]*%${loadreg}" <<<"$body"; then
    echo "expected in-place cmpl of the loaded byte"
    echo "--- $body"
    exit 1
fi
