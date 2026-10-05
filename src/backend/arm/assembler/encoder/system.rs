use super::sysreg_table::SYSREGS;
use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── System instructions ──────────────────────────────────────────────────

/// The register field of a SYS-form system instruction (`ic`, `tlbi`, `at`,
/// `dc`, ...), validated against the operation's own encoding.
///
/// Whether an operation takes a register is a property of the *instruction*,
/// not of the mnemonic: `dc cvau, x0` transfers an address, `ic iallu` takes no
/// register at all, and its encoding even spells Rt = 31.  Reading the spelling
/// first and the base word second turns that into a check instead of a guess --
/// `ic iallu, x0` and `tlbi vmalle1, x0` used to assemble with the extraneous
/// register silently OR-ed into a field the operation does not have, exactly the
/// fail-open shape this family was audited for.
///
/// `base` is the operation's full encoding with Rt = 31 already in it.
fn sys_class_rt(rt_spelling: Option<&str>, base: u32, mn: &str) -> Result<u32, String> {
    let takes_register = base & 0x1F != 0x1F;
    match (takes_register, rt_spelling) {
        (false, None) => Ok(31),
        (false, Some(name)) => Err(format!(
            "{mn}: this operation has no register operand, but `{name}` was given \
             (its encoding has Rt = 31)"
        )),
        (true, None) => Err(format!(
            "{mn}: this operation needs a 64-bit register operand (x0-x30)"
        )),
        (true, Some(name)) => gp_xt(name, mn),
    }
}


pub(crate) fn encode_dmb(operands: &[Operand]) -> Result<EncodeResult, String> {
    let option = match operands.first() {
        Some(Operand::Barrier(b)) | Some(Operand::Symbol(b)) => match b.to_lowercase().as_str() {
            "sy" => 0b1111u32,
            "st" => 0b1110,
            "ld" => 0b1101,
            "ish" => 0b1011,
            "ishst" => 0b1010,
            "ishld" => 0b1001,
            "nsh" => 0b0111,
            "nshst" => 0b0110,
            "nshld" => 0b0101,
            "osh" => 0b0011,
            "oshst" => 0b0010,
            "oshld" => 0b0001,
            _ => return Err(format!("unknown dmb option: {}", b)),
        },
        _ => 0b1111,
    };
    // DMB: 0xD50330BF | (CRm << 8)
    let word = 0xd50330bf | (option << 8);
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_dsb(operands: &[Operand]) -> Result<EncodeResult, String> {
    let option = match operands.first() {
        Some(Operand::Barrier(b)) | Some(Operand::Symbol(b)) => match b.to_lowercase().as_str() {
            "sy" => 0b1111u32,
            "st" => 0b1110,
            "ld" => 0b1101,
            "ish" => 0b1011,
            "ishst" => 0b1010,
            "ishld" => 0b1001,
            "nsh" => 0b0111,
            "nshst" => 0b0110,
            "nshld" => 0b0101,
            "osh" => 0b0011,
            "oshst" => 0b0010,
            "oshld" => 0b0001,
            _ => return Err(format!("unknown dsb option: {}", b)),
        },
        _ => 0b1111,
    };
    // DSB: 0xD503309F | (option << 8)
    let word = 0xd503309f | (option << 8);
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_mrs(operands: &[Operand]) -> Result<EncodeResult, String> {
    // MRS Xt, system_reg. Encoding 31 is XZR; a W or FP destination
    // previously encoded as X0 (`mrs w0, nzcv`, `mrs d0, fpcr`).
    let rt = match operands.first() {
        Some(Operand::Reg(name)) => gp_xt(name, "mrs")?,
        _ => return Err("mrs: expected Xt destination".to_string()),
    };
    let sysreg = match operands.get(1) {
        Some(Operand::Symbol(s)) => s.to_lowercase(),
        _ => return Err("mrs needs system register name".to_string()),
    };

    let encoding = sysreg_encoding_named(&sysreg)?;

    // MRS encoding: 0xd520_0000 has L=1 (bit 21) for read.
    // Bits [20:19] = op0, supplied entirely by the sysreg encoding field.
    let word = 0xd5200000 | (encoding << 5) | rt;
    Ok(EncodeResult::Word(word))
}

/// Compute sysreg encoding from (op0, op1, CRn, CRm, op2) fields.
fn sysreg_name_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    for i in 0..ab.len().min(bb.len()) {
        let (x, y) = (ab[i].to_ascii_lowercase(), bb[i].to_ascii_lowercase());
        if x != y {
            return x.cmp(&y);
        }
    }
    ab.len().cmp(&bb.len())
}


pub(crate) fn sysreg_encoding_named(name: &str) -> Result<u32, String> {
    let table = super::sysreg_table::SYSREGS;
    if let Ok(i) = table.binary_search_by(|&(n, _)| sysreg_name_cmp(n, name)) {
        return Ok(table[i].1);
    }
    parse_generic_sysreg(name)
}


pub(crate) fn sysreg_encoding(op0: u32, op1: u32, crn: u32, crm: u32, op2: u32) -> u32 {
    ((op0 & 3) << 14) | ((op1 & 7) << 11) | ((crn & 0xF) << 7) | ((crm & 0xF) << 3) | (op2 & 7)
}

fn msr_pstate_field(
    operands: &[Operand],
    idx: usize,
    op1: u32,
    op2: u32,
    crm_base: u32,
    name: &str,
) -> Result<Option<EncodeResult>, String> {
    let imm = match get_imm(operands, idx) {
        Ok(imm) => imm,
        Err(_) => return Ok(None),
    };
    let max = if op2 >= 0b110 { 0xF } else { 1 };
    if !(0..=max).contains(&imm) {
        return Err(format!(
            "msr {name}: immediate {imm} is outside the {}-bit field (allowed 0..={max})",
            if max == 1 { 1 } else { 4 }
        ));
    }
    let word = 0xd5000000
        | (op1 << 16)
        | (0b0100 << 12)
        | ((crm_base | imm as u32) << 8)
        | (op2 << 5)
        | 0x1F;
    Ok(Some(EncodeResult::Word(word)))
}

fn msr_pstate_imm_only(
    operands: &[Operand],
    idx: usize,
    op1: u32,
    op2: u32,
    crm_base: u32,
    name: &str,
) -> Result<EncodeResult, String> {
    match msr_pstate_field(operands, idx, op1, op2, crm_base, name)? {
        Some(word) => Ok(word),
        None => Err(format!(
            "msr {name}: needs an immediate operand ({name} is write-only and \
             has no register spelling)"
        )),
    }
}

pub(crate) fn parse_generic_sysreg(name: &str) -> Result<u32, String> {
    // Format: s<op0>_<op1>_c<CRn>_c<CRm>_<op2>.  GNU as reads the prefix
    // letters case-insensitively (`S3_0_C1_C0_1` assembles to the same word as
    // the lower-case spelling, and so do the mixed forms), so the prefixes are
    // matched that way here too -- a spelling the assembler accepts must not
    // fail on the case of its separator letters.
    let parts: Vec<&str> = name.split('_').collect();
    let bad = || format!("unsupported system register: {}", name);
    let prefixed = |text: &str, prefix: char| {
        text.as_bytes()
            .first()
            .is_some_and(|b| (*b as char).eq_ignore_ascii_case(&prefix))
    };
    if parts.len() != 5
        || !prefixed(parts[0], 's')
        || !prefixed(parts[2], 'c')
        || !prefixed(parts[3], 'c')
    {
        return Err(bad());
    }
    let field = |text: &str, strip: usize| -> Result<u32, String> {
        text.get(strip..)
            .and_then(|d| d.parse::<u32>().ok())
            .ok_or_else(bad)
    };
    let (op0, op1) = (field(parts[0], 1)?, field(parts[1], 0)?);
    let (crn, crm) = (field(parts[2], 1)?, field(parts[3], 1)?);
    let op2 = field(parts[4], 0)?;
    // Widths: op0 is 2 bits, op1 and op2 are 3, CRn and CRm are 4.
    if op0 > 3 || op1 > 7 || crn > 15 || crm > 15 || op2 > 7 {
        return Err(bad());
    }
    Ok(sysreg_encoding(op0, op1, crn, crm, op2))
}


pub(crate) fn encode_msr(operands: &[Operand]) -> Result<EncodeResult, String> {
    let sysreg = match operands.first() {
        Some(Operand::Symbol(s)) => s.to_lowercase(),
        _ => return Err("msr needs system register name".to_string()),
    };

    // MSR (immediate): msr <pstatefield>, #imm
    // Encoding: 1101_0101_0000_0 op1[18:16] 0100 CRm[11:8] op2[7:5] 11111[4:0]
    // with the immediate in CRm and the field named by (op1, CRm, op2).
    //
    // Every field here except DAIFSet/DAIFClr also has a *register* form --
    // `msr pan,x0`, `msr spsel,x0` -- because the field IS a 16-bit
    // system-register number (`mrs x0,pan` reads it back).  An operand that is
    // not an immediate therefore falls THROUGH this match to the shared
    // system-register write below; returning an error here made `msr pan,x0`
    // "expected immediate at operand 1" while GNU as assembles it.  DAIFSet
    // and DAIFClr are the exception: they exist only as `#imm`, so they do
    // fail here, exactly as GNU as does.
    match sysreg.as_str() {
        "daifset" => return msr_pstate_imm_only(operands, 1, 3, 0b110, 0, "daifset"),
        "daifclr" => return msr_pstate_imm_only(operands, 1, 3, 0b111, 0, "daifclr"),
        // SPSel: one bit (bits 1-3 are RES0, and GNU as rejects
        // `msr spsel,#2`), op1=0, CRm=4, op2=5.
        "spsel" => {
            if let Some(word) = msr_pstate_field(operands, 1, 0, 0b101, 0, "spsel")? {
                return Ok(word);
            }
        }
        // The pointer-authentication/speculation-control fields, each one bit:
        // PAN (0, 4, 4), UAO (0, 4, 3), SSBS/DIT/TCO (3, 4, 1/2/4) and
        // ALLINT (1, 4, 0).  The first five all have register forms too, so a
        // non-immediate operand falls through like PAN's does.
        "uao" => {
            if let Some(word) = msr_pstate_field(operands, 1, 0, 0b011, 0, "uao")? {
                return Ok(word);
            }
        }
        "allint" => {
            if let Some(word) = msr_pstate_field(operands, 1, 1, 0b000, 0, "allint")? {
                return Ok(word);
            }
        }
        // The SME state fields (SVCR's S<M|Z> bits) are immediate-only, and
        // their one-bit immediate lands in CRm's low bit: `msr svcrsm,#1` is
        // CRm=3.  GNU as rejects `msr svcrsm,x0` (there is no such register
        // name), so these do not fall through.
        "svcrsm" => return msr_pstate_imm_only(operands, 1, 3, 0b011, 0b010, "svcrsm"),
        "svcrza" => return msr_pstate_imm_only(operands, 1, 3, 0b011, 0b100, "svcrza"),
        "svcrsmza" => return msr_pstate_imm_only(operands, 1, 3, 0b011, 0b110, "svcrsmza"),
        "pan" => {
            if let Some(word) = msr_pstate_field(operands, 1, 0, 0b100, 0, "pan")? {
                return Ok(word);
            }
        }
        "ssbs" => {
            if let Some(word) = msr_pstate_field(operands, 1, 3, 0b001, 0, "ssbs")? {
                return Ok(word);
            }
        }
        "dit" => {
            if let Some(word) = msr_pstate_field(operands, 1, 3, 0b010, 0, "dit")? {
                return Ok(word);
            }
        }
        "tco" => {
            if let Some(word) = msr_pstate_field(operands, 1, 3, 0b100, 0, "tco")? {
                return Ok(word);
            }
        }
        _ => {}
    }

    // MSR (register): msr sysreg, Xt -- X register only (see encode_mrs).
    let rt = match operands.get(1) {
        Some(Operand::Reg(name)) => gp_xt(name, "msr")?,
        _ => return Err("msr: expected Xt source register".to_string()),
    };

    let encoding = sysreg_encoding_named(&sysreg)?;

    // MSR encoding: 0xd500_0000 has L=0 (bit 21) for write.
    // Bits [20:19] = op0, supplied entirely by the sysreg encoding field.
    let word = 0xd5000000 | (encoding << 5) | rt;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_hlt(operands: &[Operand]) -> Result<EncodeResult, String> {
    let imm = get_imm(operands, 0)?;
    if !(0..=0xFFFF).contains(&imm) {
        return Err(format!(
            "hlt: immediate {imm} does not fit the 16-bit comment field (allowed 0..=65535)"
        ));
    }
    Ok(EncodeResult::Word(0xd4400000 | ((imm as u32) << 5)))
}

pub(crate) fn encode_svc(operands: &[Operand]) -> Result<EncodeResult, String> {
    let imm = imm_in_range(get_imm(operands, 0)?, 0, 0xFFFF, "svc", "immediate")?;
    let word = 0xd4000001 | (imm << 5);
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_hvc(operands: &[Operand]) -> Result<EncodeResult, String> {
    let imm = imm_in_range(get_imm(operands, 0)?, 0, 0xFFFF, "hvc", "immediate")?;
    let word = 0xd4000002 | (imm << 5);
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_ic(raw_operands: &str) -> Result<EncodeResult, String> {
    let parts: Vec<&str> = raw_operands.splitn(2, ',').collect();
    let op_name = parts[0].trim().to_lowercase();
    let base = match op_name.as_str() {
        "ialluis" => 0xd508711fu32,
        "iallu" => 0xd508751f,
        "ivau" => 0xd50b7520,
        _ => return Err(format!("unsupported ic operation: {}", op_name)),
    };
    let rt = sys_class_rt(
        (parts.len() > 1).then(|| parts[1].trim()),
        base,
        "ic",
    )?;
    let word = (base & !0x1F) | rt;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_smc(operands: &[Operand]) -> Result<EncodeResult, String> {
    let imm = imm_in_range(get_imm(operands, 0)?, 0, 0xFFFF, "smc", "immediate")?;
    let word = 0xd4000003 | (imm << 5);
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_at(_operands: &[Operand], raw_operands: &str) -> Result<EncodeResult, String> {
    let parts: Vec<&str> = raw_operands.splitn(2, ',').collect();
    let op_name = parts[0].trim().to_lowercase();
    // AT encoding: SYS instruction. Base words from GCC (Rt = 0):
    let base = match op_name.as_str() {
        "s1e1r" => 0xd5087800u32,
        "s1e1w" => 0xd5087820,
        "s1e0r" => 0xd5087840,
        "s1e0w" => 0xd5087860,
        _ => return Err(format!("unsupported at operation: {}", op_name)),
    };
    let rt = sys_class_rt((parts.len() > 1).then(|| parts[1].trim()), base, "at")?;
    let word = (base & !0x1F) | rt;
    Ok(EncodeResult::Word(word))
}

/// Encode `sys #op1, Cn, Cm, #op2, Xt` instruction.
pub(crate) fn encode_sys(raw_operands: &str) -> Result<EncodeResult, String> {
    let parts: Vec<&str> = raw_operands.split(',').map(|s| s.trim()).collect();
    if parts.len() < 4 {
        return Err(format!(
            "sys needs at least 4 operands, got: {}",
            raw_operands
        ));
    }
    let op1: u32 = parts[0]
        .trim_start_matches('#')
        .trim()
        .parse()
        .map_err(|_| format!("sys: invalid op1: {}", parts[0]))?;
    let crn: u32 = parts[1]
        .trim()
        .to_lowercase()
        .trim_start_matches('c')
        .parse()
        .map_err(|_| format!("sys: invalid CRn: {}", parts[1]))?;
    let crm: u32 = parts[2]
        .trim()
        .to_lowercase()
        .trim_start_matches('c')
        .parse()
        .map_err(|_| format!("sys: invalid CRm: {}", parts[2]))?;
    let op2: u32 = parts[3]
        .trim_start_matches('#')
        .trim()
        .parse()
        .map_err(|_| format!("sys: invalid op2: {}", parts[3]))?;
    if op1 > 7 {
        return Err(format!("sys: op1 #{op1} is out of range (0 to 7)"));
    }
    if crn > 15 {
        return Err(format!("sys: CRn #{crn} is out of range (0 to 15)"));
    }
    if crm > 15 {
        return Err(format!("sys: CRm #{crm} is out of range (0 to 15)"));
    }
    if op2 > 7 {
        return Err(format!("sys: op2 #{op2} is out of range (0 to 7)"));
    }
    let rt = if parts.len() >= 5 {
        gp_xt(parts[4].trim(), "sys")?
    } else {
        31 // xzr if no register specified
    };
    let word = 0xd5080000 | (op1 << 16) | (crn << 12) | (crm << 8) | (op2 << 5) | rt;
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_brk(operands: &[Operand]) -> Result<EncodeResult, String> {
    let imm = imm_in_range(get_imm(operands, 0)?, 0, 0xFFFF, "brk", "immediate")?;
    let word = 0xd4200000 | (imm << 5);
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_tlbi(
    _operands: &[Operand],
    raw_operands: &str,
) -> Result<EncodeResult, String> {
    let parts: Vec<&str> = raw_operands.splitn(2, ',').collect();
    let op_name = parts[0].trim().to_lowercase();
    // TLBI encoding: SYS instruction with fixed fields
    // Full word from GCC objdump for known ops (with Rt = 31 for the
    // broadcast operations, which take no register):
    let base = match op_name.as_str() {
        // Standard ARMv8.0 TLBI operations
        "vmalle1is" => 0xd508831fu32,
        "vmalle1" => 0xd508871f,
        "alle1is" => 0xd50c839f,
        "alle1" => 0xd50c879f,
        "alle2is" => 0xd50c831f,
        "vale1is" => 0xd50883a0,
        "vale1" => 0xd50887a0,
        "vale2is" => 0xd50c83a0,
        "vale2" => 0xd50c87a0,
        "vaae1is" => 0xd5088360,
        "vaae1" => 0xd5088760,
        "vaale1is" => 0xd50883e0,
        "vaale1" => 0xd50887e0,
        "vae1is" => 0xd5088320,
        "vae1" => 0xd5088720,
        "vae2is" => 0xd50c8320,
        "vae2" => 0xd50c8720,
        "aside1is" => 0xd5088340,
        "aside1" => 0xd5088740,
        "vmalls12e1is" => 0xd50c83df,
        "vmalls12e1" => 0xd50c87df,
        "ipas2e1is" => 0xd50c8020,
        "ipas2e1" => 0xd50c8420,
        "ipas2le1is" => 0xd50c80a0,
        "ipas2le1" => 0xd50c84a0,
        // FEAT_TLBIRANGE: range TLBI operations (ARMv8.4-A)
        "rvae1is" => 0xd5088220,
        "rvale1is" => 0xd50882a0,
        "rvaae1is" => 0xd5088260,
        "rvaale1is" => 0xd50882e0,
        "rvae1" => 0xd5088620,
        "rvale1" => 0xd50886a0,
        "rvaae1" => 0xd5088660,
        "rvaale1" => 0xd50886e0,
        "rvae1os" => 0xd5088520,
        "rvale1os" => 0xd50885a0,
        "rvaae1os" => 0xd5088560,
        "rvaale1os" => 0xd50885e0,
        "ripas2e1is" => 0xd50c8040,
        "ripas2e1" => 0xd50c8440,
        "ripas2e1os" => 0xd50c8460,
        "ripas2le1is" => 0xd50c80c0,
        "ripas2le1" => 0xd50c84c0,
        "ripas2le1os" => 0xd50c84e0,
        _ => return Err(format!("unsupported tlbi operation: {}", op_name)),
    };
    let rt = sys_class_rt((parts.len() > 1).then(|| parts[1].trim()), base, "tlbi")?;
    // Replace the Rt field (bits 4:0).
    let word = (base & !0x1F) | rt;
    Ok(EncodeResult::Word(word))
}

/// Encode HINT #imm (system hint instruction)
pub(crate) fn encode_bti(raw_operands: &str) -> Result<EncodeResult, String> {
    let target = raw_operands.trim().to_lowercase();
    let word = match target.as_str() {
        "" => 0xd503241f,   // bti (no target)
        "c" => 0xd503245f,  // bti c
        "j" => 0xd503249f,  // bti j
        "jc" => 0xd50324df, // bti jc
        _ => return Err(format!("unsupported bti target: {}", target)),
    };
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_hint(operands: &[Operand]) -> Result<EncodeResult, String> {
    // HINT #imm is a 7-bit immediate (CRm:op2). Masking used to wrap #128 to #0.
    let imm = imm_in_range(get_imm(operands, 0)?, 0, 127, "hint", "immediate")?;
    let crm = imm >> 3;
    let op2 = imm & 0x7;
    let word = 0xd503201f | (crm << 8) | (op2 << 5);
    Ok(EncodeResult::Word(word))
}

pub(crate) fn encode_dc(operands: &[Operand], raw_operands: &str) -> Result<EncodeResult, String> {
    // Check for the operation type in the operands or raw string
    let op = match operands.first() {
        Some(Operand::Symbol(s)) => s.to_lowercase(),
        _ => raw_operands.to_lowercase(),
    };

    // Find the register operand (second operand or last operand). DC takes
    // Xt; a W register previously encoded as the matching X register.
    let rt = match operands.get(1) {
        Some(Operand::Reg(name)) => gp_xt(name, "dc")?,
        _ => {
            if let Some(Operand::Reg(name)) = operands.last() {
                gp_xt(name, "dc")?
            } else {
                return Err("dc: expected Xt operand".to_string());
            }
        }
    };

    if op.contains("civac") {
        // DC CIVAC: sys #3, c7, c14, #1, Xt
        let word = 0xd50b7e20 | rt;
        return Ok(EncodeResult::Word(word));
    }
    if op.contains("cvac") {
        // DC CVAC: sys #3, c7, c10, #1, Xt
        let word = 0xd50b7a20 | rt;
        return Ok(EncodeResult::Word(word));
    }
    if op.contains("cvap") {
        // DC CVAP: sys #3, c7, c12, #1, Xt
        let word = 0xd50b7c20 | rt;
        return Ok(EncodeResult::Word(word));
    }
    if op.contains("cvau") {
        let word = 0xd50b7b20 | rt;
        return Ok(EncodeResult::Word(word));
    }
    if op.contains("ivac") {
        let word = 0xd5087620 | rt;
        return Ok(EncodeResult::Word(word));
    }
    if op.contains("zva") {
        // DC ZVA: sys #3, c7, c4, #1, Xt
        let word = 0xd50b7420 | rt;
        return Ok(EncodeResult::Word(word));
    }

    Err(format!("unsupported dc variant: {}", raw_operands))
}
