//! x86-64 peephole optimizer: types, line classification, and utility functions.
//!
//! Contains the core data structures (LineInfo, LineKind, LineStore) and the
//! classify_line function that pre-parses assembly lines into compact metadata
//! for fast pattern matching in the optimization passes.

// ── Pre-parsed line metadata ─────────────────────────────────────────────────

/// Register identifier (0..=15 for GPRs, 255 = unknown/none).
/// Matches the x86 register family numbering.
pub(super) type RegId = u8;
pub(super) const REG_NONE: RegId = 255;
/// Maximum register family ID for general-purpose registers (rax..r15 = 0..15).
/// MMX (16..23) and XMM (24..39) families are recognized by register_family_fast
/// but are not tracked by the global store forwarding pass (which only handles GP
/// register-to-register moves via reg_id_to_name).
pub(super) const REG_GP_MAX: RegId = 15;

/// Sentinel value for `rbp_offset` meaning "no %rbp reference" or "multiple/complex references".
pub(super) const RBP_OFFSET_NONE: i32 = i32::MIN;

/// Compact classification of a single assembly line.
/// Stored in a parallel array alongside the raw text, so hot loops
/// can check integer fields instead of re-parsing strings.
#[derive(Clone, Copy)]
pub(super) struct LineInfo {
    pub(super) kind: LineKind,
    /// Pre-parsed extension classification for redundant extension elimination.
    /// Avoids repeated `starts_with`/`ends_with` string comparisons in the hot
    /// `combined_local_pass` loop.
    pub(super) ext_kind: ExtKind,
    /// Byte offset of the first non-space character in the raw line.
    /// Caches `trim_asm` so passes don't repeatedly scan leading whitespace.
    pub(super) trim_start: u16,
    /// Cached result of `has_indirect_memory_access` for `Other` lines.
    /// `false` for all non-`Other` kinds. This avoids repeated byte scans in
    /// `eliminate_dead_stores` and `global_store_forwarding`.
    pub(super) has_indirect_mem: bool,
    /// Pre-parsed %rbp offset for `Other` lines that reference a stack slot
    /// (e.g., `leaq -24(%rbp), %rax`). `RBP_OFFSET_NONE` if no rbp reference,
    /// multiple references, or non-Other kind. This eliminates expensive
    /// `str::contains` checks in `eliminate_dead_stores`.
    pub(super) rbp_offset: i32,
    /// Bitmask of register families referenced in this line.
    /// Bit N set means register family N (0=rax, 1=rcx, ..., 15=r15) appears
    /// somewhere in the line text. Pre-computed during classify_line to eliminate
    /// O(n * patterns) `str::contains` calls in `eliminate_unused_callee_saves`.
    pub(super) reg_refs: u16,
    /// When true, this instruction must not be removed or modified by any
    /// peephole pass. Used for parameter pre-stores (movq %arg_reg, %callee_saved)
    /// that must survive across function calls.
    pub(super) pinned: bool,
}

/// What kind of assembly line this is, with pre-extracted fields for the
/// patterns we care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineKind {
    Nop,   // Deleted line (marked via LineInfo only)
    Empty, // Blank line

    /// `movX %reg, offset(%rbp)` – store register to stack slot
    StoreRbp {
        reg: RegId,
        offset: i32,
        size: MoveSize,
    },
    /// `movX offset(%rbp), %reg` or `movslq offset(%rbp), %reg` – load from stack slot
    LoadRbp {
        reg: RegId,
        offset: i32,
        size: MoveSize,
    },
    /// Scalar-FP / SSE stack store (`movsd|movss|movq|movd %xmmN, slot`) —
    /// same slot semantics as StoreRbp; kept separate because the XMM
    /// family ids (24-39) do not fit the GP reg_refs bitmask and the
    /// GP-oriented passes must not treat an XMM store as a GP def/use.
    StoreXmmRbp {
        offset: i32,
        size: MoveSize,
    },
    /// Scalar-FP / SSE stack load (`movsd|movss|movq|movd slot, %xmmN`).
    LoadXmmRbp {
        offset: i32,
        size: MoveSize,
    },

    /// `movq %reg, %reg` – self-move (pre-classified to avoid string ops in hot loop)
    SelfMove,

    Label,       // `name:`
    Jmp,         // `jmp label`
    JmpIndirect, // `jmpq *%rax` or `jmp __x86_indirect_thunk_rax`
    CondJmp,     // `je`/`jne`/`jl`/... label
    Call,        // `call ...`
    Ret,         // `ret`
    Push {
        reg: RegId,
    }, // `pushq %reg`
    Pop {
        reg: RegId,
    }, // `popq %reg`
    SetCC {
        reg: RegId,
    }, // `setCC %reg` (byte register; reg is family ID)
    Cmp,         // `cmpX`/`testX`/`ucomis*`
    Directive,   // Lines starting with `.`

    /// Everything else (regular instructions).
    /// `dest_reg` is the pre-parsed destination register family (REG_NONE if unknown).
    /// This allows fast register-modification checks without re-parsing.
    Other {
        dest_reg: RegId,
    },

    /// A line inside a `#APP`/`#NO_APP` inline-asm region. User-authored
    /// assembly is OPAQUE: it may be a deliberate length template (kernel
    /// ALTERNATIVE()), a patch site (jump labels, static_call), or have any
    /// side effect. No pass may remove, rewrite, or reason about it. Marked
    /// pinned with all reg_refs set and has_indirect_mem=true so every
    /// conservative fallback path treats it as clobbering everything.
    InlineAsm,
}

/// Pre-parsed classification of what kind of extension/operation an instruction performs.
/// Used by the redundant extension elimination pass to avoid repeated string comparisons
/// in the hot combined_local_pass loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExtKind {
    /// Not an extension or producer recognized for the extension elimination pass
    None,
    /// movzbq %al, %rax (or similar zero-extend of %al to %rax)
    MovzbqAlRax,
    /// movzwq %ax, %rax
    MovzwqAxRax,
    /// movsbq %al, %rax
    MovsbqAlRax,
    /// movslq %eax, %rax
    MovslqEaxRax,
    /// movl %eax, %eax (zero-extend of lower 32 bits)
    MovlEaxEax,
    /// cltq instruction
    Cltq,
    /// Producer that writes to %rax with movzbq
    ProducerMovzbqToRax,
    /// Producer that writes to %rax with movzwq
    ProducerMovzwqToRax,
    /// Producer that writes to %rax with movsbq
    ProducerMovsbqToRax,
    /// Producer that writes to %rax with movslq
    ProducerMovslqToRax,
    /// Producer: movq $const, %rax
    ProducerMovqConstRax,
    /// Producer: 32-bit arithmetic op (addl, subl, imull, andl, orl, xorl, shll, shrl)
    ProducerArith32,
    /// Producer: movl to %eax
    ProducerMovlToEax,
    /// Producer: movzbl to %eax or movzbq to %rax
    ProducerMovzbToEax,
    /// Producer: movzwl to %eax or movzwq to %rax
    ProducerMovzwToEax,
    /// Producer: divl or idivl %ecx
    ProducerDiv32,
    /// Producer: movq %REG, %rax (64-bit register-to-rax copy, REG != rax).
    /// Used for fusion: `movq %REG, %rax; movl %eax, %eax` -> `movl %REGd, %eax`.
    ProducerMovqRegToRax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)] // x86 mnemonic suffix conventions (SLQ = movslq)
pub(super) enum MoveSize {
    Q,   // movq  (64-bit)
    L,   // movl  (32-bit)
    W,   // movw  (16-bit)
    B,   // movb  (8-bit)
    SLQ, // movslq (sign-extend 32->64)
    SD,  // movsd (scalar double, 8 bytes)
    SS,  // movss (scalar single, 4 bytes)
}

impl MoveSize {
    pub(super) fn mnemonic(self) -> &'static str {
        match self {
            MoveSize::Q => "movq",
            MoveSize::L => "movl",
            MoveSize::W => "movw",
            MoveSize::B => "movb",
            MoveSize::SLQ => "movslq",
            MoveSize::SD => "movsd",
            MoveSize::SS => "movss",
        }
    }

    /// Return the number of bytes this move size covers.
    pub(super) fn byte_size(self) -> i32 {
        match self {
            MoveSize::Q | MoveSize::SD => 8,
            MoveSize::L | MoveSize::SLQ | MoveSize::SS => 4,
            MoveSize::W => 2,
            MoveSize::B => 1,
        }
    }
}

impl LineInfo {
    #[inline]
    pub(super) fn is_nop(self) -> bool {
        self.kind == LineKind::Nop
    }
    #[inline]
    pub(super) fn is_barrier(self) -> bool {
        matches!(
            self.kind,
            LineKind::Label | LineKind::Call | LineKind::Jmp | LineKind::JmpIndirect |
            LineKind::CondJmp | LineKind::Ret | LineKind::Directive |
            // ms178: push/pop (incl. pushfq/popfq with REG_NONE) read/write the
            // stack and adjust %rsp. Treating them as barriers keeps every
            // offset-tracking pass sound in their presence. The codegen keeps
            // logical slot offsets consistent across pushfq/popfq, but the
            // barrier only costs a little optimization locality, never
            // correctness.
            LineKind::Push { .. } | LineKind::Pop { .. }
        )
    }
    #[inline]
    pub(super) fn is_push(self) -> bool {
        matches!(self.kind, LineKind::Push { .. })
    }

    /// Get the trimmed content of a line using the cached trim offset.
    /// This avoids re-scanning leading whitespace on every access.
    #[inline]
    pub(super) fn trimmed<'a>(&self, line: &'a str) -> &'a str {
        let start = self.trim_start as usize;
        if start >= line.len() {
            // Line was replaced with shorter/empty text after classification
            line.trim()
        } else {
            &line[start..]
        }
    }
}

// ── Line parsing ─────────────────────────────────────────────────────────────

/// Helper to construct a LineInfo with default ext_kind and has_indirect_mem.
#[inline]
pub(super) fn line_info(kind: LineKind, ts: u16) -> LineInfo {
    LineInfo {
        kind,
        ext_kind: ExtKind::None,
        trim_start: ts,
        has_indirect_mem: false,
        rbp_offset: RBP_OFFSET_NONE,
        reg_refs: 0,
        pinned: false,
    }
}

/// Build a `LineInfo` for a `Cmp`/`test`/`ucomis` line, populating the
/// memory-operand caches (`has_indirect_mem`, `rbp_offset`) that the other
/// `line_info*` constructors fill for `Other` lines. Without these, a compare
/// with a stack operand such as `cmpl 40(%rsp), %eax` would be invisible to the
/// dead-store pass, which would then wrongly delete the store feeding it.
#[inline]
pub(super) fn cmp_line_info(ts: u16, s: &str, sb: &[u8]) -> LineInfo {
    let has_indirect = has_indirect_memory_access(s);
    let rbp_off = if has_indirect {
        RBP_OFFSET_NONE
    } else {
        parse_rbp_offset(s)
    };
    let reg_refs = scan_register_refs(sb);
    LineInfo {
        kind: LineKind::Cmp,
        ext_kind: ExtKind::None,
        trim_start: ts,
        has_indirect_mem: has_indirect,
        rbp_offset: rbp_off,
        reg_refs,
        pinned: false,
    }
}

/// Build a `LineInfo` for a scalar-FP / SSE frame-slot move
/// (`StoreXmmRbp` / `LoadXmmRbp`).
///
/// These lines used to be classified as `Other`, which populated the
/// memory-operand caches (`rbp_offset`, `has_indirect_mem`) and the GP
/// `reg_refs` bitmask (the `%rbp`/`%rsp` base). Every slot-tracking pass that
/// reaches an XMM slot move through a generic `_ =>` arm — windowed
/// dead-store elimination, frame compaction, store forwarding's range
/// invalidation, the `%rbp`-as-data-register escape checks — depends on
/// those caches. Constructing the new kinds with the bare `line_info` helper
/// left them empty, so a GP store feeding `movss slot, %xmmN` was deleted as
/// dead (gcc.c-torture 20000605-1.c, pr47538.c, pr59229.c, pr109938.c,
/// pr109986.c, 20021120-1.c at -O0; 920428-2.c/pr37573.c at -O1;
/// 20060420-1.c at -O2/-O3). Same contract as `cmp_line_info`.
#[inline]
pub(super) fn xmm_slot_line_info(kind: LineKind, ts: u16, s: &str, sb: &[u8]) -> LineInfo {
    debug_assert!(matches!(kind, LineKind::StoreXmmRbp { .. } | LineKind::LoadXmmRbp { .. }));
    let has_indirect = has_indirect_memory_access(s);
    let rbp_off = if has_indirect {
        RBP_OFFSET_NONE
    } else {
        parse_rbp_offset(s)
    };
    LineInfo {
        kind,
        ext_kind: ExtKind::None,
        trim_start: ts,
        has_indirect_mem: has_indirect,
        rbp_offset: rbp_off,
        reg_refs: scan_register_refs(sb),
        pinned: false,
    }
}

/// Helper to construct a LineInfo with default ext_kind but pre-scanned reg_refs.
#[inline]
pub(super) fn line_info_with_regs(kind: LineKind, ts: u16, reg_refs: u16) -> LineInfo {
    LineInfo {
        kind,
        ext_kind: ExtKind::None,
        trim_start: ts,
        has_indirect_mem: false,
        rbp_offset: RBP_OFFSET_NONE,
        reg_refs,
        pinned: false,
    }
}

/// Helper to construct a LineInfo with a specific ext_kind.
#[inline]
pub(super) fn line_info_ext(kind: LineKind, ext: ExtKind, ts: u16) -> LineInfo {
    LineInfo {
        kind,
        ext_kind: ext,
        trim_start: ts,
        has_indirect_mem: false,
        rbp_offset: RBP_OFFSET_NONE,
        reg_refs: 0,
        pinned: false,
    }
}

/// Parse one assembly line into a `LineInfo`.
pub(super) fn classify_line(raw: &str) -> LineInfo {
    let b = raw.as_bytes();

    // Compute trim offset once and cache it
    let trim_start = compute_trim_offset(b);
    debug_assert!(
        trim_start <= u16::MAX as usize,
        "assembly line with >65535 leading spaces"
    );
    let s = &raw[trim_start..];
    let sb = s.as_bytes();

    if sb.is_empty() {
        return line_info(LineKind::Empty, trim_start as u16);
    }

    let first = sb[0];
    let last = sb[sb.len() - 1];
    let ts = trim_start as u16;

    // Label: ends with ':'
    if last == b':' {
        return line_info(LineKind::Label, ts);
    }

    // Directive: starts with '.'
    if first == b'.' {
        return line_info(LineKind::Directive, ts);
    }

    // Scalar-FP / SSE XMM stack moves: classified precisely (offset + byte
    // size) so eliminate_never_read_stores can delete unread FP slots and
    // the read census is exact instead of a conservative 32-byte block.
    // Handled BEFORE the mov fast path (movsd/movss/movd are not 'mov' + a
    // GP-sized suffix) and also for the VEX memory forms (vmovsd/vmovss).
    if first == b'v' && sb.len() >= 7 && sb[1] == b'm' && sb[2] == b'o' && sb[3] == b'v' {
        if let Some((store, offset_str, size)) = parse_fp_slot_move(s) {
            let offset = fast_parse_i32(offset_str);
            return xmm_slot_line_info(
                if store {
                    LineKind::StoreXmmRbp { offset, size }
                } else {
                    LineKind::LoadXmmRbp { offset, size }
                },
                ts,
                s,
                sb,
            );
        }
    }

    // Fast path: only try store/load/self-move/extension parsing if line starts with 'mov' or 'movs'
    if first == b'm' && sb.len() >= 4 && sb[1] == b'o' && sb[2] == b'v' {
        if let Some((store, offset_str, size)) = parse_fp_slot_move(s) {
            let offset = fast_parse_i32(offset_str);
            return xmm_slot_line_info(
                if store {
                    LineKind::StoreXmmRbp { offset, size }
                } else {
                    LineKind::LoadXmmRbp { offset, size }
                },
                ts,
                s,
                sb,
            );
        }
        if let Some((reg_str, offset_str, size)) = parse_store_to_rbp_str(s) {
            let reg = register_family_fast(reg_str);
            // GP stores keep the historical StoreRbp classification.
            // XMM stores (movq %xmmN, slot) get the XMM slot kind — the
            // XMM family ids cannot be represented in the GP reg_refs
            // bitmask, and the GP consumers must not see them as GP.
            if reg <= REG_GP_MAX {
                let offset = fast_parse_i32(offset_str);
                let rr = (1u16 << reg) | (1u16 << 5);
                return line_info_with_regs(LineKind::StoreRbp { reg, offset, size }, ts, rr);
            }
            if is_xmm_family(reg) {
                let offset = fast_parse_i32(offset_str);
                return line_info(LineKind::StoreXmmRbp { offset, size }, ts);
            }
        }
        if let Some((offset_str, reg_str, size)) = parse_load_from_rbp_str(s) {
            let reg = register_family_fast(reg_str);
            if reg <= REG_GP_MAX {
                let offset = fast_parse_i32(offset_str);
                let ext = if reg == 0 {
                    match size {
                        MoveSize::SLQ => ExtKind::ProducerMovslqToRax,
                        MoveSize::L => ExtKind::ProducerMovlToEax,
                        _ => ExtKind::None,
                    }
                } else {
                    ExtKind::None
                };
                let rr = (1u16 << reg) | (1u16 << 5);
                let mut info = line_info_ext(LineKind::LoadRbp { reg, offset, size }, ext, ts);
                info.reg_refs = rr;
                return info;
            }
            if is_xmm_family(reg) {
                let offset = fast_parse_i32(offset_str);
                return line_info(LineKind::LoadXmmRbp { offset, size }, ts);
            }
        }
        // Check for self-move: movq %reg, %reg (same src and dst)
        if sb[3] == b'q' && sb.len() >= 6 && sb[4] == b' ' && is_self_move_fast(sb) {
            return line_info(LineKind::SelfMove, ts);
        }
        // Pre-classify extension-related instructions
        let ext = classify_mov_ext(s, sb);
        if ext != ExtKind::None {
            let dest_reg = parse_dest_reg_fast(s);
            let has_indirect = has_indirect_memory_access(s);
            let rbp_off = if has_indirect {
                RBP_OFFSET_NONE
            } else {
                parse_rbp_offset(s)
            };
            let reg_refs = scan_register_refs(sb);
            return LineInfo {
                kind: LineKind::Other { dest_reg },
                ext_kind: ext,
                trim_start: ts,
                has_indirect_mem: has_indirect,
                rbp_offset: rbp_off,
                reg_refs,
                pinned: false,
            };
        }
    }

    // Control flow: dispatch on first byte
    if first == b'j' {
        if sb.len() >= 4 && sb[1] == b'm' && sb[2] == b'p' {
            if sb[3] == b' ' {
                // `jmp label` or `jmp __x86_indirect_thunk_rax` or `jmp *%reg`
                if s.contains("indirect_thunk") {
                    return line_info_with_regs(LineKind::JmpIndirect, ts, scan_register_refs(sb));
                }
                // `jmp *%rcx` / `jmp *%rax` – indirect jump through register
                if sb.len() > 4 && sb[4] == b'*' {
                    return line_info_with_regs(LineKind::JmpIndirect, ts, scan_register_refs(sb));
                }
                return line_info(LineKind::Jmp, ts);
            }
            if sb[3] == b'q' && sb.len() >= 5 && sb[4] == b' ' {
                // `jmpq *%rax` – computed goto indirect jump
                return line_info_with_regs(LineKind::JmpIndirect, ts, scan_register_refs(sb));
            }
        }
        if is_conditional_jump(s) {
            return line_info(LineKind::CondJmp, ts);
        }
    }

    if first == b'c' {
        if sb.len() >= 4 && sb[1] == b'a' && sb[2] == b'l' && sb[3] == b'l' {
            return line_info_with_regs(LineKind::Call, ts, scan_register_refs(sb));
        }
        // Compare: cmpX
        if sb.len() >= 5 && sb[1] == b'm' && sb[2] == b'p' {
            return cmp_line_info(ts, s, sb);
        }
        // cltq - classify as extension producer
        if s == "cltq" {
            let mut info = line_info_ext(LineKind::Other { dest_reg: 0 }, ExtKind::Cltq, ts);
            info.reg_refs = 1u16 << 0; // implicitly references rax
            return info;
        }
    }

    if first == b'r' && s == "ret" {
        return line_info(LineKind::Ret, ts);
    }

    // Test instructions
    if first == b't' && sb.len() >= 5 && sb[1] == b'e' && sb[2] == b's' && sb[3] == b't' {
        return cmp_line_info(ts, s, sb);
    }

    // ucomis* instructions
    if first == b'u' && (s.starts_with("ucomisd ") || s.starts_with("ucomiss ")) {
        return cmp_line_info(ts, s, sb);
    }

    // Push / Pop (extract register for fast checks)
    if first == b'p' {
        if let Some(rest) = s.strip_prefix("pushq ") {
            let reg = register_family_fast(rest.trim());
            // Only use reg for bitmask if it's a GP register; otherwise use full scan
            let rr = if reg <= REG_GP_MAX {
                1u16 << reg
            } else {
                scan_register_refs(sb)
            };
            return line_info_with_regs(LineKind::Push { reg }, ts, rr);
        }
        if let Some(rest) = s.strip_prefix("popq ") {
            let reg = register_family_fast(rest.trim());
            let rr = if reg <= REG_GP_MAX {
                1u16 << reg
            } else {
                scan_register_refs(sb)
            };
            return line_info_with_regs(LineKind::Pop { reg }, ts, rr);
        }
        // ms178: pushfq/popfq (and 32-bit pushfl/popfl) are classified with
        // REG_NONE so they are distinguishable from real register pushes.
        // Codegen accounts for the %rsp shift by adjusting its logical frame
        // offsets, so the peephole's slot offsets stay consistent across them;
        // the passes treat them as barriers (see is_barrier) to be safe.
        // Previously they fell through to `Other`, which made every pass that
        // tracks stack offsets unsound in their presence — forcing Phase 2/3
        // to be skipped globally whenever ANY function used a Select (the
        // origin of pushfq), which disabled the whole global peephole for
        // files like gzip's deflate.c.
        if s.starts_with("pushf") || s.starts_with("pushfl") {
            return line_info(LineKind::Push { reg: REG_NONE }, ts);
        }
        if s.starts_with("popf") || s.starts_with("popfl") {
            return line_info(LineKind::Pop { reg: REG_NONE }, ts);
        }
    }

    // SetCC: extract the actual destination register (not always %al).
    // Inline asm can emit setCC to any byte register (e.g., sete %cl, seta %dl).
    if first == b's' && sb.len() >= 4 && sb[1] == b'e' && sb[2] == b't' && parse_setcc(s).is_some()
    {
        let setcc_reg = if let Some(space_pos) = s.rfind(' ') {
            let operand = s[space_pos + 1..].trim();
            register_family_fast(operand)
        } else {
            0 // default to rax family if can't parse
        };
        return line_info_with_regs(
            LineKind::SetCC { reg: setcc_reg },
            ts,
            scan_register_refs(sb),
        );
    }

    // Pre-classify 32-bit arithmetic producers for extension elimination
    let ext = classify_arith_ext(s, sb, first);

    // Pre-parse destination register for fast modification checks.
    let dest_reg = parse_dest_reg_fast(s);
    // Cache has_indirect_memory_access for Other lines
    let has_indirect = has_indirect_memory_access(s);
    let rbp_off = if has_indirect {
        RBP_OFFSET_NONE
    } else {
        parse_rbp_offset(s)
    };
    // Pre-scan register references for O(1) checks in eliminate_unused_callee_saves
    let reg_refs = scan_register_refs(sb);
    LineInfo {
        kind: LineKind::Other { dest_reg },
        ext_kind: ext,
        trim_start: ts,
        has_indirect_mem: has_indirect,
        rbp_offset: rbp_off,
        reg_refs,
        pinned: false,
    }
}

/// Fast check for self-move: `movq %REG, %REG` where both register names match.
/// Works on the raw bytes after trimming. The caller ensures sb starts with "movq "
/// and has length >= 6.
#[inline]
pub(super) fn is_self_move_fast(sb: &[u8]) -> bool {
    // sb = "movq %REG, %REG" - find the comma
    let len = sb.len();
    // The source starts at byte 5 (after "movq ")
    if len < 10 || sb[5] != b'%' {
        return false;
    }
    // Find comma
    let mut comma = 6;
    while comma < len {
        if sb[comma] == b',' {
            break;
        }
        comma += 1;
    }
    if comma >= len {
        return false;
    }
    // Source = sb[5..comma], skip whitespace after comma
    let src = &sb[5..comma];
    let mut dst_start = comma + 1;
    while dst_start < len && sb[dst_start] == b' ' {
        dst_start += 1;
    }
    let dst = &sb[dst_start..];
    // Trim trailing whitespace from dst
    let mut dst_end = dst.len();
    while dst_end > 0 && dst[dst_end - 1] == b' ' {
        dst_end -= 1;
    }
    let dst = &dst[..dst_end];
    src == dst && src.len() >= 2 && src[0] == b'%'
}

/// Classify mov-family instructions for extension elimination.
/// Called only when the line starts with "mov".
#[inline]
pub(super) fn classify_mov_ext(s: &str, sb: &[u8]) -> ExtKind {
    let len = sb.len();
    // Check specific extension patterns that the combined_local_pass cares about.
    // Note: movslq %eax, %rax serves dual roles: it IS a consumer (can be eliminated
    // after another movslq to %rax) and is also a producer for cltq elimination.
    // We classify it as ProducerMovslqToRax so cltq elimination works, and add
    // MovslqEaxRax to the consumer matching so it can also be eliminated.
    if s == "movzbq %al, %rax" {
        return ExtKind::MovzbqAlRax;
    }
    if s == "movzwq %ax, %rax" {
        return ExtKind::MovzwqAxRax;
    }
    if s == "movsbq %al, %rax" {
        return ExtKind::MovsbqAlRax;
    }
    if s == "movslq %eax, %rax" {
        return ExtKind::MovslqEaxRax;
    }
    if s == "movl %eax, %eax" {
        return ExtKind::MovlEaxEax;
    }

    // Producers: movslq ... %rax
    if len >= 7 && sb[3] == b's' && sb[4] == b'l' && sb[5] == b'q' && sb[6] == b' ' {
        // movslq - check if destination is %rax
        if s.ends_with("%rax") {
            return ExtKind::ProducerMovslqToRax;
        }
    }

    // Producers: movq $const, %rax
    if len >= 6 && sb[3] == b'q' && sb[4] == b' ' && sb[5] == b'$' && s.ends_with("%rax") {
        return ExtKind::ProducerMovqConstRax;
    }

    // Producers: movq %REG, %rax (register-to-rax copy, REG != rax)
    // Used for fusion: `movq %REG, %rax; movl %eax, %eax` -> `movl %REGd, %eax`
    if len >= 6 && sb[3] == b'q' && sb[4] == b' ' && sb[5] == b'%' && s.ends_with(", %rax") {
        // Extract source register and verify it's not %rax itself
        let src = &s[5..s.len() - 6]; // between "movq " and ", %rax"
        if src.starts_with('%') && src != "%rax" && !src.contains('(') {
            return ExtKind::ProducerMovqRegToRax;
        }
    }

    // Producers: movzbq ... %rax
    if s.starts_with("movzbq ") && s.ends_with("%rax") {
        return ExtKind::ProducerMovzbqToRax;
    }
    if s.starts_with("movzwq ") && s.ends_with("%rax") {
        return ExtKind::ProducerMovzwqToRax;
    }
    if s.starts_with("movsbq ") && s.ends_with("%rax") {
        return ExtKind::ProducerMovsbqToRax;
    }

    // Producers: movl ... %eax
    if len >= 6 && sb[3] == b'l' && sb[4] == b' ' && s.ends_with("%eax") {
        return ExtKind::ProducerMovlToEax;
    }

    // Producers: movzbl ... %eax
    if s.starts_with("movzbl ") && s.ends_with("%eax") {
        return ExtKind::ProducerMovzbToEax;
    }
    // movzbq ... %rax is already handled above as ProducerMovzbqToRax
    // which also counts as ProducerMovzbToEax for movl %eax,%eax elimination

    // Producers: movzwl ... %eax
    if s.starts_with("movzwl ") && s.ends_with("%eax") {
        return ExtKind::ProducerMovzwToEax;
    }
    // movzwq ... %rax is already handled above

    ExtKind::None
}

/// Classify arithmetic instructions that produce 32-bit results for extension elimination.
#[inline]
pub(super) fn classify_arith_ext(s: &str, _sb: &[u8], first: u8) -> ExtKind {
    match first {
        b'a' => {
            if s.starts_with("addl ") || s.starts_with("andl ") {
                ExtKind::ProducerArith32
            } else {
                ExtKind::None
            }
        }
        b's' => {
            if s.starts_with("subl ") || s.starts_with("shll ") || s.starts_with("shrl ") {
                ExtKind::ProducerArith32
            } else {
                ExtKind::None
            }
        }
        b'i' => {
            if s.starts_with("imull ") {
                ExtKind::ProducerArith32
            } else if s == "idivl %ecx" {
                ExtKind::ProducerDiv32
            } else {
                ExtKind::None
            }
        }
        b'o' => {
            if s.starts_with("orl ") {
                ExtKind::ProducerArith32
            } else {
                ExtKind::None
            }
        }
        b'x' => {
            if s.starts_with("xorl ") {
                ExtKind::ProducerArith32
            } else {
                ExtKind::None
            }
        }
        b'd' => {
            if s == "divl %ecx" {
                ExtKind::ProducerDiv32
            } else {
                ExtKind::None
            }
        }
        _ => ExtKind::None,
    }
}

/// Compute byte offset to first non-space character.
#[inline]
pub(super) fn compute_trim_offset(b: &[u8]) -> usize {
    let mut i = 0;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    i
}

/// Fast extraction of the destination register family from a generic instruction.
/// Handles the AT&T syntax convention where the last operand is the destination.
/// Also handles implicit writes (cltq, cqto, div, mul, etc.).
#[inline]
pub(super) fn parse_dest_reg_fast(s: &str) -> RegId {
    let b = s.as_bytes();
    // Implicit rax writers
    if b.len() >= 4 && b[0] == b'c' && (s == "cltq" || s == "cqto" || s == "cdq" || s == "cqo") {
        return 0; // rax family
    }
    // Single-operand div/idiv/mul implicitly write rax:rdx.
    // Note: imul is not listed because the codegen only emits two/three-operand
    // forms (imulq %rcx, %rax / imulq $imm, %rax, %rax) which write to the
    // explicit destination, handled by the comma-based dest extraction below.
    if b.len() >= 3
        && (b[0] == b'd' || b[0] == b'i' || b[0] == b'm')
        && (s.starts_with("div") || s.starts_with("idiv") || s.starts_with("mul"))
    {
        return 0; // rax family (also rdx, but we track rax as primary)
    }
    // For AT&T syntax the final *top-level* operand is the destination.  A
    // raw last-comma search is unsound for SIB memory operands such as
    // `movb $0, (%rdx,%r8)`: it mistakes `%r8)` for a destination register.
    // That corrupts dead-register analysis after a LEA→SIB rewrite and can
    // remove the live index producer.  Ignore commas nested in parentheses.
    if let Some(comma_pos) = last_top_level_comma(b) {
        let after_comma = &s[comma_pos + 1..];
        let trimmed = after_comma.trim();
        if !trimmed.starts_with('%') {
            return REG_NONE;
        }
        return register_family_fast(trimmed);
    }
    // Single-operand instructions that modify their operand in-place:
    //   inc, dec, not, neg    (arithmetic/logic)
    //   bswapl, bswapq        (byte swap)
    //   shr, shl, sar, ror, rol (shift/rotate by 1 with no immediate)
    //   popcntl, popcntq      (popcount, though these usually have 2 operands with comma)
    if b.len() >= 4 {
        let is_single_op_modifier = (b[0] == b'i' || b[0] == b'd' || b[0] == b'n')
            && (s.starts_with("inc")
                || s.starts_with("dec")
                || s.starts_with("not")
                || s.starts_with("neg"))
            || b[0] == b'b' && (s.starts_with("bswapl") || s.starts_with("bswapq"))
            || b[0] == b's'
                && (s.starts_with("shr") || s.starts_with("shl") || s.starts_with("sar"))
            || b[0] == b'r'
                && (s.starts_with("ror")
                    || s.starts_with("rol")
                    || s.starts_with("rcr")
                    || s.starts_with("rcl"));
        if is_single_op_modifier {
            if let Some(space_pos) = s.find(' ') {
                let operand = s[space_pos + 1..].trim();
                return register_family(operand).unwrap_or(REG_NONE);
            }
        }
    }
    REG_NONE
}

/// Find the last occurrence of byte `needle` in `haystack`.
#[inline]
pub(super) fn memrchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    let mut i = haystack.len();
    while i > 0 {
        i -= 1;
        if haystack[i] == needle {
            return Some(i);
        }
    }
    None
}

/// Find the last comma that separates operands rather than SIB address fields.
/// x86 AT&T memory operands contain commas inside balanced parentheses, so a
/// conventional `memrchr(',')` cannot be used to determine an instruction's
/// destination operand.
#[inline]
pub(super) fn last_top_level_comma(bytes: &[u8]) -> Option<usize> {
    let mut depth = 0u32;
    let mut last = None;
    for (idx, &byte) in bytes.iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => last = Some(idx),
            _ => {}
        }
    }
    last
}

/// Fast i32 parse for stack offsets like "-8", "-24", "0", etc.
/// Falls back to 0 on unparseable inputs (should not happen with valid asm).
#[inline]
pub(super) fn fast_parse_i32(s: &str) -> i32 {
    let b = s.as_bytes();
    if b.is_empty() {
        return 0;
    }
    let (neg, start) = if b[0] == b'-' { (true, 1) } else { (false, 0) };
    let mut v: i32 = 0;
    for &c in &b[start..] {
        if c.is_ascii_digit() {
            v = v.wrapping_mul(10).wrapping_add((c - b'0') as i32);
        } else {
            break;
        }
    }
    if neg {
        // wrapping_neg: the one edge case is a genuine INT32_MIN
        // operand (e.g. `addq $-2147483648, %rax` from kernel constant
        // folding), whose magnitude overflows i32; wrapping_neg maps it
        // to exactly INT32_MIN, which is the correct parsed value.
        v.wrapping_neg()
    } else {
        v
    }
}

// ── Existing string-level parsers (used once during classify and for mutations) ──

/// Strip a `mov*` prefix from an instruction, returning the remainder and size.
/// Handles movq, movl, movw, movb, and optionally movslq (for load parsing).
pub(super) fn strip_mov_prefix(s: &str, allow_slq: bool) -> Option<(&str, MoveSize)> {
    if let Some(r) = s.strip_prefix("movq ") {
        Some((r, MoveSize::Q))
    } else if let Some(r) = s.strip_prefix("movl ") {
        Some((r, MoveSize::L))
    } else if let Some(r) = s.strip_prefix("movw ") {
        Some((r, MoveSize::W))
    } else if let Some(r) = s.strip_prefix("movb ") {
        Some((r, MoveSize::B))
    } else if allow_slq {
        s.strip_prefix("movslq ").map(|r| (r, MoveSize::SLQ))
    } else {
        None
    }
}

/// Parse a scalar-FP / SSE stack move: `movsd`, `movss`, `movd` (or AVX
/// `vmovsd`/`vmovss`) between an XMM register and a frame slot. Returns
/// (is_store, offset_str, size). The string-op and reg-reg forms are
/// rejected by construction (the slot operand, not `%ds:/%es:` or an XMM,
/// must be one of the two ends).
pub(super) fn parse_fp_slot_move(s: &str) -> Option<(bool, &str, MoveSize)> {
    let body = if let Some(r) = s.strip_prefix("vmovsd ") {
        (r, MoveSize::SD)
    } else if let Some(r) = s.strip_prefix("vmovss ") {
        (r, MoveSize::SS)
    } else if let Some(r) = s.strip_prefix("movsd ") {
        (r, MoveSize::SD)
    } else if let Some(r) = s.strip_prefix("movss ") {
        (r, MoveSize::SS)
    } else if let Some(r) = s.strip_prefix("movd ") {
        (r, MoveSize::L)
    } else if let Some(r) = s.strip_prefix("vmovd ") {
        (r, MoveSize::L)
    } else {
        return None;
    };
    let (src, dst) = body.0.split_once(',')?;
    let src = src.trim();
    let dst = dst.trim();
    let store = src.starts_with('%') && (dst.ends_with("(%rbp)") || dst.ends_with("(%rsp)"));
    let load = dst.starts_with('%') && (src.ends_with("(%rbp)") || src.ends_with("(%rsp)"));
    if !store && !load {
        return None;
    }
    let mem = if store { dst } else { src };
    if mem.ends_with("(%rbp)") {
        Some((store, &mem[..mem.len() - 6], body.1))
    } else if mem.ends_with("(%rsp)") {
        Some((store, &mem[..mem.len() - 6], body.1))
    } else {
        None
    }
}

/// True when `reg_id` is an XMM family id (24..39), which cannot be
/// represented in the GP reg_refs bitmask and must not be handled by the
/// GP-oriented StoreRbp/LoadRbp consumers.
pub(super) fn is_xmm_family(reg_id: RegId) -> bool {
    (24..=39).contains(&reg_id)
}

/// Parse `movX %reg, offset(%rbp)` (store to rbp-relative slot).
/// Returns (register_str, offset_str, size).
pub(super) fn parse_store_to_rbp_str(s: &str) -> Option<(&str, &str, MoveSize)> {
    let (rest, size) = strip_mov_prefix(s, false)?;

    let (src, dst) = rest.split_once(',')?;
    let src = src.trim();
    let dst = dst.trim();

    if !src.starts_with('%') {
        return None;
    }
    // Accept both (%rbp) and (%rsp) for frame pointer / frame-pointer-less modes
    if dst.ends_with("(%rbp)") {
        let offset = &dst[..dst.len() - 6];
        return Some((src, offset, size));
    }
    if dst.ends_with("(%rsp)") {
        let offset = &dst[..dst.len() - 6];
        return Some((src, offset, size));
    }
    None
}

/// Parse `movX offset(%rbp), %reg` or `movX offset(%rsp), %reg`
/// or `movslq offset(%rbp), %reg` (load from stack frame).
/// Returns (offset_str, register_str, size).
pub(super) fn parse_load_from_rbp_str(s: &str) -> Option<(&str, &str, MoveSize)> {
    let (rest, size) = strip_mov_prefix(s, true)?;

    let (src, dst) = rest.split_once(',')?;
    let src = src.trim();
    let dst = dst.trim();

    if !dst.starts_with('%') {
        return None;
    }

    // Accept both (%rbp) and (%rsp) for frame pointer / frame-pointer-less modes
    if src.ends_with("(%rbp)") {
        let offset = &src[..src.len() - 6];
        return Some((offset, dst, size));
    }
    if src.ends_with("(%rsp)") {
        let offset = &src[..src.len() - 6];
        return Some((offset, dst, size));
    }
    None
}

// Re-export the shared LineStore: zero-allocation line storage that keeps
// one contiguous String plus (start, len) byte offsets per line.
// See backend/peephole_common.rs for the implementation.
pub(super) use crate::backend::peephole_common::LineStore;

// ── NOP / replace helpers ────────────────────────────────────────────────────

#[inline]
pub(super) fn mark_nop(info: &mut LineInfo) {
    if info.pinned {
        return;
    } // Never remove pinned instructions
    *info = LineInfo {
        kind: LineKind::Nop,
        ext_kind: ExtKind::None,
        trim_start: 0,
        has_indirect_mem: false,
        rbp_offset: RBP_OFFSET_NONE,
        reg_refs: 0,
        pinned: false,
    };
}

/// Replace a line's text and re-classify it.
#[inline]
pub(super) fn replace_line(
    store: &mut LineStore,
    info: &mut LineInfo,
    idx: usize,
    new_text: String,
) {
    store.replace(idx, new_text);
    *info = classify_line(store.get(idx));
}

/// Find the next non-NOP line at or after `start`, returning its index.
/// Returns `len` (infos.len()) if no non-NOP line is found before `limit`.
#[inline]
pub(super) fn next_non_nop(infos: &[LineInfo], start: usize, limit: usize) -> usize {
    let mut j = start;
    while j < limit && infos[j].is_nop() {
        j += 1;
    }
    j
}

/// Collect up to `N` non-NOP line indices starting after `start`.
/// Returns the number of indices found (may be < N if we hit the end).
#[inline]
pub(super) fn collect_non_nop_indices<const N: usize>(
    infos: &[LineInfo],
    start: usize,
    limit: usize,
    out: &mut [usize; N],
) -> usize {
    let mut count = 0;
    let mut j = start + 1;
    while j < limit && count < N {
        if !infos[j].is_nop() {
            out[count] = j;
            count += 1;
        }
        j += 1;
    }
    count
}

/// Check if two byte ranges `[a, a+a_size)` and `[b, b+b_size)` overlap.
#[inline]
pub(super) fn ranges_overlap(a: i32, a_size: i32, b: i32, b_size: i32) -> bool {
    a < b + b_size && b < a + a_size
}

// ── Helper functions ─────────────────────────────────────────────────────────

pub(super) fn has_indirect_memory_access(s: &str) -> bool {
    // rep movsb/movsd/stosb/stosd etc. use implicit memory operands through
    // %rdi (dest) and %rsi (source) that aren't visible in the instruction text.
    // They also modify %rdi, %rsi, and %rcx. Treat them as indirect memory access
    // to ensure store forwarding and dead store elimination are conservative.
    let trimmed = s.trim_start();
    if trimmed.starts_with("rep ") || trimmed.starts_with("rep\t") {
        return true;
    }

    // Multi-instruction lines (containing ';' statement separator) commonly come
    // from inline asm templates. The peephole optimizer cannot safely analyze
    // which registers are clobbered by the first instruction(s) in such lines
    // (e.g., "1: rdmsr ; xor %esi,%esi" where rdmsr implicitly clobbers eax/edx
    // but the parser only sees the xor destination). Treat these conservatively
    // to prevent incorrect store forwarding across inline asm boundaries.
    if s.contains(';') {
        return true;
    }

    // Privileged/special x86 instructions that implicitly clobber multiple
    // registers not visible in the instruction text:
    //   rdmsr   -> reads ecx, writes eax:edx
    //   wrmsr   -> reads ecx:eax:edx (all inputs, but may affect state)
    //   cpuid   -> reads eax, writes eax:ebx:ecx:edx
    //   rdtsc   -> writes eax:edx
    //   rdtscp  -> writes eax:edx:ecx
    //   xgetbv  -> reads ecx, writes eax:edx
    //   syscall -> writes rcx (return RIP) and r11 (saved RFLAGS)
    //   sysenter -> modifies rsp, writes rip (OS-dependent register effects)
    //   int     -> software interrupt; clobbers varies by handler (treat conservatively)
    //   iret    -> restores rip, cs, rflags, rsp, ss from stack
    // Treat these as barriers for store forwarding since parse_dest_reg_fast
    // cannot detect their implicit register writes.
    if trimmed.starts_with("rdmsr")
        || trimmed.starts_with("wrmsr")
        || trimmed.starts_with("cpuid")
        || trimmed.starts_with("rdtsc")
        || trimmed.starts_with("xgetbv")
        || trimmed.starts_with("syscall")
        || trimmed.starts_with("sysenter")
        || trimmed.starts_with("int ")
        || trimmed.starts_with("int\t")
        || trimmed.starts_with("iret")
    {
        return true;
    }

    // Also handle numeric-label-prefixed instructions (common in inline asm).
    // Pattern: "N: instruction" where N is a digit. The instruction after the
    // label may have implicit clobbers that parse_dest_reg_fast can't detect.
    if !trimmed.is_empty() {
        let fb = trimmed.as_bytes()[0];
        if fb.is_ascii_digit() && trimmed.contains(": ") {
            return true;
        }
    }

    // Look for patterns like "(%r" where the register is not rbp or rsp
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'(' && i + 2 < len && bytes[i + 1] == b'%' {
            // Found "(%" - check if it's %rbp or %rsp (which are not indirect aliasing)
            let rest = &s[i + 2..];
            let is_frame_base = rest.starts_with("rbp") || rest.starts_with("rsp");
            if rest.starts_with("rip") {
                // RIP-relative: a fixed global/rodata address, not a stack slot.
            } else if !is_frame_base {
                return true;
            } else {
                // (%rsp,...) / (%rbp,...): INDEXED addressing computes the slot
                // at runtime, so no text-level offset can describe it. Treat it
                // as indirect; a missed marker here let dead-store elimination
                // drop stores read through an index.
                let after_base = &rest[3..];
                if !after_base.starts_with(')') {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Vector-move frame access extent: `Some(bytes)` when the line is an
/// SSE/AVX PACKED vector load/store with a frame-base memory operand
/// (`(%rbp` / `(%rsp`), in which case the access spans
/// `[rbp_offset, rbp_offset + bytes)`. `None` for every other line.
///
/// The scalar slot tracker models one offset per line with an implicit
/// 8-byte point (`parse_rbp_offset`), which is exact for scalar integer
/// moves and for the scalar-FP forms (`movsd` = 8 bytes, `movss` = 4,
/// SSE `movq` = 8) — the FP memory-operand folds DEPEND on those lines
/// staying precisely modelled, so they must never become opaque. The packed
/// 16/32-byte family is where the point model goes anti-conservative: a
/// `movdqu 96(%rsp), %xmm0` reads `[96, 112)`, but the cached offset 96
/// only credits reads of `[96, 104)` — the high half at 104 is invisible.
/// `eliminate_dead_stores` elided an i128 parameter's high-half home store
/// on exactly that miss (its only reader inside the window was the `movdqu`
/// copy, and the wide condition test then read stale frame bytes). This is
/// the same range-vs-point disease store_forwarding fixed with its
/// 16-byte invalidate ("16 bytes covers every x86 access width") — here as
/// a returned extent so the caller can overlap-check precisely instead of
/// going opaque (which would kill the FP folds).
pub(super) fn vector_frame_move_extent(s: &str) -> Option<i32> {
    // Frame-base memory operand present at all? Cheap text gate first.
    if !s.contains("(%rbp") && !s.contains("(%rsp") {
        return None;
    }
    // Optional AVX `v` prefix, then the SSE packed family. Scalar forms
    // (`movsd`, `movss`) and the SSE integer `movq`/`movd` with xmm regs are
    // point-accurate (4/8 bytes) and deliberately absent — the existing
    // single-offset model already covers them. String-op spellings that
    // collide (`movsq`, `movsl`, `movsb`, `movsw`) use %rsi/%rdi bases,
    // never the frame base, so the gate above already excludes them.
    const PACKED_MNEMONICS: &[&str] = &[
        "movdqu", "movdqa", "movups", "movaps", "movupd", "movapd", "lddqu",
    ];
    let body = s.strip_prefix('v').unwrap_or(s);
    for m in PACKED_MNEMONICS {
        if body.starts_with(m) {
            // %ymm forms move 32 bytes; %xmm (and %st implicit) 16.
            let extent = if s.contains("%ymm") { 32 } else { 16 };
            return Some(extent);
        }
    }
    None
}

/// Pre-parse an `Other` line for a %rbp offset reference.
/// Looks for patterns like `N(%rbp)` and returns the offset N.
/// Returns `RBP_OFFSET_NONE` if no rbp reference or multiple references found.
/// This is called once during classify_line and cached in LineInfo.rbp_offset,
/// eliminating the expensive `str::contains` in eliminate_dead_stores.
pub(super) fn parse_rbp_offset(s: &str) -> i32 {
    let bytes = s.as_bytes();
    let len = bytes.len();
    // Search for "(%rbp)" or "(%rsp)" pattern (both used for stack frame refs)
    let mut found_offset = RBP_OFFSET_NONE;
    let mut i = 0;
    while i + 5 < len {
        if bytes[i] == b'('
            && bytes[i + 1] == b'%'
            && bytes[i + 2] == b'r'
            && ((bytes[i + 3] == b'b' && bytes[i + 4] == b'p')
                || (bytes[i + 3] == b's' && bytes[i + 4] == b'p'))
            && bytes[i + 5] == b')'
        {
            // Found "(%rbp)" at position i. Parse the offset before '('.
            // The offset is the integer immediately before the '(' character.
            let offset = if i == 0 {
                0 // bare (%rbp) with no offset
            } else {
                // Scan backwards for digits/minus sign
                let end = i;
                let mut start = end;
                while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'-') {
                    start -= 1;
                }
                if start == end {
                    0 // no numeric prefix, bare (%rbp) after space/comma
                } else {
                    fast_parse_i32(&s[start..end])
                }
            };
            if found_offset == RBP_OFFSET_NONE {
                found_offset = offset;
            } else if found_offset != offset {
                // Multiple different rbp offsets - can't pre-classify
                return RBP_OFFSET_NONE;
            }
            i += 6;
            continue;
        }
        i += 1;
    }
    found_offset
}

/// Check if a line is a conditional jump instruction.
/// Uses byte-level dispatch on the second character to avoid 18 `starts_with` calls.
pub(super) fn is_conditional_jump(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 3 || b[0] != b'j' {
        return false;
    }
    // Dispatch on second byte to narrow candidates quickly
    match b[1] {
        b'e' => b[2] == b' ',                                                   // je
        b'l' => b[2] == b' ' || (b.len() >= 4 && b[2] == b'e' && b[3] == b' '), // jl, jle
        b'g' => b[2] == b' ' || (b.len() >= 4 && b[2] == b'e' && b[3] == b' '), // jg, jge
        b'b' => b[2] == b' ' || (b.len() >= 4 && b[2] == b'e' && b[3] == b' '), // jb, jbe
        b'a' => b[2] == b' ' || (b.len() >= 4 && b[2] == b'e' && b[3] == b' '), // ja, jae
        // GAS aliases: `jc` = jb, `jnc` = jae. The kernel's bit-test idiom
        // (`bt $bit,%r; jc .Lx`) relies on these; missing them here left the
        // lines unclassified as CondJmp, the identical_blocks predecessor
        // analysis saw no incoming edge, and the pass merged live blocks
        // away (kernel/events/core.c is_sb_event lost its return-false block,
        // dangling `jnc` relocation -> sym index 0 -> link failure).
        b'c' => b[2] == b' ' || (b.len() >= 4 && b[2] == b'n' && b[3] == b' '), // jc, jnc
        b's' => b[2] == b' ',                                                   // js
        b'o' => b[2] == b' ',                                                   // jo
        b'p' => b[2] == b' ',                                                   // jp
        b'z' => b[2] == b' ',                                                   // jz
        b'n' => {
            // jne, jns, jno, jnp, jnz, jnc (jnc = jae alias; must live here —
            // the `n` arm runs before the `c` arm in this dispatch)
            if b.len() >= 4 {
                match b[2] {
                    b'e' | b's' | b'o' | b'p' | b'z' | b'c' => b[3] == b' ',
                    _ => false,
                }
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Parse `movq %src, %dst` and return %dst if %src matches expected_src.
pub(super) fn parse_reg_to_reg_move<'a>(line: &'a str, expected_src: &str) -> Option<&'a str> {
    for prefix in &["movq ", "movl "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            if let Some((src, dst)) = rest.split_once(',') {
                let src = src.trim();
                let dst = dst.trim();
                if src == expected_src && dst.starts_with('%') {
                    return Some(dst);
                }
            }
        }
    }
    None
}

/// Check if an instruction writes to a specific register as its destination.
pub(super) fn instruction_writes_to(inst: &str, reg: &str) -> bool {
    if let Some((_op, operands)) = inst.split_once(' ') {
        if let Some((_src, dst)) = operands.rsplit_once(',') {
            let dst = dst.trim();
            if dst == reg || register_overlaps(dst, reg) {
                return true;
            }
        }
    }
    false
}

/// Check if an instruction can have its destination register replaced safely.
pub(super) fn can_redirect_instruction(inst: &str) -> bool {
    if inst.starts_with("movabsq ") {
        return false;
    }
    if inst.starts_with(".") || inst.ends_with(":") {
        return false;
    }
    true
}

/// Replace the destination register in an instruction.
pub(super) fn replace_dest_register(inst: &str, old_reg: &str, new_reg: &str) -> Option<String> {
    if !old_reg.starts_with("%r") || !new_reg.starts_with("%r") {
        return None;
    }

    // Handle `xorq %reg, %reg` or `xorl %ereg, %ereg` (zero idiom)
    if let Some(rest) = inst.strip_prefix("xorq ") {
        if let Some((src, dst)) = rest.split_once(',') {
            let src = src.trim();
            let dst = dst.trim();
            if src == old_reg && dst == old_reg {
                return Some(format!("xorq {}, {}", new_reg, new_reg));
            }
        }
    }
    if let Some(rest) = inst.strip_prefix("xorl ") {
        if let Some((src, dst)) = rest.split_once(',') {
            let src = src.trim();
            let dst = dst.trim();
            // Check if this is a zero idiom (same 32-bit reg) and the old_reg matches
            if src == dst {
                // Map old_reg (64-bit) to its 32-bit name to check if it matches
                let old_32 = match old_reg {
                    "%rax" => "%eax",
                    "%rbx" => "%ebx",
                    "%rcx" => "%ecx",
                    "%rdx" => "%edx",
                    "%rsi" => "%esi",
                    "%rdi" => "%edi",
                    "%rsp" => "%esp",
                    "%rbp" => "%ebp",
                    "%r8" => "%r8d",
                    "%r9" => "%r9d",
                    "%r10" => "%r10d",
                    "%r11" => "%r11d",
                    "%r12" => "%r12d",
                    "%r13" => "%r13d",
                    "%r14" => "%r14d",
                    "%r15" => "%r15d",
                    _ => "",
                };
                if src == old_32 {
                    let new_32 = match new_reg {
                        "%rax" => "%eax",
                        "%rbx" => "%ebx",
                        "%rcx" => "%ecx",
                        "%rdx" => "%edx",
                        "%rsi" => "%esi",
                        "%rdi" => "%edi",
                        "%rsp" => "%esp",
                        "%rbp" => "%ebp",
                        "%r8" => "%r8d",
                        "%r9" => "%r9d",
                        "%r10" => "%r10d",
                        "%r11" => "%r11d",
                        "%r12" => "%r12d",
                        "%r13" => "%r13d",
                        "%r14" => "%r14d",
                        "%r15" => "%r15d",
                        _ => new_reg,
                    };
                    return Some(format!("xorl {}, {}", new_32, new_32));
                }
            }
        }
    }

    for prefix in &["movq ", "movslq ", "leaq ", "movzbq "] {
        if let Some(rest) = inst.strip_prefix(prefix) {
            if let Some((src, dst)) = rest.rsplit_once(',') {
                let src = src.trim();
                let dst = dst.trim();
                if dst == old_reg && !src.contains(old_reg) {
                    return Some(format!("{}{}, {}", prefix, src, new_reg));
                }
            }
        }
    }

    None
}

/// Parse a setCC instruction and return the condition code string.
pub(super) fn parse_setcc(s: &str) -> Option<&str> {
    if !s.starts_with("set") {
        return None;
    }
    let rest = &s[3..];
    let space_idx = rest.find(' ')?;
    let cc = &rest[..space_idx];
    match cc {
        "e" | "ne" | "l" | "le" | "g" | "ge" | "b" | "be" | "a" | "ae" | "c" | "nc" | "s"
        | "ns" | "o" | "no" | "p" | "np" | "z" | "nz" => Some(cc),
        _ => None,
    }
}

/// Invert a condition code (e.g., "e" -> "ne", "l" -> "ge")
pub(super) fn invert_cc(cc: &str) -> &str {
    match cc {
        "e" | "z" => "ne",
        "ne" | "nz" => "e",
        "l" => "ge",
        "ge" => "l",
        "le" => "g",
        "g" => "le",
        "b" | "c" => "ae",
        "ae" | "nc" => "b",
        "be" => "a",
        "a" => "be",
        "s" => "ns",
        "ns" => "s",
        "o" => "no",
        "no" => "o",
        "p" => "np",
        "np" => "p",
        _ => cc,
    }
}

/// Check if two register names overlap (e.g., %eax overlaps with %rax).
pub(super) fn register_overlaps(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let a_family = register_family(a);
    let b_family = register_family(b);
    a_family.is_some() && a_family == b_family
}

/// Get the register family (0-15) for an x86 register name.
pub(super) fn register_family(reg: &str) -> Option<u8> {
    let id = register_family_fast(reg);
    if id == REG_NONE {
        None
    } else {
        Some(id)
    }
}

/// Fast register family lookup using byte-level dispatch.
/// Returns REG_NONE if the register is not recognized.
///
/// Handles all x86-64 GP register aliases:
///   64-bit: %rax..%rdi, %r8..%r15
///   32-bit: %eax..%edi, %r8d..%r15d
///   16-bit: %ax..%di,   %r8w..%r15w
///    8-bit: %al..%dil,  %r8b..%r15b, %ah/%bh/%ch/%dh
/// Also handles MMX (%mm0..%mm7) and SSE (%xmm0..%xmm15) registers:
///   GP families: 0-15, MMX families: 16-23, XMM families: 24-39.
#[inline]
pub(super) fn register_family_fast(reg: &str) -> RegId {
    let b = reg.as_bytes();
    let len = b.len();
    if len < 3 || b[0] != b'%' {
        return REG_NONE;
    }
    // Dispatch on the prefix character after '%' to find the family.
    // 64-bit (%r..) and 32-bit (%e..) prefixes use a shared lookup on b[2..],
    // since %rax/%eax, %rcx/%ecx, etc. share the same suffix→family mapping.
    match b[1] {
        b'r' | b'e' => {
            if len < 4 {
                // len==3: only %r8, %r9 are valid
                return if b[1] == b'r' {
                    reg_digit_to_id(b[2])
                } else {
                    REG_NONE
                };
            }
            // For len>=4, map (b[2], b[3]) to family id.
            // This covers %rax/%eax, %rcx/%ecx, etc. and %r10..%r15.
            match (b[2], b[3]) {
                (b'a', b'x') => 0, // %rax / %eax
                (b'c', b'x') => 1, // %rcx / %ecx
                (b'd', b'x') => 2, // %rdx / %edx
                (b'd', b'i') => 7, // %rdi / %edi
                (b'b', b'x') => 3, // %rbx / %ebx
                (b'b', b'p') => 5, // %rbp / %ebp
                (b's', b'p') => 4, // %rsp / %esp
                (b's', b'i') => 6, // %rsi / %esi
                (b'8', _) => 8,    // %r8d / %r8w / %r8b
                (b'9', _) => 9,    // %r9d / %r9w / %r9b
                (b'1', b'0') => 10,
                (b'1', b'1') => 11,
                (b'1', b'2') => 12,
                (b'1', b'3') => 13,
                (b'1', b'4') => 14,
                (b'1', b'5') => 15,
                _ => REG_NONE,
            }
        }
        // 16-bit / 8-bit short forms: %ax, %al, %ah, %cx, %cl, etc.
        b'a' => {
            if matches!(b[2], b'x' | b'l' | b'h') {
                0
            } else {
                REG_NONE
            }
        }
        b'c' => {
            if matches!(b[2], b'x' | b'l' | b'h') {
                1
            } else {
                REG_NONE
            }
        }
        b'd' => match b[2] {
            b'i' => 7,
            b'x' | b'l' | b'h' => 2,
            _ => REG_NONE,
        },
        b'b' => match b[2] {
            b'p' => 5,
            b'x' | b'l' | b'h' => 3,
            _ => REG_NONE,
        },
        b's' => match b[2] {
            b'p' => 4,
            b'i' => 6,
            _ => REG_NONE,
        },
        // MMX registers: %mm0..%mm7 → families 16..23
        b'm' if len == 4 && b[2] == b'm' && b[3] >= b'0' && b[3] <= b'7' => 16 + (b[3] - b'0'),
        // XMM registers: %xmm0..%xmm15 → families 24..39
        b'x' if len >= 5 && b[2] == b'm' && b[3] == b'm' => {
            if len == 5 && b[4] >= b'0' && b[4] <= b'9' {
                24 + (b[4] - b'0')
            } else if len == 6 && b[4] == b'1' && b[5] >= b'0' && b[5] <= b'5' {
                34 + (b[5] - b'0') // xmm10..xmm15
            } else {
                REG_NONE
            }
        }
        _ => REG_NONE,
    }
}

/// Map a single ASCII digit byte to a register family id (8 or 9), or REG_NONE.
#[inline]
pub(super) fn reg_digit_to_id(digit: u8) -> RegId {
    match digit {
        b'8' => 8,
        b'9' => 9,
        _ => REG_NONE,
    }
}

/// Register name table indexed by [size][family_id].
/// Sizes: 0=Q/SLQ(64-bit), 1=L(32-bit), 2=W(16-bit), 3=B(8-bit).
pub(super) const REG_NAMES: [[&str; 16]; 4] = [
    // Q / SLQ (64-bit)
    [
        "%rax", "%rcx", "%rdx", "%rbx", "%rsp", "%rbp", "%rsi", "%rdi", "%r8", "%r9", "%r10",
        "%r11", "%r12", "%r13", "%r14", "%r15",
    ],
    // L (32-bit)
    [
        "%eax", "%ecx", "%edx", "%ebx", "%esp", "%ebp", "%esi", "%edi", "%r8d", "%r9d", "%r10d",
        "%r11d", "%r12d", "%r13d", "%r14d", "%r15d",
    ],
    // W (16-bit)
    [
        "%ax", "%cx", "%dx", "%bx", "%sp", "%bp", "%si", "%di", "%r8w", "%r9w", "%r10w", "%r11w",
        "%r12w", "%r13w", "%r14w", "%r15w",
    ],
    // B (8-bit)
    [
        "%al", "%cl", "%dl", "%bl", "%spl", "%bpl", "%sil", "%dil", "%r8b", "%r9b", "%r10b",
        "%r11b", "%r12b", "%r13b", "%r14b", "%r15b",
    ],
];

/// Scan assembly line bytes for register references and return a bitmask.
/// Bit N set means register family N is referenced somewhere in the line.
/// This is O(n) in line length and is called once per line during classify_line.
/// The bitmask eliminates O(n * patterns) `str::contains` calls in passes like
/// `eliminate_unused_callee_saves`.
#[inline]
pub(super) fn scan_register_refs(b: &[u8]) -> u16 {
    let mut refs: u16 = 0;
    let len = b.len();
    let mut i = 0;
    while i < len {
        if b[i] == b'%' && i + 2 < len {
            // Try to identify the register family from the bytes after '%'
            let fam = match b[i + 1] {
                b'r' => {
                    if i + 3 < len {
                        match (b[i + 2], b[i + 3]) {
                            (b'a', b'x') => Some(0u8),
                            (b'c', b'x') => Some(1),
                            (b'd', b'x') => Some(2),
                            (b'b', b'x') => Some(3),
                            (b's', b'p') => Some(4),
                            (b'b', b'p') => Some(5),
                            (b's', b'i') => Some(6),
                            (b'd', b'i') => Some(7),
                            (b'8', _) => Some(8),
                            (b'9', _) => Some(9),
                            (b'1', b'0') => Some(10),
                            (b'1', b'1') => Some(11),
                            (b'1', b'2') => Some(12),
                            (b'1', b'3') => Some(13),
                            (b'1', b'4') => Some(14),
                            (b'1', b'5') => Some(15),
                            _ => None,
                        }
                    } else {
                        // Short: %r8, %r9 (only i + 2 < len is guaranteed here)
                        match b[i + 2] {
                            b'8' => Some(8),
                            b'9' => Some(9),
                            _ => None,
                        }
                    }
                }
                b'e' => {
                    if i + 3 < len {
                        match (b[i + 2], b[i + 3]) {
                            (b'a', b'x') => Some(0),
                            (b'c', b'x') => Some(1),
                            (b'd', b'x') => Some(2),
                            (b'b', b'x') => Some(3),
                            (b's', b'p') => Some(4),
                            (b'b', b'p') => Some(5),
                            (b's', b'i') => Some(6),
                            (b'd', b'i') => Some(7),
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
                // 16-bit / 8-bit: %ax, %al, %ah, %cx, %cl, etc.
                b'a' => Some(0),
                b'c' => Some(1),
                b'd' => {
                    if i + 2 < len && b[i + 2] == b'i' {
                        Some(7)
                    } else {
                        Some(2)
                    }
                }
                b'b' => {
                    if i + 2 < len && b[i + 2] == b'p' {
                        Some(5)
                    } else {
                        Some(3)
                    }
                }
                b's' => {
                    if i + 2 < len {
                        match b[i + 2] {
                            b'p' => Some(4),
                            b'i' => Some(6),
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(id) = fam {
                refs |= 1u16 << id;
            }
            i += 2; // skip past '%X'
        } else {
            i += 1;
        }
    }
    // Architectural implicit operands (`cqto`, `idivq %r11`, `cpuid`, string
    // primitives, ...) are invisible to the textual scan above; every consumer
    // of `reg_refs` must see the same instruction as the CPU executes it.
    refs | implicit_reg_refs(b)
}

// ── Implicit register operands ───────────────────────────────────────────────

/// Register families an instruction reads or writes WITHOUT naming them in
/// its operand text.
///
/// `reg_refs` is the "does this line mention family F at all" oracle for
/// every text pass (copy coalescing, copy propagation, relay folding, dead
/// write elimination, loop trampolines, ...).  A purely textual scan misses
/// the architectural implicit operands, so `cqto` / `idivq %r11` looked like
/// lines that never touch `%rdx`, and `coalesce_register_copies` happily
/// renamed a parameter home `movq %rdx, %r8` back onto `%rdx` across a
/// division — the reload of the parameter (`movzbl %dl, ...`) then read the
/// remainder (stress lab `intexpr` seed 1, `-O1`).  `load_op_fuse` used to
/// add `RAX | RDX | RCX | string regs` privately; folding the knowledge into
/// the classifier makes every consumer see the same truth.
///
/// The table records the implicit READS and the implicit WRITES separately:
/// [`implicit_reg_refs`] unions both (the "mentions" oracle — a set bit only
/// makes a pass more conservative), while [`implicit_write_refs`] and
/// [`implicit_read_refs`] project the exact halves for callers that must not
/// conflate reading with writing (redefinition checks vs. observation
/// checks).
///
/// Prefixes (`lock`, `rep`, `repe`, `repz`, `repne`, `repnz`) are skipped so
/// the mnemonic that follows is classified; the `rep` family additionally
/// contributes `%rcx` (count) — and `rep ret` / `rep nop` (pause) contribute
/// nothing.
///
/// Mnemonics are matched EXACTLY, never by prefix: `divsd`/`mulsd`/`mulpd`
/// are SSE instructions with fully explicit operands and must not inherit
/// the integer `div`/`mul` accumulator pair (the local liveness model's
/// prefix match over-approximates this; the oracle is exact).  The emitter
/// today only produces a subset of the table, but the oracle is written
/// against the ISA, so the next new emitter site is covered for free.
#[inline]
pub(super) fn implicit_reg_refs(b: &[u8]) -> u16 {
    let (reads, writes) = classify_implicit_operands(b);
    reads | writes
}

/// Families an instruction implicitly READS without naming them in its
/// operand text.  An implicit read observes the register's current value:
/// acceptance-style checks ("this use observes the full 64-bit value") must
/// consult this projection, not [`implicit_reg_refs`], or a pure writer such
/// as `cqto` would be mistaken for a reader of the `%rax` family it merely
/// consumes as input... and conversely a redefinition check that used the
/// combined set would freeze coalescing across `cqto` for `%rax` copies that
/// are architecturally untouched.
#[inline]
pub(super) fn implicit_read_refs(b: &[u8]) -> u16 {
    classify_implicit_operands(b).0
}

/// Families an instruction implicitly WRITES without naming them in its
/// operand text.  This is the exact write half of [`implicit_reg_refs`] —
/// the counterpart of the classified destination for instructions whose
/// second (or only) destination is architectural: `cqto` → `%rdx`,
/// `idivq %r11` → `%rax:%rdx`, `rep stosq` → `%rdi:%rcx`, `syscall` →
/// `%rax:%rcx:%r11`, ...
#[inline]
pub(super) fn implicit_write_refs(b: &[u8]) -> u16 {
    classify_implicit_operands(b).1
}

/// One instruction's implicit (reads, writes) over the GP family numbering
/// (0 = %rax .. 15 = %r15), derived from its mnemonic.  Both halves are
/// EXACT for every instruction listed; unknown mnemonics yield (0, 0) —
/// callers that need a MAY model must add their own conservative fallback
/// (that is what `load_op_fuse::may_write_families` layers on top).
fn classify_implicit_operands(b: &[u8]) -> (u16, u16) {
    const RAX: u16 = 1 << 0;
    const RCX: u16 = 1 << 1;
    const RDX: u16 = 1 << 2;
    const RBX: u16 = 1 << 3;
    const RSP: u16 = 1 << 4;
    const RBP: u16 = 1 << 5;
    const RSI: u16 = 1 << 6;
    const RDI: u16 = 1 << 7;
    const R8: u16 = 1 << 8;
    const R9: u16 = 1 << 9;
    const R10: u16 = 1 << 10;
    const R11: u16 = 1 << 11;

    // Split "mnemonic [operands]" on the first blank.  Lines are already
    // trimmed on the left by the caller.
    fn mnemonic(b: &[u8]) -> (&[u8], &[u8]) {
        let mut i = 0;
        while i < b.len() && b[i] != b' ' && b[i] != b'\t' {
            i += 1;
        }
        let m = &b[..i];
        while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        (m, &b[i..])
    }

    let (mut m, mut rest) = mnemonic(b);
    if m.is_empty() || m[0] == b'.' || m[m.len() - 1] == b':' || m[0] == b'#' {
        // Labels, directives and comments name no instruction.
        return (0, 0);
    }
    let mut reads: u16 = 0;
    let mut writes: u16 = 0;
    // Prefixes.
    loop {
        match m {
            b"lock" => {}
            b"rep" | b"repe" | b"repz" | b"repne" | b"repnz" => {
                // `rep ret` / `rep nop` (pause) are branch-hint idioms, not
                // counted loops: they touch no register at all.
                let (m2, _) = mnemonic(rest);
                if m2 != b"ret" && m2 != b"retq" && m2 != b"nop" {
                    reads |= RCX; // count tested for zero
                    writes |= RCX; // count decremented
                }
            }
            _ => break,
        }
        let (m2, r2) = mnemonic(rest);
        if m2.is_empty() {
            return (reads, writes);
        }
        m = m2;
        rest = r2;
    }
    let has_comma = rest.contains(&b',');
    let table: (u16, u16) = match m {
        // Sign-extension of the accumulator in place.
        b"cbtw" | b"cwtl" | b"cltq" | b"cbw" | b"cwde" | b"cdqe" => (RAX, RAX),
        // Sign-extension into the data register: %rax is the INPUT.
        b"cwtd" | b"cltd" | b"cqto" | b"cwd" | b"cdq" | b"cqo" => (RAX, RDX),
        // Single-operand multiply/divide: rax:rdx implicit.  The byte forms
        // divide/multiply AX by r/m8 (quotient AL, remainder AH, product AX)
        // and never touch the %rdx family.
        b"divb" | b"idivb" | b"mulb" | b"imulb" => (RAX, RAX),
        b"div" | b"divw" | b"divl" | b"divq" | b"idiv" | b"idivw" | b"idivl" | b"idivq"
        | b"mul" | b"mulw" | b"mull" | b"mulq" => (RAX | RDX, RAX | RDX),
        // imul with a single operand is the widening form (rax:rdx); the
        // two/three-operand forms name every register they touch.
        b"imul" | b"imulw" | b"imull" | b"imulq" if !has_comma => (RAX, RAX | RDX),
        // BMI2 mulx reads %rdx implicitly; both destinations are explicit.
        b"mulx" | b"mulxl" | b"mulxq" => (RDX, 0),
        // String instructions: %rsi source, %rdi destination, %rax data.  The
        // pointer registers are post-incremented, i.e. also written.
        b"movsb" | b"movsw" | b"movsl" | b"movsq" | b"cmpsb" | b"cmpsw" | b"cmpsl" | b"cmpsq" => {
            (RSI | RDI, RSI | RDI)
        }
        b"stosb" | b"stosw" | b"stosl" | b"stosq" | b"scasb" | b"scasw" | b"scasl" | b"scasq" => {
            (RAX | RDI, RDI)
        }
        b"lodsb" | b"lodsw" | b"lodsl" | b"lodsq" => (RSI, RAX | RSI),
        b"insb" | b"insw" | b"insl" => (RDX, RDI),
        b"outsb" | b"outsw" | b"outsl" => (RDX | RSI, RSI),
        b"in" | b"inb" | b"inw" | b"inl" => (RDX, RAX),
        b"out" | b"outb" | b"outw" | b"outl" => (RAX | RDX, 0),
        // Serialising / system instructions with fixed register interfaces.
        b"cpuid" => (RAX, RAX | RBX | RCX | RDX),
        b"rdtsc" => (0, RAX | RDX),
        b"rdtscp" => (0, RAX | RCX | RDX),
        b"rdpmc" | b"rdmsr" | b"xgetbv" => (RCX, RAX | RDX),
        b"wrmsr" | b"xsetbv" => (RAX | RDX | RCX, 0),
        b"monitor" | b"mwait" => (RAX | RCX | RDX, 0),
        // The xsave family passes its feature mask in %edx:%eax; state goes
        // to/from memory, never to a GP register.
        b"xsave" | b"xsaveopt" | b"xsavec" | b"xsaves" | b"xrstor" | b"xrstors" | b"xsave64"
        | b"xsaveopt64" | b"xsavec64" | b"xsaves64" | b"xrstor64" | b"xrstors64" => (RAX | RDX, 0),
        // syscall: number/return in %rax, arguments in rdi/rsi/rdx/r10/r8/r9,
        // and the kernel clobbers %rcx/%r11.
        b"syscall" => (RAX | RDX | RSI | RDI | R8 | R9 | R10, RAX | RCX | R11),
        b"sysenter" | b"sysexit" | b"sysret" | b"sysretq" => (RAX | RCX | RDX, RAX | RCX | RDX),
        // Software interrupt: the handler's register contract is unknown
        // (i386 `int $0x80` passes ebx/ecx/edx/esi/edi/ebp and returns eax).
        b"int" => (
            RAX | RBX | RCX | RDX | RSI | RDI | RBP,
            RAX | RBX | RCX | RDX | RSI | RDI | RBP,
        ),
        // Atomics with an implicit accumulator: %rax holds the compare value
        // and is reloaded when the exchange fails.
        b"cmpxchg" | b"cmpxchgb" | b"cmpxchgw" | b"cmpxchgl" | b"cmpxchgq" => (RAX, RAX),
        // CMPXCHG8B/16B compares %edx:%eax (reloaded on failure) against
        // memory; %ecx:%ebx are the store payload, read-only.
        b"cmpxchg8b" | b"cmpxchg16b" => (RAX | RBX | RCX | RDX, RAX | RDX),
        // Misc legacy: table lookup, flags <-> %ah, counted loops.
        b"xlat" | b"xlatb" => (RBX, RAX),
        b"lahf" => (0, RAX),
        b"sahf" => (RAX, 0),
        b"loop" | b"loope" | b"loopz" | b"loopne" | b"loopnz" => (RCX, RCX),
        b"jrcxz" | b"jecxz" => (RCX, 0),
        // Frame instructions.
        b"enter" => (0, RSP | RBP),
        b"leave" | b"leaveq" => (RBP, RSP | RBP),
        _ => (0, 0),
    };
    (reads | table.0, writes | table.1)
}

/// Write an integer and the "(%rbp)" suffix into a buffer without using core::fmt.
/// This avoids the overhead of format_args!/write! which was measured at ~2.45%
/// of total compilation time for large files. Returns the number of bytes written.
#[inline]
pub(super) fn write_rbp_pattern(buf: &mut [u8; 24], offset: i32) -> usize {
    let mut pos = 0;
    // Write the integer part
    if offset < 0 {
        buf[pos] = b'-';
        pos += 1;
        // Handle i32::MIN carefully
        let abs = if offset == i32::MIN {
            (i32::MIN as i64).unsigned_abs() as u32
        } else {
            (-offset) as u32
        };
        pos += write_u32_digits(&mut buf[pos..], abs);
    } else if offset > 0 {
        pos += write_u32_digits(&mut buf[pos..], offset as u32);
    } else {
        // offset == 0: just write "(%rbp)"
    }
    // Write "(%rbp)"
    buf[pos..pos + 6].copy_from_slice(b"(%rbp)");
    pos + 6
}

/// Write decimal digits of a u32 into a buffer. Returns the number of bytes written.
#[inline]
fn write_u32_digits(buf: &mut [u8], mut v: u32) -> usize {
    if v == 0 {
        buf[0] = b'0';
        return 1;
    }
    // Write digits in reverse order
    let mut tmp = [0u8; 10];
    let mut len = 0;
    while v > 0 {
        tmp[len] = b'0' + (v % 10) as u8;
        v /= 10;
        len += 1;
    }
    // Reverse into output buffer
    for i in 0..len {
        buf[i] = tmp[len - 1 - i];
    }
    len
}

/// Convert a register family ID and move size to the register name string.
#[inline]
pub(super) fn reg_id_to_name(id: RegId, size: MoveSize) -> &'static str {
    debug_assert!(id <= 15, "invalid register family id: {}", id);
    let row = match size {
        MoveSize::Q | MoveSize::SLQ | MoveSize::SD => 0,
        MoveSize::L | MoveSize::SS => 1,
        MoveSize::W => 2,
        MoveSize::B => 3,
    };
    REG_NAMES[row][id as usize]
}

// ── tests: the implicit-operand oracle ───────────────────────────────────────

#[cfg(test)]
mod implicit_oracle_tests {
    use super::*;

    const RAX: u16 = 1 << 0;
    const RCX: u16 = 1 << 1;
    const RDX: u16 = 1 << 2;
    const RBX: u16 = 1 << 3;
    const RSI: u16 = 1 << 6;
    const RDI: u16 = 1 << 7;
    const R8: u16 = 1 << 8;
    const R9: u16 = 1 << 9;
    const R10: u16 = 1 << 10;
    const R11: u16 = 1 << 11;

    fn reads(line: &str) -> u16 {
        implicit_read_refs(line.trim().as_bytes())
    }
    fn writes(line: &str) -> u16 {
        implicit_write_refs(line.trim().as_bytes())
    }
    fn touches(line: &str) -> u16 {
        implicit_reg_refs(line.trim().as_bytes())
    }

    #[test]
    fn the_widening_branch_writes_rdx_and_only_reads_rax() {
        // The heart of the stress-lab `intexpr` seed 1 miscompile: `cqto`
        // rewrote %rdx while being invisible to every textual scan.
        assert_eq!(touches("    cqto"), RAX | RDX);
        assert_eq!(reads("    cqto"), RAX);
        assert_eq!(writes("    cqto"), RDX);
        for m in ["cwtd", "cltd", "cqto", "cwd", "cdq", "cqo"] {
            assert_eq!(writes(m), RDX, "{m}");
            assert_eq!(reads(m), RAX, "{m}");
        }
    }

    #[test]
    fn in_place_extensions_read_and_write_the_accumulator() {
        for m in ["cbtw", "cwtl", "cltq", "cbw", "cwde", "cdqe"] {
            assert_eq!(touches(m), RAX, "{m}");
        }
    }

    #[test]
    fn integer_division_names_its_implicit_pair_exactly() {
        assert_eq!(writes("    idivq %r11"), RAX | RDX);
        assert_eq!(reads("    idivq %r11"), RAX | RDX);
        assert_eq!(writes("    divl %ecx"), RAX | RDX);
        // SSE divides/multiplies have fully explicit operands: a prefix
        // match against "div"/"mul" would tax every FP loop with a
        // phantom accumulator pair.
        assert_eq!(touches("    divsd %xmm0, %xmm1"), 0);
        assert_eq!(touches("    mulsd %xmm0, %xmm1"), 0);
        assert_eq!(touches("    mulpd %xmm0, %xmm1"), 0);
        // The byte forms divide/multiply AX: quotient AL, remainder AH,
        // product AX — the %rdx family is never touched.
        assert_eq!(writes("    idivb %r11b"), RAX);
        assert_eq!(writes("    mulb %cl"), RAX);
    }

    #[test]
    fn single_operand_imul_is_the_widening_form() {
        assert_eq!(writes("    imulq %r11"), RAX | RDX);
        assert_eq!(reads("    imulq %r11"), RAX);
        // The two/three-operand forms name every register they touch.
        assert_eq!(touches("    imulq %rcx, %rax"), 0);
        assert_eq!(touches("    imulq $5, %rcx, %rax"), 0);
    }

    #[test]
    fn system_instructions_have_their_contract() {
        // The kernel returns in %rax and destroys %rcx/%r11; the argument
        // registers are read-only inputs.
        assert_eq!(writes("    syscall"), RAX | RCX | R11);
        assert_eq!(reads("    syscall") & RDI, RDI);
        assert_eq!(reads("    syscall") & (R8 | R9 | R10), R8 | R9 | R10);
        assert_eq!(writes("    cpuid"), RAX | RBX | RCX | RDX);
        assert_eq!(writes("    rdtsc"), RAX | RDX);
        // rdtscp additionally returns the TSC_AUX in %rcx; rdtsc does not.
        assert_eq!(writes("    rdtscp"), RAX | RCX | RDX);
    }

    #[test]
    fn rep_prefixes_contribute_the_count_register() {
        assert_eq!(touches("    rep stosq"), RCX | RAX | RDI);
        assert_eq!(writes("    rep stosq"), RCX | RDI);
        assert_eq!(writes("    repne scasq"), RCX | RDI);
        // `rep ret` / `rep nop` (pause) are branch-hint idioms, not
        // counted loops: they touch no register at all.
        assert_eq!(touches("    rep ret"), 0);
        assert_eq!(touches("    rep nop"), 0);
    }

    #[test]
    fn string_primitives_post_increment_their_pointers() {
        assert_eq!(writes("    movsq"), RSI | RDI);
        assert_eq!(writes("    movsl"), RSI | RDI);
        assert_eq!(writes("    lodsq"), RAX | RSI);
        assert_eq!(reads("    lodsq"), RSI);
        // movslq is a sign-extending move, not a string primitive — the
        // two differ by exactly one letter and one semantic universe.
        assert_eq!(touches("    movslq %eax, %rax"), 0);
    }

    #[test]
    fn cmpxchg8b_reads_the_payload_and_reloads_the_compare() {
        assert_eq!(reads("    cmpxchg8b (%rdi)"), RAX | RBX | RCX | RDX);
        assert_eq!(writes("    cmpxchg8b (%rdi)"), RAX | RDX);
    }

    #[test]
    fn non_instructions_name_nothing() {
        assert_eq!(touches(".LBB1:"), 0);
        assert_eq!(touches(".cfi_startproc"), 0);
        assert_eq!(touches("#APP"), 0);
        assert_eq!(touches(""), 0);
        assert_eq!(touches("frobnicate"), 0);
        // lock alone contributes nothing; the prefixed mnemonic decides.
        assert_eq!(touches("    lock"), 0);
    }

    #[test]
    fn the_union_reaches_scan_register_refs() {
        // The classified `reg_refs` of a bare `cqto` used to be 0 — the
        // exact hole behind the stress-lab intexpr miscompile.
        let info = classify_line("    cqto");
        assert_eq!(info.reg_refs & RDX, RDX);
        assert_eq!(info.reg_refs & RAX, RAX);
        let info = classify_line("    idivq %r11");
        assert_eq!(info.reg_refs & RDX, RDX);
        assert_eq!(info.reg_refs & R11, R11);
        let info = classify_line("    syscall");
        assert_eq!(info.reg_refs & R11, R11);
        // The Cmp kind funnels through the same scan (cmpxchg lives there).
        let info = classify_line("    cmpxchgq %rbx, (%rdi)");
        assert_eq!(info.reg_refs & RAX, RAX);
    }
}
