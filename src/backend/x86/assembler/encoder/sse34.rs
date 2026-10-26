//! Legacy SSE3/SSSE3/SSE4 operand roles. An opcode helper is not an ISA
//! permission: dropping 66 for an arbitrary MMX operand can encode a different
//! instruction. Shared with i686 so both modes reject the same wrong classes.
use super::*;

pub(crate) fn check_sse34_operands(
    m: &str,
    ops: &[Operand],
    long_mode: bool,
) -> Result<(), String> {
    let xmm = |o: &Operand| {
        matches!(o, Operand::Register(r)
        if r.name.strip_prefix("xmm").and_then(|s| s.parse::<u8>().ok())
            .is_some_and(|n| n < if long_mode { 16 } else { 8 }))
    };
    let mm = |o: &Operand| {
        matches!(o, Operand::Register(r)
        if r.name.strip_prefix("mm").and_then(|s| s.parse::<u8>().ok()).is_some_and(|n| n < 8))
    };
    let mem = |o: &Operand| {
        matches!(o, Operand::Memory(_)) || matches!(o, Operand::Label(s) if !s.starts_with('{'))
    };
    let dual = matches!(
        m,
        "pabsb"
            | "pabsw"
            | "pabsd"
            | "phaddw"
            | "phaddd"
            | "phaddsw"
            | "phsubw"
            | "phsubd"
            | "phsubsw"
            | "pmaddubsw"
            | "pmulhrsw"
            | "pshufb"
            | "psignb"
            | "psignw"
            | "psignd"
    );
    let binary = dual
        || matches!(
            m,
            "addsubpd"
                | "addsubps"
                | "haddpd"
                | "haddps"
                | "hsubpd"
                | "hsubps"
                | "movddup"
                | "movshdup"
                | "movsldup"
                | "packusdw"
                | "pcmpeqq"
                | "pcmpgtq"
                | "phminposuw"
                | "pmaxsb"
                | "pmaxsd"
                | "pmaxuw"
                | "pmaxud"
                | "pminsb"
                | "pminsd"
                | "pminuw"
                | "pminud"
                | "pmovsxbw"
                | "pmovsxbd"
                | "pmovsxbq"
                | "pmovsxwd"
                | "pmovsxwq"
                | "pmovsxdq"
                | "pmovzxbw"
                | "pmovzxbd"
                | "pmovzxbq"
                | "pmovzxwd"
                | "pmovzxwq"
                | "pmovzxdq"
                | "pmuldq"
                | "pmulld"
                | "ptest"
        );
    let gp_size = |o: &Operand| match o {
        Operand::Register(r) if is_reg8(&r.name) => Some(1),
        Operand::Register(r) if is_reg16(&r.name) => Some(2),
        Operand::Register(r) if is_reg32(&r.name) => Some(4),
        Operand::Register(r) if long_mode && is_reg64(&r.name) => Some(8),
        _ => None,
    };
    let valid = if matches!(m, "crc32" | "crc32b" | "crc32w" | "crc32l" | "crc32q") {
        if let [a, b] = ops {
            let width = match m {
                "crc32b" => 1,
                "crc32w" => 2,
                "crc32q" => 8,
                "crc32l" => 4,
                _ => gp_size(a).or_else(|| gp_size(b)).unwrap_or(4),
            };
            let dest = gp_size(b);
            let rex_high_byte = matches!((a,b), (Operand::Register(src), Operand::Register(dst))
                if matches!(src.name.as_str(), "ah" | "bh" | "ch" | "dh")
                    && (dest == Some(8) || gp_id(&dst.name).is_some_and(|n| n >= 8)));
            (dest == Some(4) && matches!(width, 1 | 2 | 4)
                || dest == Some(8) && matches!(width, 1 | 8))
                && (mem(a) || gp_size(a) == Some(width))
                && !rex_high_byte
        } else {
            false
        }
    } else if binary {
        matches!(ops, [a,b] if (xmm(b) && (xmm(a) || mem(a)))
            || (dual && mm(b) && (mm(a) || mem(a))))
    } else if matches!(m, "blendvpd" | "blendvps" | "pblendvb") {
        let pair = if let [Operand::Register(mask), rest @ ..] = ops {
            if ops.len() == 3 {
                if mask.name != "xmm0" {
                    return Err("legacy blend mask must be xmm0".into());
                }
                rest
            } else {
                ops
            }
        } else {
            ops
        };
        matches!(pair, [a,b] if xmm(b) && (xmm(a) || mem(a)))
    } else if matches!(m, "lddqu" | "movntdqa") {
        matches!(ops, [a,b] if mem(a) && xmm(b))
    } else if matches!(m, "movntsd" | "movntss") {
        matches!(ops, [a,b] if xmm(a) && mem(b))
    } else if matches!(m, "extrq" | "insertq") {
        let imm = |o: &Operand| matches!(o, Operand::Immediate(ImmediateValue::Integer(n)) if (0..=255).contains(n));
        matches!(ops, [a,b] if xmm(a) && xmm(b))
            || (m == "extrq" && matches!(ops, [a,b,c] if imm(a) && imm(b) && xmm(c)))
            || (m == "insertq" && matches!(ops, [a,b,c,d] if imm(a) && imm(b) && xmm(c) && xmm(d)))
    } else {
        // Immediate forms, lane insert/extract, CRC and system instructions
        // have their own role-specific validators/encoders.
        return Ok(());
    };
    if valid {
        Ok(())
    } else {
        Err(format!("invalid operand classes for {m}"))
    }
}

impl InstructionEncoder {
    pub(crate) fn encode_sse4a_bits(
        &mut self,
        ops: &[Operand],
        insert: bool,
    ) -> Result<(), String> {
        let prefix = if insert { 0xF2 } else { 0x66 };
        if ops.len() == 2 {
            return self.encode_sse_op(ops, &[prefix, 0x0F, 0x79]);
        }
        let expected = if insert { 4 } else { 3 };
        if ops.len() != expected {
            return Err(format!("SSE4a immediate form requires {expected} operands"));
        }
        let (
            Operand::Immediate(ImmediateValue::Integer(length)),
            Operand::Immediate(ImmediateValue::Integer(index)),
        ) = (&ops[0], &ops[1])
        else {
            return Err("SSE4a bit controls must be immediate bytes".into());
        };
        let Operand::Register(dst) = &ops[ops.len() - 1] else {
            return Err("SSE4a requires XMM destination".into());
        };
        self.bytes.push(prefix);
        if insert {
            let Operand::Register(src) = &ops[2] else {
                return Err("INSERTQ requires XMM source".into());
            };
            self.emit_rex_rr(0, &dst.name, &src.name);
            self.bytes.extend_from_slice(&[0x0F, 0x78]);
            self.bytes.push(self.modrm(
                3,
                reg_num(&dst.name).ok_or("bad XMM")?,
                reg_num(&src.name).ok_or("bad XMM")?,
            ));
        } else {
            self.emit_rex_rr(0, "", &dst.name);
            self.bytes.extend_from_slice(&[0x0F, 0x78]);
            self.bytes
                .push(self.modrm(3, 0, reg_num(&dst.name).ok_or("bad XMM")?));
        }
        // AT&T lists length,index; architectural byte order is index,length.
        self.bytes.extend_from_slice(&[*index as u8, *length as u8]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::apx_tests::{fails, hex};
    #[test]
    fn saturation_strings_and_sse4a_have_distinct_prefixes_and_fields() {
        for (source, bytes) in [
            ("phaddsw %mm1,%mm2", "0f 38 03 d1"),
            ("rex64 pcmpestri $0,%xmm1,%xmm2", "66 48 0f 3a 61 d1 00"),
            ("rex64 pcmpestrm $0,%xmm9,%xmm10", "66 4d 0f 3a 60 d1 00"),
            ("rex movddup %xmm1,%xmm2", "f2 40 0f 12 d1"),
            ("rex crc32b %al,%eax", "f2 40 0f 38 f0 c0"),
            ("rex64 crc32b %al,%rax", "f2 48 0f 38 f0 c0"),
            ("phsubsw %xmm1,%xmm2", "66 0f 38 07 d1"),
            ("pcmpistri $255,%xmm1,%xmm2", "66 0f 3a 63 d1 ff"),
            ("pcmpestrm $1,%xmm1,%xmm2", "66 0f 3a 60 d1 01"),
            ("extrq $7,$9,%xmm2", "66 0f 78 c2 09 07"),
            ("insertq $7,$9,%xmm1,%xmm2", "f2 0f 78 d1 09 07"),
        ] {
            assert_eq!(hex(source), bytes, "{source}");
        }
    }
    #[test]
    fn reject_false_simd_classes_and_crc_widths() {
        for source in [
            "haddpd %mm0,%mm1",
            "pmulld %mm0,%mm1",
            "blendvps %xmm1,%xmm2,%xmm3",
            "lddqu %xmm1,%xmm2",
            "movntdqa %xmm1,%xmm2",
            "crc32w %ax,%rax",
            "crc32l %eax,%rax",
            "crc32b %xmm0,%eax",
            "rex crc32b %ah,%eax",
            "rex64 crc32b %ah,%rax",
            "extrq $256,$1,%xmm0",
            "insertq (%rax),%xmm0",
        ] {
            assert!(fails(source), "{source}");
        }
    }
}
