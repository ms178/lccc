//! Exact byte provenance for portable 32/64-bit byte-swap networks.
//!
//! Each lane is either zero or one unchanged byte of a single SSA source.
//! Logical byte shifts, whole-byte masks and compatible OR/XOR merges preserve
//! that invariant. A final reversed permutation is therefore exactly Bswap.
//! No arithmetic shifts, narrowing casts, memory operations or producers are
//! changed. Fixed arrays and per-candidate walk/depth budgets bound compile cost.
use super::*;

struct Bytes<'a> {
    defs: &'a [Option<Instruction>],
    source: Option<Operand>,
    width: usize,
    visits: u16,
}

impl Bytes<'_> {
    fn walk(&mut self, value: Operand, depth: u8) -> Option<[u8; 8]> {
        self.visits = self.visits.checked_sub(1)?;
        if depth == 0 {
            return None;
        }
        if let Some(c) = const_in_ring(value, self.width * 8) {
            return (c == 0).then_some([0; 8]);
        }
        let Operand::Value(v) = value else {
            return None;
        };
        let definition = self.defs.get(v.0 as usize)?.as_ref();
        if let Some(Instruction::Copy { src, .. }) = definition {
            return self.walk(*src, depth - 1);
        }
        if let Some(Instruction::BinOp {
            op, lhs, rhs, ty, ..
        }) = definition
        {
            // Do not interpret a different ring as this word's byte network.
            if !ty.is_integer() || ty.size() != self.width {
                return None;
            }
            let (op, lhs, rhs) = (*op, *lhs, *rhs);
            match op {
                IrBinOp::Shl | IrBinOp::LShr => {
                    let Operand::Const(count) = rhs else {
                        return None;
                    };
                    let shift = count.to_i128()?;
                    if !(0..(self.width * 8) as i128).contains(&shift) || shift % 8 != 0 {
                        return None;
                    }
                    let input = self.walk(lhs, depth - 1)?;
                    let amount = shift as usize / 8;
                    let mut output = [0; 8];
                    if op == IrBinOp::Shl {
                        output[amount..self.width].copy_from_slice(&input[..self.width - amount]);
                    } else {
                        output[..self.width - amount].copy_from_slice(&input[amount..self.width]);
                    }
                    return Some(output);
                }
                IrBinOp::And => {
                    let (input, mask) = if let Some(c) = const_in_ring(rhs, self.width * 8) {
                        (lhs, c)
                    } else {
                        (rhs, const_in_ring(lhs, self.width * 8)?)
                    };
                    let mut bytes = self.walk(input, depth - 1)?;
                    for (i, byte) in bytes[..self.width].iter_mut().enumerate() {
                        match (mask >> (i * 8)) & 255 {
                            0 => *byte = 0,
                            255 => {}
                            _ => return None,
                        }
                    }
                    return Some(bytes);
                }
                IrBinOp::Or | IrBinOp::Xor => {
                    let mut a = self.walk(lhs, depth - 1)?;
                    let b = self.walk(rhs, depth - 1)?;
                    for i in 0..self.width {
                        match (a[i], b[i]) {
                            (0, byte) => a[i] = byte,
                            (_, 0) => {}
                            (x, y) if op == IrBinOp::Or && x == y => {}
                            _ => return None,
                        }
                    }
                    return Some(a);
                }
                _ => {}
            }
        }
        // A cast or other computation can be the opaque input, but is never
        // peeled: independently extended/truncated values are distinct sources.
        match self.source {
            None => self.source = Some(value),
            Some(source) if source == value => {}
            _ => return None,
        }
        let mut identity = [0; 8];
        for (i, byte) in identity[..self.width].iter_mut().enumerate() {
            *byte = i as u8 + 1;
        }
        Some(identity)
    }
}

pub(super) fn bswap(value: Operand, ty: IrType, defs: &[Option<Instruction>]) -> Option<Operand> {
    if !matches!(ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64) {
        return None;
    }
    let mut bytes = Bytes {
        defs,
        source: None,
        width: ty.size(),
        visits: 128,
    };
    let permutation = bytes.walk(value, 24)?;
    if permutation[..bytes.width]
        .iter()
        .enumerate()
        .all(|(i, &byte)| byte as usize == bytes.width - i)
    {
        bytes.source
    } else {
        None
    }
}
