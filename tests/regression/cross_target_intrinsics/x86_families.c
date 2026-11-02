/* AUD-SILENT-ZERO-1 family sweep: one representative of every x86-only
 * intrinsic family the frontend accepts, so the gate can assert that the
 * "not available on <target>" diagnostic fires family by family rather than
 * only for the AES-NI case that exposed the silent-zero bug.
 *
 * Deliberately in this subdirectory: `tests/regression/*.c` is globbed
 * non-recursively and compiled as a stand-alone program for the HOST target,
 * and half of these are 256-bit shapes.
 *
 * Kept out of the corpus for a second reason: every function here is invalid
 * on a non-x86 target, which is exactly what the gate wants and exactly what
 * the corpus (one flag set, host target) cannot express.
 */
typedef long long v2di __attribute__((vector_size(16)));
typedef int v8si __attribute__((vector_size(32)));
typedef long long v4di __attribute__((vector_size(32)));
typedef float v8sf __attribute__((vector_size(32)));
typedef double v4df __attribute__((vector_size(32)));

/* AES-NI, 128-bit: the family that first exposed the silent zero-fill. */
v2di x86_fam_aesenc(v2di a, v2di b) { return __builtin_ia32_aesenc128(a, b); }

/* Carry-less multiply, 128-bit. */
v2di x86_fam_pclmul(v2di a, v2di b) { return __builtin_ia32_pclmulqdq128(a, b, 0); }

/* SSE4.2 CRC32. */
unsigned x86_fam_crc32(unsigned a, unsigned char b) { return __builtin_ia32_crc32qi(a, b); }

/* VAES, 256-bit. */
v4di x86_fam_vaes256(v4di a, v4di b) { return _mm256_aesenc_epi128(a, b); }

/* VPCLMULQDQ, 256-bit. */
v4di x86_fam_vpclmul256(v4di a, v4di b) { return _mm256_clmulepi64_epi128(a, b, 0); }

/* AVX float. */
v8sf x86_fam_addps256(v8sf a, v8sf b) { return _mm256_add_ps(a, b); }

/* AVX double. */
v4df x86_fam_mulpd256(v4df a, v4df b) { return _mm256_mul_pd(a, b); }

/* AVX2 integer broadcast. */
v4di x86_fam_broadcast(v2di a) { return _mm256_broadcastsi128_si256(a); }

/* No-operand VEX housekeeping: the op the AArch64 catch-all used to swallow. */
void x86_fam_zeroupper(void) { _mm256_zeroupper(); }
