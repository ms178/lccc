/* ACPICA name-string out-parameter: the store to *out_name_string must not be
 * dropped on any path that returns AE_OK.
 *
 * Provenance: a Linux 6.18.52-cachymod kernel built entirely with LCCC booted
 * in QEMU and, on the ACPI error path provoked by SeaBIOS' DSDT, warned:
 *
 *   WARNING: CPU: 1 PID: 1 at free_large_kmalloc+0x99/0x190
 *    kfree+0xb6/0x370
 *    acpi_ds_create_operand+0x26c/0x580
 *   page dumped because: Not a kmalloc allocation   (pfn:0x173b, refcount:1)
 *
 * Captured live over the QEMU gdbstub (scripts/qemu_gdbstub_probe.py), the
 * pointer handed to kfree() was 0xffffffff8173b8c6 — the instruction after
 * `call acpi_ut_track_stack_ptr` inside acpi_ut_trace_ptr, i.e. a *return
 * address* read off the stack, not a pointer anyone allocated.
 *
 * The chain is fully pinned down:
 *   acpi_ds_create_operand()   passes  lea -0x38(%rbp),%rdx  as out_name_string
 *                              frees    mov -0x38(%rbp),%r9; mov %r9,%rdi; kfree
 *                              -> store target and load source agree (correct)
 *   status check               mov %eax,%r14d; test %r14d,%r14d; je  (correct,
 *                              so the free is not on a failure path)
 *   acpi_ex_get_name_string()  success epilogue is
 *                              mov -0x40(%rbp),%rcx; mov %r13,(%rcx)
 *   -> the only way the caller sees garbage on an AE_OK return is a lost or
 *      misdirected store of the allocated name_string.
 *
 * This test reproduces that contract in isolation. Three things make it a
 * faithful reproduction rather than a plausible-looking one:
 *
 *  1. The functions are extracted verbatim from drivers/acpi/acpica/exnames.c
 *     (v6.18), including the `while (num_segments && (status = ...))` shape
 *     and the `acpi_ut_valid_name_char(c, 0)` position-0 quirk that makes '_'
 *     an invalid segment character.
 *  2. The kernel config has CONFIG_ACPI_DEBUG=y, so ACPI_FUNCTION_TRACE*,
 *     ACPI_DEBUG_PRINT, ACPI_ERROR and return_PTR/return_ACPI_STATUS are NOT
 *     no-ops — they are real calls, and the garbage that leaked was literally
 *     a trace function's return address. The stubs here are therefore
 *     __attribute__((noinline)) real calls with side effects, not macros that
 *     vanish.
 *  3. The caller mirrors acpi_ds_create_operand: an *uninitialized* `char
 *     *name_string`, a status check, then an unconditional free.
 *
 * Detection is exact rather than probabilistic: acpi_os_allocate() hands out
 * from a private arena, so after an AE_OK return the out-parameter is either
 * an arena pointer (contract honoured) or it is not (store lost). The string
 * contents and consumed length are checked too, so a store of the *wrong*
 * value fails as well.
 *
 * Expected: identical output from LCCC and GCC, every case "ok". */

#include <stdio.h>
#include <string.h>
#include <stdlib.h>

typedef unsigned char u8;
typedef unsigned int u32;
typedef unsigned int acpi_status;
typedef unsigned int acpi_object_type;

/* AML encoding constants (amlcode.h). */
#define AML_ROOT_PREFIX          0x5C	/* '\' */
#define AML_PARENT_PREFIX        0x5E	/* '^' */
#define AML_DUAL_NAME_PREFIX     0x2E
#define AML_MULTI_NAME_PREFIX    0x2F
#define ACPI_NAMESEG_SIZE        4
#define ACPI_UINT32_MAX          ((u32)0xFFFFFFFF)
#define FALSE                    0
#define TRUE                     1

/* Status codes: only the AE_OK == 0 / ACPI_FAILURE(a) == (a) relationship is
 * load-bearing here, so the AML-specific numbers need not match acexcep.h. */
#define AE_OK                    ((acpi_status)0x0000)
#define AE_ERROR                 ((acpi_status)0x0001)
#define AE_NO_MEMORY             ((acpi_status)0x0002)
#define AE_AML_BAD_NAME          ((acpi_status)0x3006)
#define AE_CTRL_PENDING          ((acpi_status)0x2005)
#define ACPI_SUCCESS(a)          (!(a))
#define ACPI_FAILURE(a)          (a)

/* Object types; only the three field types take the first branch. */
#define ACPI_TYPE_ANY            0x00
#define ACPI_TYPE_LOCAL_REGION_FIELD 0x10
#define ACPI_TYPE_LOCAL_BANK_FIELD   0x11
#define ACPI_TYPE_LOCAL_INDEX_FIELD  0x12

/* ------------------------------------------------------------------ *
 * Trace / error / allocation stubs.
 *
 * noinline + external side effect: in the kernel these are real calls
 * (acpi_ut_trace_ptr, acpi_debug_print, acpi_ut_status_exit, ...) and the
 * leaked value was one of their return addresses. If the compiler folded them
 * away the reproduction would not exercise the same register pressure or the
 * same call boundaries, and the defect would hide.
 * ------------------------------------------------------------------ */

static volatile unsigned long trace_calls;
static volatile unsigned long exit_calls;
static volatile unsigned long alloc_calls;
static volatile unsigned long free_calls;

/* Counters must stay address-free: the suite compares stdout byte for byte
 * against the GCC oracle and against a second LCCC configuration, so folding a
 * pointer value in would make the output ASLR-dependent and the comparison
 * meaningless. Only the (stable) function-name tag and status are mixed in. */
__attribute__((noinline)) static void acpi_ut_trace_ptr(const char *fn,
							const void *ptr)
{
	(void)ptr;
	trace_calls += (unsigned long)fn[0] + (unsigned long)fn[1];
}

__attribute__((noinline)) static void acpi_ut_trace(const char *fn)
{
	trace_calls += (unsigned long)fn[0];
}

__attribute__((noinline)) static void acpi_ut_status_exit(const char *fn,
							  acpi_status st)
{
	exit_calls += (unsigned long)st + (unsigned long)fn[0];
}

__attribute__((noinline)) static void acpi_ut_ptr_exit(const char *fn,
						       const void *ptr)
{
	(void)ptr;
	exit_calls += (unsigned long)fn[0] + (unsigned long)fn[1];
}

/* Variadic, like the real acpi_debug_print / acpi_ut_error. */
__attribute__((noinline)) static void acpi_debug_print(const char *fmt, ...)
{
	trace_calls += (unsigned long)fmt[0];
}

__attribute__((noinline)) static void acpi_ut_error(const char *fmt, ...)
{
	trace_calls += (unsigned long)fmt[0] + 1u;
}

#define ACPI_FUNCTION_TRACE(a)            acpi_ut_trace(#a)
#define ACPI_FUNCTION_TRACE_PTR(a, b)     acpi_ut_trace_ptr(#a, (const void *)(b))
#define return_ACPI_STATUS(s) \
	do { acpi_ut_status_exit(__func__, (s)); return (s); } while (0)
#define return_PTR(p) \
	do { acpi_ut_ptr_exit(__func__, (const void *)(p)); return (p); } while (0)
#define ACPI_DEBUG_PRINT(a)               acpi_debug_print a
#define ACPI_ERROR(a)                     acpi_ut_error a
#define AE_INFO                           __func__, __LINE__

/* ------------------------------------------------------------------ *
 * Arena allocator: makes "is this the pointer we allocated?" exact.
 * ------------------------------------------------------------------ */

#define ARENA_BYTES 65536
static char arena[ARENA_BYTES];
static size_t arena_used;
static unsigned long arena_live;

static int arena_owns(const void *p)
{
	return p >= (const void *)arena &&
	       p < (const void *)(arena + ARENA_BYTES);
}

__attribute__((noinline)) static void *acpi_os_allocate(size_t size)
{
	void *p;

	alloc_calls++;
	if (size == 0 || arena_used + size + 8 > ARENA_BYTES)
		return NULL;
	arena_used += (8 - (arena_used & 7)) & 7;	/* align to 8 */
	p = arena + arena_used;
	arena_used += size;
	arena_live++;
	memset(p, 0xA5, size);	/* poison: an unwritten buffer is visible */
	return p;
}

__attribute__((noinline)) static void acpi_os_free(void *p)
{
	free_calls++;
	/* Freeing a non-arena pointer is exactly the kernel's WARN. */
	if (!arena_owns(p)) {
		printf("FREE-OF-FOREIGN-POINTER\n");
		exit(3);
	}
	arena_live--;
}

#define ACPI_ALLOCATE(s)          acpi_os_allocate((size_t)(s))
#define ACPI_FREE(p)              acpi_os_free((void *)(p))
#define ACPI_CAST_PTR(t, p)       ((t *)(void *)(p))

/* Verbatim from drivers/acpi/acpica/utascii.c:60 — note it accepts '_'
 * unconditionally and does NOT accept lower case, and that '!' is allowed only
 * in the last position. acpi_ex_name_segment always calls it with position 0,
 * so the '!' arm is dead there, but it is kept for fidelity: getting this
 * predicate wrong silently changes which AML inputs take the failure paths,
 * which is the whole point of the test. */
__attribute__((noinline)) static u8 acpi_ut_valid_name_char(char character,
							    u32 position)
{
	if (!((character >= 'A' && character <= 'Z') ||
	      (character >= '0' && character <= '9') || (character == '_'))) {

		/* Allow a '!' in the last position */

		if (character == '!' && position == 3) {
			return (TRUE);
		}

		return (FALSE);
	}

	return (TRUE);
}

/* ================= extracted verbatim from exnames.c ================= */

static char *acpi_ex_allocate_name_string(u32 prefix_count, u32 num_name_segs)
{
	char *temp_ptr;
	char *name_string;
	u32 size_needed;

	ACPI_FUNCTION_TRACE(ex_allocate_name_string);

	if (prefix_count == ACPI_UINT32_MAX) {

		/* Special case for root */

		size_needed = 1 + (ACPI_NAMESEG_SIZE * num_name_segs) + 2 + 1;
	} else {
		size_needed =
		    prefix_count + (ACPI_NAMESEG_SIZE * num_name_segs) + 2 + 1;
	}

	name_string = ACPI_ALLOCATE(size_needed);
	if (!name_string) {
		ACPI_ERROR((AE_INFO, "Could not allocate size %u", size_needed));
		return_PTR(NULL);
	}

	temp_ptr = name_string;

	if (prefix_count == ACPI_UINT32_MAX) {
		*temp_ptr++ = AML_ROOT_PREFIX;
	} else {
		while (prefix_count--) {
			*temp_ptr++ = AML_PARENT_PREFIX;
		}
	}

	if (num_name_segs > 2) {
		*temp_ptr++ = AML_MULTI_NAME_PREFIX;
		*temp_ptr++ = (char)num_name_segs;
	} else if (2 == num_name_segs) {
		*temp_ptr++ = AML_DUAL_NAME_PREFIX;
	}

	*temp_ptr = 0;

	return_PTR(name_string);
}

static acpi_status acpi_ex_name_segment(u8 ** in_aml_address, char *name_string)
{
	char *aml_address = (void *)*in_aml_address;
	acpi_status status = AE_OK;
	u32 index;
	char char_buf[5];

	ACPI_FUNCTION_TRACE(ex_name_segment);

	char_buf[0] = *aml_address;

	if ('0' <= char_buf[0] && char_buf[0] <= '9') {
		ACPI_ERROR((AE_INFO, "Invalid leading digit: %c", char_buf[0]));
		return_ACPI_STATUS(AE_CTRL_PENDING);
	}

	for (index = 0;
	     (index < ACPI_NAMESEG_SIZE)
	     && (acpi_ut_valid_name_char(*aml_address, 0)); index++) {
		char_buf[index] = *aml_address++;
	}

	if (index == 4) {

		char_buf[4] = '\0';

		if (name_string) {
			ACPI_DEBUG_PRINT(("Appending NameSeg %s\n", char_buf));
			strcat(name_string, char_buf);
		} else {
			ACPI_DEBUG_PRINT(("No Name string - %s\n", char_buf));
		}
	} else if (index == 0) {
		ACPI_DEBUG_PRINT(("Leading character is not alpha: %02Xh\n",
				  char_buf[0]));
		status = AE_CTRL_PENDING;
	} else {
		status = AE_AML_BAD_NAME;
		ACPI_ERROR((AE_INFO, "Bad character 0x%02x in name, at %p",
			    *aml_address, aml_address));
	}

	*in_aml_address = ACPI_CAST_PTR(u8, aml_address);
	return_ACPI_STATUS(status);
}

acpi_status
acpi_ex_get_name_string(acpi_object_type data_type,
			u8 * in_aml_address,
			char **out_name_string, u32 * out_name_length)
{
	acpi_status status = AE_OK;
	u8 *aml_address = in_aml_address;
	char *name_string = NULL;
	u32 num_segments;
	u32 prefix_count = 0;
	u8 has_prefix = FALSE;

	ACPI_FUNCTION_TRACE_PTR(ex_get_name_string, aml_address);

	if (ACPI_TYPE_LOCAL_REGION_FIELD == data_type ||
	    ACPI_TYPE_LOCAL_BANK_FIELD == data_type ||
	    ACPI_TYPE_LOCAL_INDEX_FIELD == data_type) {

		name_string = acpi_ex_allocate_name_string(0, 1);
		if (!name_string) {
			status = AE_NO_MEMORY;
		} else {
			status =
			    acpi_ex_name_segment(&aml_address, name_string);
		}
	} else {
		switch (*aml_address) {
		case AML_ROOT_PREFIX:

			ACPI_DEBUG_PRINT(("RootPrefix(\\) at %p\n", aml_address));

			aml_address++;
			prefix_count = ACPI_UINT32_MAX;
			has_prefix = TRUE;
			break;

		case AML_PARENT_PREFIX:

			do {
				ACPI_DEBUG_PRINT(("ParentPrefix (^) at %p\n",
						  aml_address));

				aml_address++;
				prefix_count++;

			} while (*aml_address == AML_PARENT_PREFIX);

			has_prefix = TRUE;
			break;

		default:
			break;
		}

		switch (*aml_address) {
		case AML_DUAL_NAME_PREFIX:

			ACPI_DEBUG_PRINT(("DualNamePrefix at %p\n", aml_address));

			aml_address++;
			name_string =
			    acpi_ex_allocate_name_string(prefix_count, 2);
			if (!name_string) {
				status = AE_NO_MEMORY;
				break;
			}

			has_prefix = TRUE;

			status =
			    acpi_ex_name_segment(&aml_address, name_string);
			if (ACPI_SUCCESS(status)) {
				status =
				    acpi_ex_name_segment(&aml_address,
							 name_string);
			}
			break;

		case AML_MULTI_NAME_PREFIX:

			ACPI_DEBUG_PRINT(("MultiNamePrefix at %p\n", aml_address));

			aml_address++;
			num_segments = *aml_address;

			name_string =
			    acpi_ex_allocate_name_string(prefix_count,
							 num_segments);
			if (!name_string) {
				status = AE_NO_MEMORY;
				break;
			}

			aml_address++;
			has_prefix = TRUE;

			while (num_segments &&
			       (status =
				acpi_ex_name_segment(&aml_address,
						     name_string)) == AE_OK) {
				num_segments--;
			}

			break;

		case 0:

			if (prefix_count == ACPI_UINT32_MAX) {
				ACPI_DEBUG_PRINT(("NameSeg is \"\\\" followed by NULL\n"));
			}

			aml_address++;
			name_string =
			    acpi_ex_allocate_name_string(prefix_count, 0);
			if (!name_string) {
				status = AE_NO_MEMORY;
				break;
			}

			break;

		default:

			name_string =
			    acpi_ex_allocate_name_string(prefix_count, 1);
			if (!name_string) {
				status = AE_NO_MEMORY;
				break;
			}

			status =
			    acpi_ex_name_segment(&aml_address, name_string);
			break;
		}
	}

	if (AE_CTRL_PENDING == status && has_prefix) {

		ACPI_ERROR((AE_INFO, "Malformed Name at %p", name_string));
		status = AE_AML_BAD_NAME;
	}

	if (ACPI_FAILURE(status)) {
		if (name_string) {
			ACPI_FREE(name_string);
		}
		return_ACPI_STATUS(status);
	}

	*out_name_string = name_string;
	*out_name_length = (u32) (aml_address - in_aml_address);

	return_ACPI_STATUS(status);
}

/* ===================== end of extracted code ===================== */

/* The ACPICA name string embeds raw prefix bytes (0x2F/0x2E/segment count),
 * so print it escaped: the oracle comparison is on stdout, and non-printable
 * bytes there make a diff unreadable. */
static char shown[128];

static const char *show(const char *s)
{
	size_t o = 0;
	size_t i;

	for (i = 0; s[i] && o < sizeof(shown) - 5; i++) {
		unsigned char c = (unsigned char)s[i];

		if (c >= 0x20 && c < 0x7f && c != '\\') {
			shown[o++] = (char)c;
		} else {
			o += (size_t)snprintf(shown + o, sizeof(shown) - o,
					      "\\\\x%02x", c);
		}
	}
	shown[o] = 0;
	return shown;
}

/* The caller, shaped like acpi_ds_create_operand: uninitialized local, status
 * check, then an unconditional ACPI_FREE. noinline so the callee is a real
 * call with a real out-parameter, as in the kernel. */
__attribute__((noinline))
static int caller_get_and_free(acpi_object_type data_type, u8 *aml,
			       const char *want, u32 want_len, const char *tag)
{
	char *name_string;	/* deliberately uninitialized, as in ACPICA */
	u32 name_length;
	acpi_status status;

	status = acpi_ex_get_name_string(data_type, aml, &name_string,
					 &name_length);

	if (ACPI_FAILURE(status)) {
		/* Failure paths must not hand back a pointer at all. */
		printf("%-22s status=%08x (failure, as expected)\n", tag,
		       status);
		return 0;
	}

	if (!name_string) {
		printf("%-22s FAIL: AE_OK but out_name_string is NULL\n", tag);
		return 1;
	}
	if (!arena_owns(name_string)) {
		printf("%-22s FAIL: AE_OK but out_name_string is not an arena "
		       "pointer (store lost)\n", tag);
		return 1;
	}
	if (name_length != want_len) {
		printf("%-22s FAIL: length %u, want %u\n", tag, name_length,
		       want_len);
		return 1;
	}
	if (strcmp(name_string, want) != 0) {
		printf("%-22s FAIL: name %s, want %s\n", tag,
		       show(name_string), show(want));
		return 1;
	}

	ACPI_FREE(name_string);
	printf("%-22s ok status=0 len=%u name=%s\n", tag, name_length,
	       show(want));
	return 0;
}

/* Multi-segment path, the one the kernel took for \_SB.PCI0.PRES._INI */
static u8 aml_root_multi3[] = {
	AML_ROOT_PREFIX, AML_MULTI_NAME_PREFIX, 3,
	'_', 'S', 'B', '_',
	'P', 'C', 'I', '0',
	'P', 'R', 'E', 'S',
	0
};
static u8 aml_root_multi4[] = {
	AML_ROOT_PREFIX, AML_MULTI_NAME_PREFIX, 4,
	'_', 'S', 'B', '_',
	'P', 'C', 'I', '0',
	'P', 'R', 'E', 'S',
	'_', 'I', 'N', 'I',
	0
};
static u8 aml_root_dual[] = {
	AML_ROOT_PREFIX, AML_DUAL_NAME_PREFIX,
	'_', 'S', 'B', '_',
	'P', 'C', 'I', '0',
	0
};
static u8 aml_parent3_multi2[] = {
	AML_PARENT_PREFIX, AML_PARENT_PREFIX, AML_PARENT_PREFIX,
	AML_MULTI_NAME_PREFIX, 2,
	'A', 'B', 'C', 'D',
	'E', 'F', 'G', 'H',
	0
};
static u8 aml_dual_noprefix[] = {
	AML_DUAL_NAME_PREFIX,
	'A', 'B', 'C', 'D',
	'E', 'F', 'G', 'H',
	0
};
static u8 aml_plain_segment[] = { 'D', 'E', 'V', 'C', 0 };
static u8 aml_null_name[] = { 0 };
static u8 aml_root_null[] = { AML_ROOT_PREFIX, 0 };
static u8 aml_field_name[] = { 'F', 'L', 'D', '0', 0 };
/* Failure paths: these must return non-zero and free nothing to the caller. */
static u8 aml_short_segment[] = { 'A', 'B', 0 };
static u8 aml_leading_digit[] = { '9', 'A', 'B', 'C', 0 };
static u8 aml_root_leading_digit[] = { AML_ROOT_PREFIX, '9', 'A', 'B', 'C', 0 };
static u8 aml_parent_no_segment[] = { AML_PARENT_PREFIX, AML_PARENT_PREFIX, 0 };
static u8 aml_multi_bad_tail[] = {
	AML_MULTI_NAME_PREFIX, 2,
	'A', 'B', 'C', 'D',
	'X', 'Y', 0			/* second segment too short */
};

int main(void)
{
	int fails = 0;

	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_root_multi3,
				     "\\" "/\x03" "_SB_PCI0PRES", 15, "root+multi3");
	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_root_multi4,
				     "\\" "/\x04" "_SB_PCI0PRES_INI", 19, "root+multi4");
	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_root_dual,
				     "\\" "\x2e" "_SB_PCI0", 10, "root+dual");
	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_parent3_multi2,
				     "^^^" "\x2e" "ABCDEFGH", 13, "parent3+multi2");
	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_dual_noprefix,
				     "\x2e" "ABCDEFGH", 9, "dual-noprefix");
	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_plain_segment,
				     "DEVC", 4, "plain-segment");
	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_null_name,
				     "", 1, "null-name");
	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_root_null,
				     "\\", 2, "root-null");
	/* Field types take the first branch and disallow prefixes. */
	fails += caller_get_and_free(ACPI_TYPE_LOCAL_REGION_FIELD, aml_field_name,
				     "FLD0", 4, "field-region");
	fails += caller_get_and_free(ACPI_TYPE_LOCAL_BANK_FIELD, aml_field_name,
				     "FLD0", 4, "field-bank");
	fails += caller_get_and_free(ACPI_TYPE_LOCAL_INDEX_FIELD, aml_field_name,
				     "FLD0", 4, "field-index");
	/* Failure paths. */
	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_short_segment,
				     "", 0, "short-segment");
	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_leading_digit,
				     "", 0, "leading-digit");
	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_root_leading_digit,
				     "", 0, "root+leading-digit");
	/* ^^ followed by the null name: `case 0` still allocates, so this is a
	 * success returning "^^" — not a failure, despite the trailing prefix. */
	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_parent_no_segment,
				     "^^", 3, "parent+null-name");
	fails += caller_get_and_free(ACPI_TYPE_ANY, aml_multi_bad_tail,
				     "", 0, "multi-bad-tail");

	/* Every successful case freed exactly what it allocated. */
	if (arena_live != 0) {
		printf("FAIL: %lu arena block(s) leaked\n", arena_live);
		fails++;
	}

	printf("trace=%lu exit=%lu alloc=%lu free=%lu arena_used=%u\n",
	       trace_calls, exit_calls, alloc_calls, free_calls,
	       (unsigned)arena_used);
	printf("%s (%d failure%s)\n", fails ? "FAILED" : "ALL-OK", fails,
	       fails == 1 ? "" : "s");
	return fails ? 1 : 0;
}
