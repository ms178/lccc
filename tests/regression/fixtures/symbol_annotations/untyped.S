/* The negative control of check_symbol_annotations.py: two symbols as an
 * assembly source without `.type` and `.size` defines them, which is how the
 * compiler's own helpers used to reach the symbol table. */
	.data
	.globl bare_object
bare_object:
	.long 1
	.text
	.globl bare_function
bare_function:
	ret
