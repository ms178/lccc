//! AVX2 widening operations read a 128-bit value and produce 256 bits.
//! The source width must not be inferred from the destination register width.
use crate::common::types::CType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AvxWidenKind {
    ByteToWord,
    ByteToDword,
    ByteToQword,
    WordToDword,
    WordToQword,
    DwordToQword,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AvxWiden {
    pub signed: bool,
    pub kind: AvxWidenKind,
}

impl AvxWiden {
    pub fn mnemonic(self) -> &'static str {
        use AvxWidenKind::*;
        match (self.signed, self.kind) {
            (true, ByteToWord) => "vpmovsxbw",
            (true, ByteToDword) => "vpmovsxbd",
            (true, ByteToQword) => "vpmovsxbq",
            (true, WordToDword) => "vpmovsxwd",
            (true, WordToQword) => "vpmovsxwq",
            (true, DwordToQword) => "vpmovsxdq",
            (false, ByteToWord) => "vpmovzxbw",
            (false, ByteToDword) => "vpmovzxbd",
            (false, ByteToQword) => "vpmovzxbq",
            (false, WordToDword) => "vpmovzxwd",
            (false, WordToQword) => "vpmovzxwq",
            (false, DwordToQword) => "vpmovzxdq",
        }
    }

    /// GCC's builtin results use signed vector carrier types even for PMOVZX.
    pub fn result_element(self) -> CType {
        use AvxWidenKind::*;
        match self.kind {
            ByteToWord => CType::Short,
            ByteToDword | WordToDword => CType::Int,
            ByteToQword | WordToQword | DwordToQword => CType::LongLong,
        }
    }

    pub fn source_bits(self) -> usize {
        use AvxWidenKind::*;
        match self.kind {
            ByteToWord | ByteToDword | ByteToQword => 8,
            WordToDword | WordToQword => 16,
            DwordToQword => 32,
        }
    }

    pub fn raw_builtin(name: &str) -> Option<Self> {
        AVX_WIDEN
            .iter()
            .find_map(|&(_, raw, _, op)| (raw == name).then_some(op))
    }
}

/// Public, GNU-native and private header-pointer ABI names.
pub const AVX_WIDEN: &[(&str, &str, &str, AvxWiden)] = &[
    (
        "_mm256_cvtepi8_epi16",
        "__builtin_ia32_pmovsxbw256",
        "__lccc_avx2_pmovsxbw256",
        AvxWiden {
            signed: true,
            kind: AvxWidenKind::ByteToWord,
        },
    ),
    (
        "_mm256_cvtepi8_epi32",
        "__builtin_ia32_pmovsxbd256",
        "__lccc_avx2_pmovsxbd256",
        AvxWiden {
            signed: true,
            kind: AvxWidenKind::ByteToDword,
        },
    ),
    (
        "_mm256_cvtepi8_epi64",
        "__builtin_ia32_pmovsxbq256",
        "__lccc_avx2_pmovsxbq256",
        AvxWiden {
            signed: true,
            kind: AvxWidenKind::ByteToQword,
        },
    ),
    (
        "_mm256_cvtepi16_epi32",
        "__builtin_ia32_pmovsxwd256",
        "__lccc_avx2_pmovsxwd256",
        AvxWiden {
            signed: true,
            kind: AvxWidenKind::WordToDword,
        },
    ),
    (
        "_mm256_cvtepi16_epi64",
        "__builtin_ia32_pmovsxwq256",
        "__lccc_avx2_pmovsxwq256",
        AvxWiden {
            signed: true,
            kind: AvxWidenKind::WordToQword,
        },
    ),
    (
        "_mm256_cvtepi32_epi64",
        "__builtin_ia32_pmovsxdq256",
        "__lccc_avx2_pmovsxdq256",
        AvxWiden {
            signed: true,
            kind: AvxWidenKind::DwordToQword,
        },
    ),
    (
        "_mm256_cvtepu8_epi16",
        "__builtin_ia32_pmovzxbw256",
        "__lccc_avx2_pmovzxbw256",
        AvxWiden {
            signed: false,
            kind: AvxWidenKind::ByteToWord,
        },
    ),
    (
        "_mm256_cvtepu8_epi32",
        "__builtin_ia32_pmovzxbd256",
        "__lccc_avx2_pmovzxbd256",
        AvxWiden {
            signed: false,
            kind: AvxWidenKind::ByteToDword,
        },
    ),
    (
        "_mm256_cvtepu8_epi64",
        "__builtin_ia32_pmovzxbq256",
        "__lccc_avx2_pmovzxbq256",
        AvxWiden {
            signed: false,
            kind: AvxWidenKind::ByteToQword,
        },
    ),
    (
        "_mm256_cvtepu16_epi32",
        "__builtin_ia32_pmovzxwd256",
        "__lccc_avx2_pmovzxwd256",
        AvxWiden {
            signed: false,
            kind: AvxWidenKind::WordToDword,
        },
    ),
    (
        "_mm256_cvtepu16_epi64",
        "__builtin_ia32_pmovzxwq256",
        "__lccc_avx2_pmovzxwq256",
        AvxWiden {
            signed: false,
            kind: AvxWidenKind::WordToQword,
        },
    ),
    (
        "_mm256_cvtepu32_epi64",
        "__builtin_ia32_pmovzxdq256",
        "__lccc_avx2_pmovzxdq256",
        AvxWiden {
            signed: false,
            kind: AvxWidenKind::DwordToQword,
        },
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_signed_unsigned_shapes_and_native_result_types() {
        assert_eq!(AVX_WIDEN.len(), 12);
        for (index, &(public, raw, private, op)) in AVX_WIDEN.iter().enumerate() {
            assert_eq!(AvxWiden::raw_builtin(raw), Some(op));
            assert_eq!(AvxWiden::raw_builtin(public), None);
            assert_eq!(AvxWiden::raw_builtin(private), None);
            assert!(matches!(op.source_bits(), 8 | 16 | 32));
            assert!(matches!(
                op.result_element(),
                CType::Short | CType::Int | CType::LongLong
            ));
            let ir = crate::ir::intrinsics::IntrinsicOp::AvxWiden(op);
            assert_eq!(ir.vector_result_width(), Some(32));
            assert!(ir.reads_pointer_arg());
            for &(p, r, h, other) in &AVX_WIDEN[index + 1..] {
                assert_ne!(public, p);
                assert_ne!(raw, r);
                assert_ne!(private, h);
                assert_ne!(op, other);
            }
        }
    }
}
