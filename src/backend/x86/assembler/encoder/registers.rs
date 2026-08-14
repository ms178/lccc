use super::super::parser::*;

/// Register encoding (3-bit register number in ModR/M and SIB).
pub(crate) fn reg_num(name: &str) -> Option<u8> {
    match name {
        "al" | "ax" | "eax" | "rax" | "xmm0" | "st" | "st(0)" | "mm0" | "es" | "ymm0" | "zmm0" | "k0" => Some(0),
        "cl" | "cx" | "ecx" | "rcx" | "xmm1" | "st(1)" | "mm1" | "cs" | "ymm1" | "zmm1" | "k1" => Some(1),
        "dl" | "dx" | "edx" | "rdx" | "xmm2" | "st(2)" | "mm2" | "ss" | "ymm2" | "zmm2" | "k2" => Some(2),
        "bl" | "bx" | "ebx" | "rbx" | "xmm3" | "st(3)" | "mm3" | "ds" | "ymm3" | "zmm3" | "k3" => Some(3),
        "ah" | "spl" | "sp" | "esp" | "rsp" | "xmm4" | "st(4)" | "mm4" | "fs" | "ymm4" | "zmm4" | "k4" => Some(4),
        "ch" | "bpl" | "bp" | "ebp" | "rbp" | "xmm5" | "st(5)" | "mm5" | "gs" | "ymm5" | "zmm5" | "k5" => Some(5),
        "dh" | "sil" | "si" | "esi" | "rsi" | "xmm6" | "st(6)" | "mm6" | "ymm6" | "zmm6" | "k6" => Some(6),
        "bh" | "dil" | "di" | "edi" | "rdi" | "xmm7" | "st(7)" | "mm7" | "ymm7" | "zmm7" | "k7" => Some(7),
        "r8b" | "r8w" | "r8d" | "r8" | "xmm8" | "ymm8" | "zmm8" => Some(0),
        "r9b" | "r9w" | "r9d" | "r9" | "xmm9" | "ymm9" | "zmm9" => Some(1),
        "r10b" | "r10w" | "r10d" | "r10" | "xmm10" | "ymm10" | "zmm10" => Some(2),
        "r11b" | "r11w" | "r11d" | "r11" | "xmm11" | "ymm11" | "zmm11" => Some(3),
        "r12b" | "r12w" | "r12d" | "r12" | "xmm12" | "ymm12" | "zmm12" => Some(4),
        "r13b" | "r13w" | "r13d" | "r13" | "xmm13" | "ymm13" | "zmm13" => Some(5),
        "r14b" | "r14w" | "r14d" | "r14" | "xmm14" | "ymm14" | "zmm14" => Some(6),
        "r15b" | "r15w" | "r15d" | "r15" | "xmm15" | "ymm15" | "zmm15" => Some(7),
        _ => None,
    }
}

/// Is this an MMX register?
pub(crate) fn is_mmx(name: &str) -> bool {
    name.starts_with("mm") && !name.starts_with("mmx")
        && name.len() <= 3
        && name.as_bytes().get(2).is_some_and(|c| c.is_ascii_digit())
}

/// Is this a segment register?
pub(crate) fn is_segment_reg(name: &str) -> bool {
    matches!(name, "es" | "cs" | "ss" | "ds" | "fs" | "gs")
}

pub(crate) fn is_control_reg(name: &str) -> bool {
    matches!(name, "cr0" | "cr2" | "cr3" | "cr4" | "cr8")
}

pub(crate) fn control_reg_num(name: &str) -> Option<u8> {
    match name {
        "cr0" => Some(0),
        "cr2" => Some(2),
        "cr3" => Some(3),
        "cr4" => Some(4),
        "cr8" => Some(8),
        _ => None,
    }
}

pub(crate) fn is_debug_reg(name: &str) -> bool {
    matches!(name, "db0" | "db1" | "db2" | "db3" | "db4" | "db5" | "db6" | "db7"
                  | "dr0" | "dr1" | "dr2" | "dr3" | "dr4" | "dr5" | "dr6" | "dr7")
}

pub(crate) fn debug_reg_num(name: &str) -> Option<u8> {
    match name {
        "db0" | "dr0" => Some(0),
        "db1" | "dr1" => Some(1),
        "db2" | "dr2" => Some(2),
        "db3" | "dr3" => Some(3),
        "db4" | "dr4" => Some(4),
        "db5" | "dr5" => Some(5),
        "db6" | "dr6" => Some(6),
        "db7" | "dr7" => Some(7),
        _ => None,
    }
}

/// Is this a YMM register?
pub(crate) fn is_ymm(name: &str) -> bool {
    name.starts_with("ymm")
}

/// Does this register need the REX.B/R/X extension bit?
pub(crate) fn needs_rex_ext(name: &str) -> bool {
    name.starts_with("r8") || name.starts_with("r9") || name.starts_with("r10")
        || name.starts_with("r11") || name.starts_with("r12") || name.starts_with("r13")
        || name.starts_with("r14") || name.starts_with("r15")
        || name.starts_with("xmm8") || name.starts_with("xmm9")
        || name.starts_with("xmm10") || name.starts_with("xmm11")
        || name.starts_with("xmm12") || name.starts_with("xmm13")
        || name.starts_with("xmm14") || name.starts_with("xmm15")
        || name.starts_with("ymm8") || name.starts_with("ymm9")
        || name.starts_with("ymm10") || name.starts_with("ymm11")
        || name.starts_with("ymm12") || name.starts_with("ymm13")
        || name.starts_with("ymm14") || name.starts_with("ymm15")
        || name.starts_with("zmm8") || name.starts_with("zmm9")
        || name.starts_with("zmm10") || name.starts_with("zmm11")
        || name.starts_with("zmm12") || name.starts_with("zmm13")
        || name.starts_with("zmm14") || name.starts_with("zmm15")
}

/// Is this a ZMM (512-bit AVX-512) register?
pub(crate) fn is_zmm(name: &str) -> bool {
    name.starts_with("zmm")
}

/// Is this an AVX-512 mask (opmask) register?
pub(crate) fn is_kreg(name: &str) -> bool {
    name.len() == 2 && name.starts_with('k')
        && name.as_bytes().get(1).is_some_and(|c| c.is_ascii_digit())
}

/// Does a ZMM register need the EVEX R' extension bit (zmm16-31)?
pub(crate) fn needs_evex_rprime(name: &str) -> bool {
    if let Some(num) = name.strip_prefix("zmm") {
        if let Ok(n) = num.parse::<u8>() {
            return n >= 16;
        }
    }
    false
}

/// Does this register need the VEX.B extension bit? Same as REX ext but for VEX-encoded instructions.
pub(crate) fn needs_vex_ext(name: &str) -> bool {
    needs_rex_ext(name)
}

/// Is this a 64-bit GP register?
pub(crate) fn is_reg64(name: &str) -> bool {
    matches!(name, "rax" | "rcx" | "rdx" | "rbx" | "rsp" | "rbp" | "rsi" | "rdi"
        | "r8" | "r9" | "r10" | "r11" | "r12" | "r13" | "r14" | "r15")
}

/// Is this a 32-bit GP register?
pub(crate) fn is_reg32(name: &str) -> bool {
    matches!(name, "eax" | "ecx" | "edx" | "ebx" | "esp" | "ebp" | "esi" | "edi"
        | "r8d" | "r9d" | "r10d" | "r11d" | "r12d" | "r13d" | "r14d" | "r15d")
}

/// Is this a 16-bit GP register?
pub(crate) fn is_reg16(name: &str) -> bool {
    matches!(name, "ax" | "cx" | "dx" | "bx" | "sp" | "bp" | "si" | "di"
        | "r8w" | "r9w" | "r10w" | "r11w" | "r12w" | "r13w" | "r14w" | "r15w")
}

/// Is this an 8-bit GP register?
pub(crate) fn is_reg8(name: &str) -> bool {
    matches!(name, "al" | "cl" | "dl" | "bl" | "ah" | "ch" | "dh" | "bh"
        | "spl" | "bpl" | "sil" | "dil"
        | "r8b" | "r9b" | "r10b" | "r11b" | "r12b" | "r13b" | "r14b" | "r15b")
}

/// Does this 8-bit register require REX prefix for access (spl, bpl, sil, dil)?
pub(crate) fn is_rex_required_8bit(name: &str) -> bool {
    matches!(name, "spl" | "bpl" | "sil" | "dil")
}

/// Is this an XMM register?
pub(crate) fn is_xmm(name: &str) -> bool {
    name.starts_with("xmm")
}

/// Is this an XMM or YMM register?
pub(crate) fn is_xmm_or_ymm(name: &str) -> bool {
    name.starts_with("xmm") || name.starts_with("ymm")
}

/// Get the operand-size suffix character for a register name.
/// Returns 'q' for 64-bit, 'l' for 32-bit, 'w' for 16-bit, 'b' for 8-bit.
pub(crate) fn register_size_suffix(name: &str) -> Option<char> {
    if is_reg64(name) { return Some('q'); }
    if is_reg32(name) { return Some('l'); }
    if is_reg16(name) { return Some('w'); }
    if is_reg8(name) { return Some('b'); }
    None
}

/// Set of base mnemonics that accept AT&T size suffixes (b/w/l/q).
/// Only these mnemonics will have suffixes inferred from operand types.
pub(crate) const SUFFIXABLE_MNEMONICS: &[&str] = &[
    "mov", "add", "sub", "and", "or", "xor", "cmp", "test",
    "push", "pop", "lea",
    "shl", "shr", "sar", "rol", "ror",
    "inc", "dec", "neg", "not",
    "imul", "mul", "div", "idiv",
    "adc", "sbb",
    "xchg", "cmpxchg", "xadd", "bswap",
    "bsf", "bsr",
];

/// Infer the AT&T size suffix for an unsuffixed mnemonic from its operands.
///
/// Hand-written assembly (e.g., musl's .s files) often omits the size suffix
/// when it can be inferred from register operands. For example:
///   push %rax  -> pushq %rax
///   mov %edi,%ecx -> movl %edi,%ecx
///   and $0x3f,%ecx -> andl $0x3f,%ecx
///
/// Only mnemonics in the SUFFIXABLE_MNEMONICS whitelist are candidates for
/// suffix inference. All others are returned as-is.
pub(crate) fn infer_suffix(mnemonic: &str, ops: &[Operand]) -> String {
    // Check if this is a known suffixable mnemonic
    if !SUFFIXABLE_MNEMONICS.contains(&mnemonic) {
        return mnemonic.to_string();
    }

    // For shift/rotate instructions, infer size from the *destination* (last) operand,
    // not %cl (the first operand). E.g., "shl %cl, %edx" should become "shll", not "shlb".
    let is_shift = matches!(mnemonic, "shl" | "shr" | "sar" | "rol" | "ror");
    if is_shift && ops.len() == 2 {
        if let Operand::Register(r) = &ops[1] {
            if let Some(suffix) = register_size_suffix(&r.name) {
                return format!("{}{}", mnemonic, suffix);
            }
        }
    }

    // Find the first register operand to determine size
    for op in ops {
        if let Operand::Register(r) = op {
            if let Some(suffix) = register_size_suffix(&r.name) {
                return format!("{}{}", mnemonic, suffix);
            }
        }
    }

    // No register operand found - return as-is
    mnemonic.to_string()
}

/// Get operand size from mnemonic suffix.
pub(crate) fn mnemonic_size_suffix(mnemonic: &str) -> Option<u8> {
    // Handle mnemonics that don't follow the simple suffix pattern
    match mnemonic {
        "cltq" | "cqto" | "cltd" | "cdq" | "cqo" | "cbtw" | "cbw" | "cwtl"
        | "cwde" | "cdqe" | "cwtd" | "cwd" | "ret" | "nop" | "ud2"
        // `rdrand`/`rdseed` end in `d`, and `movbe`/`lddqu` in `e`/`u`; none of
        // those letters is an AT&T size suffix.  Without an entry here the
        // width check below compares a bogus suffix against the real register.
        | "rdrand" | "rdseed" | "movbe" | "lddqu" | "vlddqu" | "bswap"
        | "endbr64" | "pause" | "mfence" | "lfence" | "sfence" | "clflush"
        | "syscall" | "sysenter" | "cpuid" | "rdtsc" | "rdtscp" | "rdpmc"
        | "clc" | "stc" | "cli" | "sti" | "cld" | "std" | "sahf" | "lahf" | "fninit" | "finit" | "fwait" | "wait" | "fnstcw" | "fstcw"
        | "fld1" | "fldl2e" | "fldlg2" | "fldln2" | "fldz" | "fldpi" | "fldl2t"
        | "fabs" | "fsqrt" | "frndint" | "f2xm1" | "fscale" | "fpatan" | "fprem" | "fprem1"
        | "fyl2x" | "fyl2xp1" | "fptan" | "fsin" | "fcos" | "fsincos" | "fxtract" | "fnclex" | "fclex" | "fxch"
        | "fadd" | "fmul" | "fsub" | "fdiv" | "fcom" | "fcomp" | "fcompp" | "fucompp"
        | "fnstenv" | "fstenv" | "fldenv" | "fnsave" | "fsave" | "frstor" | "fnstsw" | "fstsw"
        | "fnop" | "fincstp" | "fdecstp" | "ffree"
        | "ldmxcsr" | "stmxcsr" | "wbinvd" | "invd" | "rdsspq" | "rdsspd"
        | "lmsw" | "smsw"
        | "pushf" | "pushfq" | "pushfl" | "popf" | "popfq" | "popfl" | "int3"
        | "movsq" | "stosq" | "movsw" | "stosw" | "lodsb" | "lodsw" | "lodsd" | "lodsq"
        | "scasb" | "scasw" | "scasd" | "scasq" | "cmpsb" | "cmpsw" | "cmpsd" | "cmpsq"
        | "insb" | "insw" | "insd" | "insl" | "outsb" | "outsw" | "outsd" | "outsl"
        | "inb" | "inw" | "inl" | "outb" | "outw" | "outl" => return None,
        _ => {}
    }

    let last = mnemonic.as_bytes().last()?;
    match last {
        b'b' => Some(1),
        b'w' => Some(2),
        b'l' | b'd' => Some(4),
        b'q' => Some(8),
        _ => None,
    }
}

/// Register-class identity used for operand-consistency checking.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RegClass {
    Gp(u8), // width in bytes: 1, 2, 4, 8
    Xmm, Ymm, Zmm, Mask, Mmx, X87, Seg, Ctrl, Dbg, Other,
}

pub(crate) fn reg_class(name: &str) -> RegClass {
    if is_reg64(name) { RegClass::Gp(8) }
    else if is_reg32(name) { RegClass::Gp(4) }
    else if is_reg16(name) { RegClass::Gp(2) }
    else if is_reg8(name) { RegClass::Gp(1) }
    else if is_xmm(name) { RegClass::Xmm }
    else if is_ymm(name) { RegClass::Ymm }
    else if is_zmm(name) { RegClass::Zmm }
    else if is_kreg(name) { RegClass::Mask }
    else if is_mmx(name) { RegClass::Mmx }
    else if name == "st" || name.starts_with("st(") { RegClass::X87 }
    else if is_segment_reg(name) { RegClass::Seg }
    else if is_control_reg(name) { RegClass::Ctrl }
    else if is_debug_reg(name) { RegClass::Dbg }
    else { RegClass::Other }
}

/// Mnemonics whose register operands are INTENTIONALLY of different widths or
/// classes, and therefore exempt from the uniform-width check.
fn is_mixed_width_mnemonic(m: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "movs", "movz", "movbe", "cvt", "vcvt", "pmov", "vpmov", "pinsr", "pextr",
        "vpinsr", "vpextr", "extractps", "vextractps", "insertps", "vinsertps",
        "vextract", "vinsert", "vbroadcast", "vpbroadcast", "broadcast",
        "kmov", "movmsk", "vmovmsk",
        "crc32", "bt", "shld", "shrd",
        "vperm", "vpsll", "vpsrl", "vpsra", "psll", "psrl", "psra",
        "vgather", "vpgather", "vscatter", "vpscatter", "in", "out", "vzero",
        "enter", "lar", "lsl", "lgdt", "lidt", "sgdt", "sidt", "lldt", "sldt",
        "ltr", "str", "lmsw", "smsw", "verr", "verw", "vmread", "vmwrite",
        "vcmp", "cmpp", "cmps", "vdpp", "dpp", "vround", "round", "vblend",
        "blend", "vpblend", "pblend", "vpalignr", "palignr", "vshuf", "shuf",
        "pshuf", "vpshuf", "vptest", "ptest", "vtest", "aeskeygenassist",
        "pclmul", "vpclmul", "vpdp", "vpmadd", "pmadd", "psadbw", "vpsadbw",
        "mpsadbw", "vmpsadbw", "vfixup", "vrange", "vreduce", "vgetmant",
        "vrndscale", "vscalef", "vfpclass", "vpternlog", "vpcmp", "vpshld",
        "vpshrd", "vpconflict", "vplzcnt", "vpopcnt", "vpcompress", "vpexpand",
        "vcompress", "vexpand", "sh", "sal", "sar", "rol", "ror", "rcl", "rcr",
    ];
    PREFIXES.iter().any(|p| m.starts_with(p))
}

/// Validate that an instruction's operands are mutually consistent.
///
/// This catches the class of malformed input that would otherwise be encoded
/// as a *different, valid* instruction. It is deliberately conservative: it
/// only rejects combinations that are unambiguously illegal, so it can never
/// turn working assembly into an error.
pub(crate) fn validate_operands(mnemonic: &str, ops: &[Operand]) -> Result<(), String> {
    // `movd`/`movq` cross register FILES (`movq %xmm0,%rax`), so they are
    // exempt from the uniform-width rule ONLY when a vector register is
    // actually involved. As a plain GP move, `movq %rax,%eax` is just as
    // malformed as `mov %rax,%eax` and must be rejected.
    let vector_movdq = matches!(mnemonic, "movd" | "movq" | "vmovd" | "vmovq")
        && ops.iter().any(|op| matches!(op, Operand::Register(r)
            if matches!(reg_class(&r.name),
                        RegClass::Xmm | RegClass::Ymm | RegClass::Zmm | RegClass::Mmx)));
    if vector_movdq {
        // Still enforce that no ymm/zmm operand appears: movq is 64-bit only.
        for op in ops {
            if let Operand::Register(r) = op {
                if matches!(reg_class(&r.name), RegClass::Ymm | RegClass::Zmm) {
                    return Err(format!(
                        "`{}` operand must be xmm or GPR: %{}", mnemonic, r.name));
                }
            }
        }
        return Ok(());
    }

    // 1. SIB scale must be 1/2/4/8; %rsp can never be an index.
    for op in ops {
        if let Operand::Memory(m) = op {
            if let Some(scale) = m.scale {
                if !matches!(scale, 1 | 2 | 4 | 8) {
                    return Err(format!(
                        "invalid address scale {} (must be 1, 2, 4 or 8)", scale));
                }
            }
            if let Some(idx) = &m.index {
                let n = idx.name.trim_start_matches('%');
                if n == "rsp" || n == "esp" || n == "sp" {
                    return Err(format!("%{} cannot be used as an index register", n));
                }
            }
        }
    }

    // 2a. Extension source must be narrower than the destination and match
    //     the mnemonic's source suffix.
    if ops.len() == 2 && (mnemonic.starts_with("movs") || mnemonic.starts_with("movz"))
        && mnemonic.len() > 4
    {
        let src_w = match &mnemonic[4..5] {
            "b" => Some(1u8), "w" => Some(2), "l" => Some(4), _ => None,
        };
        if let (Some(w), Operand::Register(a)) = (src_w, &ops[0]) {
            if let RegClass::Gp(aw) = reg_class(&a.name) {
                if aw != w {
                    return Err(format!(
                        "`{}` source must be a {}-bit register, got %{}",
                        mnemonic, w * 8, a.name));
                }
            }
        }
        if let (Operand::Register(a), Operand::Register(b)) = (&ops[0], &ops[1]) {
            if let (RegClass::Gp(aw), RegClass::Gp(bw)) =
                (reg_class(&a.name), reg_class(&b.name))
            {
                if aw >= bw {
                    return Err(format!(
                        "`{}` destination must be wider than source: %{} -> %{}",
                        mnemonic, a.name, b.name));
                }
            }
        }
    }

    // 2b. `cmovCC` has no 8-bit form.
    if mnemonic.starts_with("cmov") {
        for op in ops {
            if let Operand::Register(r) = op {
                if reg_class(&r.name) == RegClass::Gp(1) {
                    return Err(format!("`{}` has no 8-bit form: %{}", mnemonic, r.name));
                }
            }
        }
    }

    // 2c. `movd`/`movq` never take a ymm/zmm operand.
    if matches!(mnemonic, "movd" | "movq" | "vmovd" | "vmovq") {
        for op in ops {
            if let Operand::Register(r) = op {
                if matches!(reg_class(&r.name), RegClass::Ymm | RegClass::Zmm) {
                    return Err(format!(
                        "`{}` operand must be xmm or GPR: %{}", mnemonic, r.name));
                }
            }
        }
    }

    // 3. Uniform-width rule for plain two-register data/ALU forms.
    if ops.len() == 2 && !is_mixed_width_mnemonic(mnemonic) {
        if let (Operand::Register(a), Operand::Register(b)) = (&ops[0], &ops[1]) {
            let (ca, cb) = (reg_class(&a.name), reg_class(&b.name));
            let comparable = matches!(
                (ca, cb),
                (RegClass::Gp(_), RegClass::Gp(_))
                    | (RegClass::Xmm, RegClass::Xmm)
                    | (RegClass::Ymm, RegClass::Ymm)
                    | (RegClass::Xmm, RegClass::Ymm)
                    | (RegClass::Ymm, RegClass::Xmm)
            );
            if comparable && ca != cb {
                return Err(format!(
                    "operand size mismatch for `{}`: %{} and %{}",
                    mnemonic, a.name, b.name));
            }
        }
    }

    // 4. A vector mnemonic's register operands must all be the same width.
    if mnemonic.starts_with('v') && !is_mixed_width_mnemonic(mnemonic) {
        let mut seen: Option<RegClass> = None;
        for op in ops {
            if let Operand::Register(r) = op {
                let c = reg_class(&r.name);
                if !matches!(c, RegClass::Xmm | RegClass::Ymm | RegClass::Zmm) {
                    continue;
                }
                match seen {
                    None => seen = Some(c),
                    Some(prev) if prev != c => {
                        return Err(format!(
                            "mixed vector register widths in `{}`", mnemonic));
                    }
                    _ => {}
                }
            }
        }
    }

    // 5. A suffixed mnemonic must agree with its register operands' width.
    //    `setCC`/`cmovCC`/`jCC` end in a CONDITION CODE, not a size suffix
    //    (`setl` is set-if-less, not "set long"), so they must be excluded.
    let is_cc_mnemonic = mnemonic.starts_with("set")
        || mnemonic.starts_with("cmov")
        || (mnemonic.starts_with('j') && mnemonic != "jmp" && mnemonic != "jmpq");
    if let Some(sz) = mnemonic_size_suffix(mnemonic) {
        if !is_cc_mnemonic && !is_mixed_width_mnemonic(mnemonic) && matches!(sz, 1 | 2 | 4 | 8) {
            for op in ops {
                if let Operand::Register(r) = op {
                    if let RegClass::Gp(w) = reg_class(&r.name) {
                        if w != sz {
                            return Err(format!(
                                "`{}` operand size does not match register %{}",
                                mnemonic, r.name));
                        }
                    }
                }
            }
        }
    }

    // 6. REX / legacy-high-byte conflict. %ah/%ch/%dh/%bh are encodable only
    //    WITHOUT REX, while %spl/%bpl/%sil/%dil and %r8b-%r15b REQUIRE it.
    //    An instruction naming both is unencodable: the REX byte one operand
    //    demands silently reinterprets the other, touching the WRONG REGISTER.
    {
        let mut has_high8 = false;
        let mut needs_rex = false;
        for op in ops {
            if let Operand::Register(r) = op {
                let n = r.name.trim_start_matches('%');
                if matches!(n, "ah" | "ch" | "dh" | "bh") {
                    has_high8 = true;
                } else if is_rex_required_8bit(n) || needs_rex_ext(n) {
                    needs_rex = true;
                }
            } else if let Operand::Memory(m) = op {
                if m.base.as_ref().is_some_and(|b| needs_rex_ext(&b.name))
                    || m.index.as_ref().is_some_and(|i| needs_rex_ext(&i.name))
                {
                    needs_rex = true;
                }
            }
        }
        if has_high8 && needs_rex {
            return Err(
                "cannot encode %ah/%ch/%dh/%bh together with a REX-requiring register"
                    .to_string());
        }
    }

    // 7. In 64-bit mode push/pop take 64-bit (or 16-bit) operands only.
    // `infer_suffix` may already have rewritten `push %eax` to `pushl`, so
    // match every spelling rather than only the canonical 64-bit one.
    if matches!(mnemonic,
                "push" | "pushq" | "pushl" | "pushw" | "pushb"
                    | "pop" | "popq" | "popl" | "popw" | "popb") {
        if let Some(Operand::Register(r)) = ops.first() {
            if let RegClass::Gp(w) = reg_class(&r.name) {
                if w == 4 || w == 1 {
                    return Err(format!(
                        "cannot {} a {}-bit register in 64-bit mode: %{}",
                        if mnemonic.starts_with("push") { "push" } else { "pop" },
                        w * 8, r.name));
                }
            }
        }
    }

    // 8. Zero-operand instructions must not be given operands.
    if matches!(
        mnemonic,
        "vzeroupper" | "vzeroall" | "cpuid" | "leave" | "leaveq"
            | "hlt" | "pause" | "syscall" | "sysret" | "ud2" | "cwtl"
            | "cltq" | "cqto" | "cltd" | "cwtd" | "endbr64" | "endbr32"
            | "cbtw" | "cbw" | "cwde" | "cdqe" | "cwd"
            | "lfence" | "sfence" | "mfence" | "rdtsc" | "rdtscp" | "cdq" | "cqo"
    ) && !ops.is_empty()
    {
        return Err(format!("`{}` takes no operands", mnemonic));
    }

    Ok(())
}

/// Truncate an immediate to the operand's width and reinterpret it as signed.
///
/// x86 immediates are *modular*: for a 16-bit operand `$65535`, `$-1` and
/// `$0xffff` denote the same value. An encoder that tests the raw source
/// integer against the imm8 range therefore misses the compact form whenever
/// the user wrote the unsigned spelling — GAS emits `66 83 c0 ff` for
/// `addw $65535,%ax` while a naive encoder emits the 4/5-byte `81` form.
pub(crate) fn canonical_imm(val: i64, size: u8) -> i64 {
    match size {
        1 => val as u8 as i8 as i64,
        2 => val as u16 as i16 as i64,
        4 => val as u32 as i32 as i64,
        _ => val,
    }
}

/// Whether `val`, taken modulo the operand width, fits the sign-extended
/// 8-bit immediate form (opcode `0x83` / `0x6b`).
pub(crate) fn fits_imm8(val: i64, size: u8) -> bool {
    (-128..=127).contains(&canonical_imm(val, size))
}

/// Infer register size in bytes from register name.
pub(crate) fn infer_reg_size(name: &str) -> u8 {
    if is_reg64(name) { 8 }
    else if is_reg32(name) { 4 }
    else if is_reg16(name) { 2 }
    else if is_reg8(name) { 1 }
    else if is_xmm(name) { 16 }
    else if is_ymm(name) { 32 }
    else { 8 } // mmx and other registers default to 8
}

/// Infer operand size from a pair of operands for suffix-less instructions.
pub(crate) fn infer_operand_size_from_pair(op1: &Operand, op2: &Operand) -> u8 {
    // Try to infer from register operands
    for op in [op1, op2] {
        if let Operand::Register(r) = op {
            if is_segment_reg(&r.name) { continue; }
            if is_reg64(&r.name) { return 8; }
            if is_reg32(&r.name) { return 4; }
            if is_reg16(&r.name) { return 2; }
            if is_reg8(&r.name) { return 1; }
        }
    }
    // Default to 64-bit
    8
}

/// Parse x87 register number: "st(0)" -> 0, "st" -> 0, "st(1)" -> 1, etc.
pub(crate) fn parse_st_num(name: &str) -> Result<u8, String> {
    if name == "st" || name == "st(0)" {
        return Ok(0);
    }
    if name.starts_with("st(") && name.ends_with(')') {
        let n: u8 = name[3..name.len()-1].parse()
            .map_err(|_| format!("bad st register: {}", name))?;
        if n > 7 {
            return Err(format!("st register out of range: {}", name));
        }
        return Ok(n);
    }
    Err(format!("not an st register: {}", name))
}

/// Map condition code suffix to encoding.
pub(crate) fn cc_from_mnemonic(cc_str: &str) -> Result<u8, String> {
    match cc_str {
        "o" => Ok(0),
        "no" => Ok(1),
        "b" | "c" | "nae" => Ok(2),
        "nb" | "nc" | "ae" => Ok(3),
        "e" | "z" => Ok(4),
        "ne" | "nz" => Ok(5),
        "be" | "na" => Ok(6),
        "nbe" | "a" => Ok(7),
        "s" => Ok(8),
        "ns" => Ok(9),
        "p" | "pe" => Ok(10),
        "np" | "po" => Ok(11),
        "l" | "nge" => Ok(12),
        "nl" | "ge" => Ok(13),
        "le" | "ng" => Ok(14),
        "nle" | "g" => Ok(15),
        _ => Err(format!("unknown condition code: {}", cc_str)),
    }
}

/// True for the FMA3 mnemonics that use the VEX 3-operand `0F38` encoding.
pub(crate) fn is_fma3_vex(m: &str) -> bool {
    fma3_opcode(m).is_some()
}

/// Decode an FMA3 mnemonic into its `0F38` opcode and VEX.W bit.
///
/// The FMA3 opcode space is a regular grid:
///
/// ```text
///                 132     213     231
///   vfmadd        0x98    0xA8    0xB8      packed; +1 -> scalar
///   vfmsub        0x9A    0xAA    0xBA
///   vfnmadd       0x9C    0xAC    0xBC
///   vfnmsub       0x9E    0xAE    0xBE
///   vfmaddsub     0x96    0xA6    0xB6      packed only
///   vfmsubadd     0x97    0xA7    0xB7      packed only
/// ```
///
/// W=1 selects the double-precision element type (`pd`/`sd`), W=0 selects
/// single (`ps`/`ss`).  The mandatory prefix is 66 for every form.
pub(crate) fn fma3_opcode(m: &str) -> Option<(u8, u8)> {
    let rest = m.strip_prefix("vfm").map(|r| (r, false))
        .or_else(|| m.strip_prefix("vfnm").map(|r| (r, true)))?;
    let (rest, negated) = rest;

    // Element type: the last two characters.
    let (rest, ty) = rest.split_at(rest.len().checked_sub(2)?);
    let (w, scalar) = match ty {
        "ps" => (0u8, false),
        "pd" => (1, false),
        "ss" => (0, true),
        "sd" => (1, true),
        _ => return None,
    };

    // Operand order: the three digits before the element type.
    let (op, order) = rest.split_at(rest.len().checked_sub(3)?);
    let order_step: u8 = match order {
        "132" => 0,
        "213" => 1,
        "231" => 2,
        _ => return None,
    };

    let base: u8 = match (op, negated) {
        ("add", false) => 0x98,
        ("sub", false) => 0x9A,
        ("add", true) => 0x9C,
        ("sub", true) => 0x9E,
        // addsub/subadd have no scalar form.
        ("addsub", false) if !scalar => 0x96,
        ("subadd", false) if !scalar => 0x97,
        _ => return None,
    };

    Some((base + 0x10 * order_step + u8::from(scalar), w))
}
