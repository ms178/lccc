//! The algebra of scalar FMA3 encodings: roles versus forms.
//!
//! Every scalar FMA computes `±(m₁ × m₂) ± a` over three values and rounds
//! once.  The ISA offers three *encodings* of that one computation, which
//! differ only in which of the three operand positions holds which role:
//!
//! ```text
//!               Intel operand order        AT&T text          roles
//!   132   dst = dst × src3 + src2     op0=src3 op1=src2   m = {dst, op0}, a = op1
//!   213   dst = src2 × dst + src3     op0=src3 op1=src2   m = {dst, op1}, a = op0
//!   231   dst = src2 × src3 + dst     op0=src3 op1=src2   m = {op1, op0}, a = dst
//! ```
//!
//! The four sign families (`vfmadd`, `vfmsub`, `vfnmadd`, `vfnmsub`) attach
//! their signs to the same two roles in every form, so a change of form never
//! changes the arithmetic: multiplication is commutative and bitwise exact in
//! IEEE-754, and the FMA rounds once whichever operand order it was encoded
//! with.  What a change of form DOES change is two things the surrounding code
//! cares about:
//!
//! * **which register receives the result** — the destination is always one of
//!   the three role registers, and each form makes a different role the
//!   destination;
//! * **where a memory operand may sit** — only Intel `src3`, which is AT&T
//!   operand 0 in every form, has an r/m encoding.
//!
//! Two peephole passes need to move between forms: the copy-bracket collapse
//! in [`super::vector_copy`] (put the result where the copy-out wanted it) and
//! the load fold in [`super::memory_fold`] (put the loaded value at operand 0).
//! Both used to special-case the 231 form; the census over the benchmark
//! corpus found the 132 form on `a * b + c` — the most common FMA shape there
//! is — and loads feeding operand 1, neither of which a 231-only rule reaches.
//! This module is the one place that knows the table above, so that neither
//! pass reasons about operand positions by hand.
//!
//! Everything here is scalar (`sd`/`ss`) and register-spelled `%xmm`: packed
//! forms make the whole register the value, EVEX masks make the destination a
//! partial write, and embedded broadcasts give operand 0 a different role.
//! All three are refused by the parser rather than reasoned about.

/// The sign family of a scalar FMA mnemonic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FmaFamily {
    /// `vfmadd`: `+(m₁ × m₂) + a`
    Madd,
    /// `vfmsub`: `+(m₁ × m₂) − a`
    Msub,
    /// `vfnmadd`: `−(m₁ × m₂) + a`
    Nmadd,
    /// `vfnmsub`: `−(m₁ × m₂) − a`
    Nmsub,
}

impl FmaFamily {
    fn parse(s: &str) -> Option<(Self, &str)> {
        let rest = s.strip_prefix("vf")?;
        // Longest prefixes first: `nmadd` must not be read as `n` + `madd`.
        if let Some(r) = rest.strip_prefix("nmadd") {
            return Some((Self::Nmadd, r));
        }
        if let Some(r) = rest.strip_prefix("nmsub") {
            return Some((Self::Nmsub, r));
        }
        if let Some(r) = rest.strip_prefix("madd") {
            return Some((Self::Madd, r));
        }
        if let Some(r) = rest.strip_prefix("msub") {
            return Some((Self::Msub, r));
        }
        None
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Madd => "vfmadd",
            Self::Msub => "vfmsub",
            Self::Nmadd => "vfnmadd",
            Self::Nmsub => "vfnmsub",
        }
    }
}

/// The three operand orders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FmaForm {
    F132,
    F213,
    F231,
}

impl FmaForm {
    fn parse(s: &str) -> Option<(Self, &str)> {
        if let Some(r) = s.strip_prefix("132") {
            return Some((Self::F132, r));
        }
        if let Some(r) = s.strip_prefix("213") {
            return Some((Self::F213, r));
        }
        if let Some(r) = s.strip_prefix("231") {
            return Some((Self::F231, r));
        }
        None
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::F132 => "132",
            Self::F213 => "213",
            Self::F231 => "231",
        }
    }
}

/// The element width, named by the mnemonic suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FmaWidth {
    /// `sd`: one double, 64 bits.
    Sd,
    /// `ss`: one float, 32 bits.
    Ss,
}

impl FmaWidth {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "sd" => Some(Self::Sd),
            "ss" => Some(Self::Ss),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Sd => "sd",
            Self::Ss => "ss",
        }
    }

    /// Bits of the destination the instruction defines.
    pub(super) fn bits(self) -> u16 {
        match self {
            Self::Sd => 64,
            Self::Ss => 32,
        }
    }
}

/// A scalar FMA3 line, decoded but not interpreted: `ops` is the AT&T operand
/// text in source order, `ops[2]` the destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ScalarFma<'a> {
    pub(super) family: FmaFamily,
    pub(super) form: FmaForm,
    pub(super) width: FmaWidth,
    pub(super) ops: [&'a str; 3],
}

/// The same instruction read as arithmetic: two multiplicands and an addend,
/// each the operand text that names it (a register spelling or a memory
/// reference).  `mult` keeps Intel's `[src2, src3]`-derived order so that
/// re-encoding an unchanged instruction reproduces its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FmaRoles<'a> {
    pub(super) family: FmaFamily,
    pub(super) width: FmaWidth,
    pub(super) mult: [&'a str; 2],
    pub(super) addend: &'a str,
}

/// Number of the `%xmm` register an operand names, if it is one.
pub(super) fn xmm_reg(op: &str) -> Option<u8> {
    let d = op.trim().strip_prefix("%xmm")?;
    if d.is_empty() || d.len() > 2 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u8 = d.parse().ok()?;
    (n < 16).then_some(n)
}

/// An operand that is neither a register nor an immediate: a memory reference
/// of any addressing form.
pub(super) fn is_memory(op: &str) -> bool {
    let op = op.trim();
    !op.is_empty() && !op.starts_with('%') && !op.starts_with('$')
}

/// Decode `t` as a scalar FMA3 with three plain operands: the destination and
/// operand 1 must be `%xmm` registers, operand 0 an `%xmm` register or a
/// memory reference.  Anything with a `{` (writemask, broadcast, rounding
/// control) or a `%ymm`/`%zmm` spelling is not this instruction.
pub(super) fn parse_scalar_fma(t: &str) -> Option<ScalarFma<'_>> {
    let (mn, operands) = split_mnemonic(t);
    let (family, rest) = FmaFamily::parse(mn)?;
    let (form, rest) = FmaForm::parse(rest)?;
    let width = FmaWidth::parse(rest)?;
    if operands.contains('{') {
        return None;
    }
    let ops: Vec<&str> = operand_list(operands).into_iter().map(str::trim).collect();
    let [op0, op1, dst] = <[&str; 3]>::try_from(ops).ok()?;
    if xmm_reg(dst).is_none() || xmm_reg(op1).is_none() {
        return None;
    }
    if xmm_reg(op0).is_none() && !is_memory(op0) {
        return None;
    }
    Some(ScalarFma {
        family,
        form,
        width,
        ops: [op0, op1, dst],
    })
}

impl<'a> ScalarFma<'a> {
    /// The destination register number.
    pub(super) fn dst(&self) -> u8 {
        // The parser admitted only a register here.
        xmm_reg(self.ops[2]).unwrap_or(u8::MAX)
    }

    /// Read the instruction as arithmetic.
    pub(super) fn roles(&self) -> FmaRoles<'a> {
        let [op0, op1, dst] = self.ops;
        let (mult, addend) = match self.form {
            FmaForm::F132 => ([dst, op0], op1),
            FmaForm::F213 => ([op1, dst], op0),
            FmaForm::F231 => ([op1, op0], dst),
        };
        FmaRoles {
            family: self.family,
            width: self.width,
            mult,
            addend,
        }
    }

    /// The line's text, rebuilt from its parts.
    pub(super) fn render(&self) -> String {
        format!(
            "{}{}{} {}, {}, {}",
            self.family.as_str(),
            self.form.as_str(),
            self.width.as_str(),
            self.ops[0],
            self.ops[1],
            self.ops[2]
        )
    }
}

impl<'a> FmaRoles<'a> {
    /// Rename every role naming register `from` to the operand text `to`.
    pub(super) fn substitute(&self, from: u8, to: &'a str) -> FmaRoles<'a> {
        let sub = |op: &'a str| if xmm_reg(op) == Some(from) { to } else { op };
        FmaRoles {
            family: self.family,
            width: self.width,
            mult: [sub(self.mult[0]), sub(self.mult[1])],
            addend: sub(self.addend),
        }
    }

    /// How many of the three roles name register `n`.
    pub(super) fn mentions(&self, n: u8) -> usize {
        [self.mult[0], self.mult[1], self.addend]
            .iter()
            .filter(|op| xmm_reg(op) == Some(n))
            .count()
    }

    /// The encoding that computes these roles into register `dst`, or `None`
    /// when no single instruction can: `dst` must be one of the role
    /// registers, and at most one role may be a memory reference.
    ///
    /// When `dst` is the addend the form is 231; when it is a multiplicand the
    /// other multiplicand and the addend share operands 0 and 1, and the memory
    /// operand — if there is one — decides which goes where.  With two
    /// registers either form is legal and `prefer` breaks the tie, so that an
    /// instruction re-encoded without a change of destination keeps its text;
    /// failing a usable preference, 213 is chosen, which is the form Clang and
    /// ICX emit for `__builtin_fma(a, b, c)`.
    pub(super) fn encode(&self, dst: u8, prefer: FmaForm) -> Option<ScalarFma<'a>> {
        let n_mem = [self.mult[0], self.mult[1], self.addend]
            .iter()
            .filter(|op| is_memory(op))
            .count();
        if n_mem > 1 {
            return None;
        }
        let dst_text = if xmm_reg(self.addend) == Some(dst) {
            self.addend
        } else if xmm_reg(self.mult[0]) == Some(dst) {
            self.mult[0]
        } else if xmm_reg(self.mult[1]) == Some(dst) {
            self.mult[1]
        } else {
            return None;
        };
        let (form, op0, op1) = if xmm_reg(self.addend) == Some(dst) {
            // Operand 1 is Intel src2, operand 0 is src3: `mult` was built in
            // that order, and the memory operand can only be src3.
            let (m2, m3) = (self.mult[0], self.mult[1]);
            if is_memory(m2) {
                (FmaForm::F231, m2, m3)
            } else {
                (FmaForm::F231, m3, m2)
            }
        } else {
            let other = if xmm_reg(self.mult[0]) == Some(dst) {
                self.mult[1]
            } else {
                self.mult[0]
            };
            let a = self.addend;
            if is_memory(other) {
                (FmaForm::F132, other, a)
            } else if is_memory(a) {
                (FmaForm::F213, a, other)
            } else {
                match prefer {
                    FmaForm::F132 => (FmaForm::F132, other, a),
                    _ => (FmaForm::F213, a, other),
                }
            }
        };
        // Two register roles may name the destination (`x * x + a` with the
        // result in `x`); the reads all happen before the write, so that is
        // legal in every form.  What is not legal is a memory operand anywhere
        // but position 0, which the arms above never produce.
        debug_assert!(!is_memory(op1) && !is_memory(dst_text));
        Some(ScalarFma {
            family: self.family,
            form,
            width: self.width,
            ops: [op0, op1, dst_text],
        })
    }
}

/// Split a line into its mnemonic and its operand text.
fn split_mnemonic(t: &str) -> (&str, &str) {
    let t = t.trim();
    match t.find(|c: char| c == ' ' || c == '\t') {
        Some(p) => (&t[..p], t[p..].trim()),
        None => (t, ""),
    }
}

/// Top-level comma split: commas inside `(...)` belong to the address.
fn operand_list(operands: &str) -> Vec<&str> {
    let mut out = Vec::with_capacity(3);
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, b) in operands.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => {
                out.push(&operands[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if !operands.is_empty() {
        out.push(&operands[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILIES: [FmaFamily; 4] = [
        FmaFamily::Madd,
        FmaFamily::Msub,
        FmaFamily::Nmadd,
        FmaFamily::Nmsub,
    ];
    const FORMS: [FmaForm; 3] = [FmaForm::F132, FmaForm::F213, FmaForm::F231];
    const WIDTHS: [FmaWidth; 2] = [FmaWidth::Sd, FmaWidth::Ss];

    /// A role set with the operand texts sorted so that two encodings of the
    /// same arithmetic compare equal whatever order their multiplicands took.
    fn canonical(r: &FmaRoles) -> (FmaFamily, FmaWidth, Vec<String>, String) {
        let mut m = vec![r.mult[0].to_string(), r.mult[1].to_string()];
        m.sort();
        (r.family, r.width, m, r.addend.to_string())
    }

    #[test]
    fn every_form_parses_and_renders_back_to_itself() {
        for f in FAMILIES {
            for form in FORMS {
                for w in WIDTHS {
                    let line = format!(
                        "{}{}{} %xmm1, %xmm2, %xmm3",
                        f.as_str(),
                        form.as_str(),
                        w.as_str()
                    );
                    let p = parse_scalar_fma(&line).expect(&line);
                    assert_eq!((p.family, p.form, p.width), (f, form, w));
                    assert_eq!(p.render(), line);
                    let mem = format!(
                        "{}{}{} 8(%rsp,%rax,8), %xmm2, %xmm3",
                        f.as_str(),
                        form.as_str(),
                        w.as_str()
                    );
                    let p = parse_scalar_fma(&mem).expect(&mem);
                    assert_eq!(p.ops[0], "8(%rsp,%rax,8)");
                    assert_eq!(p.render(), mem);
                }
            }
        }
    }

    #[test]
    fn the_parser_refuses_everything_that_is_not_a_plain_scalar_fma() {
        for bad in [
            "vfmadd231pd %xmm1, %xmm2, %xmm3",
            "vfmadd231ps %ymm1, %ymm2, %ymm3",
            "vfmadd231sd %xmm1, %xmm2, %xmm3{%k1}",
            "vfmadd231sd (%rdi){1to2}, %xmm2, %xmm3",
            "vfmadd231sd %xmm1, %xmm2, %ymm3",
            "vfmadd231sd %xmm1, (%rdi), %xmm3",
            "vfmadd231sd %xmm1, %xmm2, (%rdi)",
            "vfmadd231sd %xmm1, %xmm2",
            "vfmadd231sd $1, %xmm2, %xmm3",
            "vfnadd231sd %xmm1, %xmm2, %xmm3",
            "vfmadd321sd %xmm1, %xmm2, %xmm3",
            "vfmadd231sd %xmm16, %xmm2, %xmm3",
            "vaddsd %xmm1, %xmm2, %xmm3",
        ] {
            assert!(parse_scalar_fma(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn roles_follow_the_intel_table() {
        let r = parse_scalar_fma("vfmadd132sd %xmm1, %xmm2, %xmm3")
            .unwrap()
            .roles();
        assert_eq!((r.mult, r.addend), (["%xmm3", "%xmm1"], "%xmm2"));
        let r = parse_scalar_fma("vfmadd213sd %xmm1, %xmm2, %xmm3")
            .unwrap()
            .roles();
        assert_eq!((r.mult, r.addend), (["%xmm2", "%xmm3"], "%xmm1"));
        let r = parse_scalar_fma("vfmadd231sd %xmm1, %xmm2, %xmm3")
            .unwrap()
            .roles();
        assert_eq!((r.mult, r.addend), (["%xmm2", "%xmm1"], "%xmm3"));
    }

    /// The exhaustive oracle: for every family, form, width, choice of which
    /// role register becomes the destination, and placement of a memory
    /// operand, the re-encoding must (1) exist exactly when the ISA has one,
    /// (2) compute the same roles, (3) write the requested register, (4) keep
    /// any memory operand at operand 0, and (5) parse back to itself.
    #[test]
    fn encode_preserves_roles_for_every_form_destination_and_memory_placement() {
        let mut checked = 0usize;
        for f in FAMILIES {
            for form in FORMS {
                for w in WIDTHS {
                    // Memory in role slot 0/1/2 of the ORIGINAL line's
                    // operand 0, or none.
                    for mem_at_op0 in [false, true] {
                        let op0 = if mem_at_op0 { "16(%rdi)" } else { "%xmm1" };
                        let line = format!(
                            "{}{}{} {op0}, %xmm2, %xmm3",
                            f.as_str(),
                            form.as_str(),
                            w.as_str()
                        );
                        let fma = parse_scalar_fma(&line).unwrap();
                        let roles = fma.roles();
                        for dst in 0u8..16 {
                            let got = roles.encode(dst, form);
                            let dst_is_role = roles.mentions(dst) > 0;
                            assert_eq!(got.is_some(), dst_is_role, "{line} -> xmm{dst}: {got:?}");
                            let Some(enc) = got else { continue };
                            checked += 1;
                            assert_eq!(enc.dst(), dst, "{line} -> {}", enc.render());
                            assert_eq!(
                                canonical(&enc.roles()),
                                canonical(&roles),
                                "{line} -> {}",
                                enc.render()
                            );
                            assert!(
                                !is_memory(enc.ops[1]) && !is_memory(enc.ops[2]),
                                "{}",
                                enc.render()
                            );
                            let rendered = enc.render();
                            let back = parse_scalar_fma(&rendered).expect("re-encoded line parses");
                            assert_eq!(back.render(), rendered);
                            // Re-encoding without changing the destination
                            // reproduces the original text exactly.
                            if dst == fma.dst() {
                                assert_eq!(enc.render(), line);
                            }
                        }
                    }
                }
            }
        }
        // 4 families × 3 forms × 2 widths × 2 memory placements × the number
        // of distinct role registers (3 with a register operand 0, 2 with
        // memory there).
        assert_eq!(checked, 4 * 3 * 2 * (3 + 2));
    }

    #[test]
    fn substitution_renames_a_role_and_can_introduce_a_memory_operand() {
        let fma = parse_scalar_fma("vfmadd231sd %xmm1, %xmm5, %xmm3").unwrap();
        let roles = fma.roles().substitute(5, "bodies+96(%rip)");
        let enc = roles.encode(3, FmaForm::F231).unwrap();
        assert_eq!(enc.render(), "vfmadd231sd bodies+96(%rip), %xmm1, %xmm3");
        // The addend as memory forces 213 when a multiplicand is the target.
        let fma = parse_scalar_fma("vfmadd231sd %xmm1, %xmm0, %xmm5").unwrap();
        let roles = fma.roles().substitute(5, ".LC0(%rip)");
        assert_eq!(
            roles.encode(0, FmaForm::F231).unwrap().render(),
            "vfmadd213sd .LC0(%rip), %xmm1, %xmm0"
        );
        // The other multiplicand as memory forces 132.
        let fma = parse_scalar_fma("vfmadd231sd %xmm1, %xmm0, %xmm5").unwrap();
        let roles = fma.roles().substitute(1, "(%rdi)");
        assert_eq!(
            roles.encode(0, FmaForm::F231).unwrap().render(),
            "vfmadd132sd (%rdi), %xmm5, %xmm0"
        );
        // Two memory roles have no encoding.
        assert!(
            roles
                .substitute(5, "8(%rdi)")
                .encode(0, FmaForm::F231)
                .is_none()
        );
    }

    #[test]
    fn the_tie_break_keeps_a_legal_original_form_and_otherwise_picks_213() {
        let roles = parse_scalar_fma("vfmadd132sd %xmm1, %xmm2, %xmm3")
            .unwrap()
            .roles();
        assert_eq!(
            roles.encode(3, FmaForm::F132).unwrap().render(),
            "vfmadd132sd %xmm1, %xmm2, %xmm3"
        );
        assert_eq!(
            roles.encode(3, FmaForm::F213).unwrap().render(),
            "vfmadd213sd %xmm2, %xmm1, %xmm3"
        );
        // 231 cannot put a multiplicand in the destination, so it is no
        // preference at all there.
        assert_eq!(
            roles.encode(1, FmaForm::F231).unwrap().render(),
            "vfmadd213sd %xmm2, %xmm3, %xmm1"
        );
    }
}
