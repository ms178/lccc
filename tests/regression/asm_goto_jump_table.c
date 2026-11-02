/* asm goto label targets must survive backend block renumbering.
 *
 * The driver renumbers every basic block to a globally unique id before
 * codegen. It rewrote block labels and terminator targets but NOT the BlockIds
 * stored inside instructions, so an `asm goto` kept pointing at the pre-
 * renumber id: `%l[label]` expanded to a `.LBBn` that was never defined. The
 * assembler then emitted a relocation against a null symbol, and the Linux
 * kernel's objtool rejected every object containing a static branch with
 * "special: can't find new instruction".
 *
 * This is the exact shape of the kernel's static-key machinery
 * (arch/x86/include/asm/jump_label.h): a jump to an asm goto label plus a
 * __jump_table entry holding `.long <label> - .`, which forces the label to be
 * materialized as a real symbol rather than folded into a branch displacement.
 *
 * The test asserts both halves: control flow reaches the label, and the
 * recorded self-relative offset actually points back at the jump target.
 */
#include <stdio.h>
#include <string.h>

struct jt_entry {
	int code;   /* &&label - &entry.code  */
	int target;
	long key;
};

static long some_key;

static inline int branch_taken(void)
{
	asm goto("1:\n\t"
		 "jmp %l[l_yes]\n\t"
		 ".pushsection .test_jump_table, \"aw\"\n\t"
		 ".balign 8\n\t"
		 ".long 1b - .\n\t"
		 ".long %l[l_yes] - .\n\t"
		 ".quad %c0 - .\n\t"
		 ".popsection\n\t"
		 : : "i"(&some_key) : : l_yes);
	return 0;
l_yes:
	return 1;
}

/* A second site in the same function: renumbering must keep both distinct. */
static inline int branch_two(void)
{
	asm goto("2:\n\t"
		 "jmp %l[other]\n\t"
		 ".pushsection .test_jump_table, \"aw\"\n\t"
		 ".balign 8\n\t"
		 ".long 2b - .\n\t"
		 ".long %l[other] - .\n\t"
		 ".quad %c0 - .\n\t"
		 ".popsection\n\t"
		 : : "i"(&some_key) : : other);
	return 0;
other:
	return 2;
}

int main(void)
{
	/* Control flow must actually reach the asm goto targets. */
	if (branch_taken() != 1) {
		printf("FAIL branch_taken\n");
		return 1;
	}
	if (branch_two() != 2) {
		printf("FAIL branch_two\n");
		return 2;
	}

	/* Exercise the label in a loop so the block is not trivially laid out
	 * immediately after the asm. */
	int hits = 0;
	for (int i = 0; i < 4; i++)
		hits += branch_taken();
	if (hits != 4) {
		printf("FAIL loop hits=%d\n", hits);
		return 3;
	}

	printf("PASS asm_goto_jump_table\n");
	return 0;
}
