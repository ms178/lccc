/* One caller for each intrinsic of include/x86gprintrin.h that is a single
 * instruction, and for the seven ia32intrin.h ones added with it. Compiled
 * by lccc and by GCC 16.2, never run: check_x86_gpr_intrinsics.py compares
 * what each caller executes, instruction by instruction, with operands
 * reduced to their kind and width (a register's name is the allocator's
 * business; its width selects the instruction's form). */
#include <immintrin.h>

void t_mm_clflushopt(void *p) { _mm_clflushopt(p); }
void t_mm_clwb(void *p) { _mm_clwb(p); }
void t_cldemote(void *p) { _cldemote(p); }
void t_mm_clzero(void *p) { _mm_clzero(p); }
void t_wbinvd(void) { _wbinvd(); }
void t_wbnoinvd(void) { _wbnoinvd(); }
void t_directstoreu_u32(void *p, unsigned v) { _directstoreu_u32(p, v); }
void t_movdir64b(void *p, const void *q) { _movdir64b(p, q); }
int t_enqcmd(void *p, const void *q) { return _enqcmd(p, q); }
int t_enqcmds(void *p, const void *q) { return _enqcmds(p, q); }
unsigned t_rdpid_u32(void) { return _rdpid_u32(); }
void t_ptwrite32(unsigned v) { _ptwrite32(v); }
unsigned t_rdpkru_u32(void) { return _rdpkru_u32(); }
void t_wrpkru(unsigned k) { _wrpkru(k); }
void t_serialize(void) { _serialize(); }
void t_hreset(unsigned v) { _hreset(v); }
void t_umonitor(void *p) { _umonitor(p); }
unsigned char t_umwait(unsigned c, unsigned long long t) { return _umwait(c, t); }
unsigned char t_tpause(unsigned c, unsigned long long t) { return _tpause(c, t); }
void t_mm_monitorx(const void *p, unsigned e, unsigned h) { _mm_monitorx(p, e, h); }
void t_mm_mwaitx(unsigned e, unsigned h, unsigned c) { _mm_mwaitx(e, h, c); }
unsigned t_xbegin(void) { return _xbegin(); }
void t_xend(void) { _xend(); }
void t_xabort(void) { _xabort(0x5a); }
int t_xtest(void) { return _xtest(); }
void t_xsusldtrk(void) { _xsusldtrk(); }
void t_xresldtrk(void) { _xresldtrk(); }
void t_xsaveopt(void *p, long long m) { _xsaveopt(p, m); }
void t_xsavec(void *p, long long m) { _xsavec(p, m); }
void t_xsaves(void *p, long long m) { _xsaves(p, m); }
void t_xrstors(void *p, long long m) { _xrstors(p, m); }
void t_fxsave(void *p) { _fxsave(p); }
void t_fxrstor(void *p) { _fxrstor(p); }
void t_inc_ssp(unsigned n) { _inc_ssp(n); }
void t_saveprevssp(void) { _saveprevssp(); }
void t_rstorssp(void *p) { _rstorssp(p); }
void t_wrssd(unsigned v, void *p) { _wrssd(v, p); }
void t_wrussd(unsigned v, void *p) { _wrussd(v, p); }
void t_setssbsy(void) { _setssbsy(); }
void t_clrssbsy(void *p) { _clrssbsy(p); }
unsigned t_encls_u32(unsigned leaf, size_t *d) { return _encls_u32(leaf, d); }
unsigned t_encls_ecreate(size_t *d) { return _encls_u32(0x00, d); }
unsigned t_encls_eremove(size_t *d) { return _encls_u32(0x03, d); }
unsigned t_enclu_u32(unsigned leaf, size_t *d) { return _enclu_u32(leaf, d); }
unsigned t_enclv_u32(unsigned leaf, size_t *d) { return _enclv_u32(leaf, d); }
unsigned t_pconfig_u32(unsigned leaf, size_t *d) { return _pconfig_u32(leaf, d); }
unsigned long long t_rdpmc(int c) { return __rdpmc(c); }
unsigned t_crc32b(unsigned c, unsigned char v) { return __crc32b(c, v); }
unsigned t_crc32w(unsigned c, unsigned short v) { return __crc32w(c, v); }
unsigned t_crc32d(unsigned c, unsigned v) { return __crc32d(c, v); }
#ifdef __x86_64__
void t_directstoreu_u64(void *p, unsigned long long v) { _directstoreu_u64(p, v); }
unsigned t_readfsbase_u32(void) { return _readfsbase_u32(); }
unsigned long long t_readfsbase_u64(void) { return _readfsbase_u64(); }
unsigned t_readgsbase_u32(void) { return _readgsbase_u32(); }
unsigned long long t_readgsbase_u64(void) { return _readgsbase_u64(); }
void t_writefsbase_u32(unsigned v) { _writefsbase_u32(v); }
void t_writefsbase_u64(unsigned long long v) { _writefsbase_u64(v); }
void t_writegsbase_u32(unsigned v) { _writegsbase_u32(v); }
void t_writegsbase_u64(unsigned long long v) { _writegsbase_u64(v); }
void t_ptwrite64(unsigned long long v) { _ptwrite64(v); }
void t_clui(void) { _clui(); }
void t_stui(void) { _stui(); }
void t_senduipi(unsigned long long v) { _senduipi(v); }
unsigned char t_testui(void) { return _testui(); }
void t_xsaveopt64(void *p, long long m) { _xsaveopt64(p, m); }
void t_xsavec64(void *p, long long m) { _xsavec64(p, m); }
void t_xsaves64(void *p, long long m) { _xsaves64(p, m); }
void t_xrstors64(void *p, long long m) { _xrstors64(p, m); }
void t_fxsave64(void *p) { _fxsave64(p); }
void t_fxrstor64(void *p) { _fxrstor64(p); }
unsigned long long t_get_ssp(void) { return _get_ssp(); }
void t_wrssq(unsigned long long v, void *p) { _wrssq(v, p); }
void t_wrussq(unsigned long long v, void *p) { _wrussq(v, p); }
unsigned long long t_crc32q(unsigned long long c, unsigned long long v) { return __crc32q(c, v); }
unsigned long long t_readeflags(void) { return __readeflags(); }
void t_writeeflags(unsigned long long v) { __writeeflags(v); }
#else
unsigned t_get_ssp(void) { return _get_ssp(); }
unsigned t_readeflags(void) { return __readeflags(); }
void t_writeeflags(unsigned v) { __writeeflags(v); }
#endif
