//! `.riscv.attributes` merging, exact with GNU ld 2.47.
//!
//! The previous linker kept the first input's attribute section verbatim,
//! dropping every tag that only a later input carried (glibc's
//! `Tag_RISCV_priv_spec 1.11` and `Tag_RISCV_unaligned_access` vanished from
//! every hosted link). This ports bfd's `riscv_merge_attributes`
//! (`bfd/elfxx-riscv.c`):
//!
//! - inputs without an attribute section are skipped; the first input that
//!   has one is copied verbatim (including its raw arch string);
//! - `Tag_RISCV_arch`: the ISA strings merge as the union of their subsets in
//!   canonical order, each at its newest version (`riscv_merge_arch_attr_info`).
//!   The merged list then gets bfd's implicit-subset closure and conflict
//!   checks. At link time bfd parses with no default ISA spec, so implied
//!   subsets carry no version and are never printed; they only feed the
//!   conflict checks. The link-time output therefore never contains a subset
//!   no input named, which the oracle confirms (`rv64i2p1_c2p0` +
//!   `rv64i2p1_f2p2_d2p2` merges without `zca`/`zcd`);
//! - `Tag_RISCV_priv_spec{,_minor,_revision}`: the first recognized version
//!   wins and then upgrades to the newest spec class;
//! - `Tag_RISCV_unaligned_access`: OR; `Tag_RISCV_stack_align`: must agree;
//! - any other tag survives only where every attribute-bearing input agrees
//!   (`_bfd_elf_merge_unknown_attribute_low`).
//!
//! Merge failures that bfd turns into "failed to merge target specific data"
//! are link errors here as well.

use super::elf_read::ElfObject;
use std::collections::BTreeMap;

const SECTION: &str = ".riscv.attributes";
const VENDOR: &[u8] = b"riscv";
const TAG_FILE: u64 = 1;
const TAG_STACK_ALIGN: u64 = 4;
const TAG_ARCH: u64 = 5;
const TAG_UNALIGNED_ACCESS: u64 = 6;
const TAG_PRIV_SPEC: u64 = 8;
const TAG_PRIV_SPEC_MINOR: u64 = 10;
const TAG_PRIV_SPEC_REVISION: u64 = 12;
/// `RISCV_UNKNOWN_VERSION`.
const UNKNOWN: i32 = -1;

/// One attribute value. RISC-V types tags by parity
/// (`riscv_elf_obj_attrs_arg_type`): odd tags are strings, even ones ULEB128.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Int(u64),
    Str(Vec<u8>),
}

/// The `Tag_File` attributes of one input; an absent tag is the default.
type Attrs = BTreeMap<u64, Value>;

fn read_uleb(data: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *data.get(*pos).ok_or("truncated ULEB128")?;
        *pos += 1;
        if shift < 64 {
            value |= u64::from(byte & 0x7f) << shift;
        }
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
    }
}

fn write_uleb(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn read_ntbs(data: &[u8], pos: &mut usize) -> Result<Vec<u8>, String> {
    let rest = data.get(*pos..).ok_or("truncated string")?;
    let len = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or("unterminated string")?;
    *pos += len + 1;
    Ok(rest[..len].to_vec())
}

fn read_u32(data: &[u8], pos: usize) -> Option<usize> {
    let bytes: [u8; 4] = data.get(pos..pos + 4)?.try_into().ok()?;
    usize::try_from(u32::from_le_bytes(bytes)).ok()
}

/// Parse an attribute section as `_bfd_elf_parse_attributes` does: only
/// format `'A'` is understood, only the `riscv` vendor's `Tag_File` scope
/// carries attributes (section/symbol scopes are skipped, as by bfd), and a
/// later occurrence of a tag overrides an earlier one.
fn parse(data: &[u8], file: &str) -> Result<Attrs, String> {
    let bad = |what: &str| format!("{file}: malformed {SECTION}: {what}");
    let mut attrs = Attrs::new();
    if data.first() != Some(&b'A') {
        return Ok(attrs);
    }
    let mut pos = 1;
    while pos < data.len() {
        let len = read_u32(data, pos).ok_or_else(|| bad("truncated subsection"))?;
        let end = pos
            .checked_add(len)
            .filter(|&e| len >= 4 && e <= data.len());
        let end = end.ok_or_else(|| bad("subsection length"))?;
        let mut p = pos + 4;
        let vendor = read_ntbs(&data[..end], &mut p).map_err(|e| bad(&e))?;
        if vendor == b"gnu" {
            // bfd merges these through the generic GNU attribute rules;
            // no RISC-V toolchain emits them, so refuse rather than guess.
            return Err(format!(
                "{file}: unsupported GNU-vendor attributes in {SECTION}"
            ));
        }
        if vendor != VENDOR {
            pos = end;
            continue;
        }
        while p < end {
            let scope = read_uleb(&data[..end], &mut p).map_err(|e| bad(&e))?;
            let start = p;
            let sub_len = read_u32(data, p).ok_or_else(|| bad("truncated scope"))?;
            let sub_end = start
                .checked_sub(1)
                .and_then(|s| s.checked_add(sub_len))
                .filter(|&e| e > start && e <= end)
                .ok_or_else(|| bad("scope length"))?;
            p = start + 4;
            if scope != TAG_FILE {
                p = sub_end;
                continue;
            }
            while p < sub_end {
                let tag = read_uleb(&data[..sub_end], &mut p).map_err(|e| bad(&e))?;
                let value = if tag & 1 != 0 {
                    Value::Str(read_ntbs(&data[..sub_end], &mut p).map_err(|e| bad(&e))?)
                } else {
                    Value::Int(read_uleb(&data[..sub_end], &mut p).map_err(|e| bad(&e))?)
                };
                attrs.insert(tag, value);
            }
        }
        pos = end;
    }
    Ok(attrs)
}

fn int(attrs: &Attrs, tag: u64) -> u64 {
    match attrs.get(&tag) {
        Some(Value::Int(v)) => *v,
        _ => 0,
    }
}

fn set_int(attrs: &mut Attrs, tag: u64, value: u64) {
    if value == 0 {
        attrs.remove(&tag);
    } else {
        attrs.insert(tag, Value::Int(value));
    }
}

/// `riscv_get_priv_spec_class_from_numbers`: 0 is `PRIV_SPEC_CLASS_NONE`,
/// then 1.9.1 < 1.10 < 1.11 < 1.12 < 1.13.
fn priv_class(attrs: &Attrs) -> u8 {
    let (major, minor, rev) = (
        int(attrs, TAG_PRIV_SPEC),
        int(attrs, TAG_PRIV_SPEC_MINOR),
        int(attrs, TAG_PRIV_SPEC_REVISION),
    );
    match (major, minor, rev) {
        (1, 9, 1) => 1,
        (1, 10, 0) => 2,
        (1, 11, 0) => 3,
        (1, 12, 0) => 4,
        (1, 13, 0) => 5,
        _ => 0,
    }
}

#[derive(Clone, Copy)]
enum Check {
    Always,
    /// `check_implicit_for_i`: the subset's version is below 2.1.
    IBelow2p1,
    /// `check_implicit_for_zcf`: RV32 with `f`.
    Rv32WithF,
    /// `check_implicit_for_zcd`: `d` is present.
    WithD,
}

/// bfd's `riscv_implicit_subsets[]`, generated by
/// `scripts/gen_riscv_implicit_subsets.py` from binutils 2.47. The order
/// matters: bfd walks the table once.
const IMPLICIT: &[(&str, &[&str], Check)] = &[
    (
        "g",
        &["i", "m", "a", "f", "d", "zicsr", "zifencei"],
        Check::Always,
    ),
    ("e", &["i"], Check::Always),
    ("i", &["zicsr", "zifencei"], Check::IBelow2p1),
    ("zicntr", &["zicsr"], Check::Always),
    ("zihpm", &["zicsr"], Check::Always),
    ("m", &["zmmul"], Check::Always),
    ("zabha", &["zaamo"], Check::Always),
    ("zacas", &["zaamo"], Check::Always),
    ("a", &["zaamo", "zalrsc"], Check::Always),
    ("zalasr", &["zaamo", "zalrsc", "zabha"], Check::Always),
    ("xsfvcp", &["zve32x"], Check::Always),
    ("xsfvqmaccqoq", &["zve32x", "zvl256b"], Check::Always),
    ("xsfvqmaccdod", &["zve32x", "zvl128b"], Check::Always),
    ("xsfvfnrclipxfqf", &["zve32f"], Check::Always),
    ("xtheadvector", &["zicsr"], Check::Always),
    ("xtheadzvamo", &["zaamo"], Check::Always),
    ("xsmtvdot", &["zve32x"], Check::Always),
    ("xsmtvdotii", &["xsmtvdot"], Check::Always),
    ("v", &["zve64d", "zvl128b"], Check::Always),
    ("zvabd", &["zve32x"], Check::Always),
    ("zvfh", &["zvfhmin", "zfhmin"], Check::Always),
    ("zvfhmin", &["zve32f"], Check::Always),
    ("zvfbfwma", &["zve32f", "zfbfmin"], Check::Always),
    ("zvfbfmin", &["zve32f"], Check::Always),
    ("zvfbdota32f", &["zve32f"], Check::Always),
    ("zvfqwbdota8f", &["zve32f"], Check::Always),
    ("zvfqwdota8f", &["zve32f"], Check::Always),
    ("zvfwbdota16bf", &["zve32f"], Check::Always),
    ("zvfwdota16bf", &["zve32f"], Check::Always),
    ("zvqwbdota8i", &["zve32x"], Check::Always),
    ("zvqwbdota16i", &["zve64x"], Check::Always),
    ("zvqwdota8i", &["zve32x"], Check::Always),
    ("zvqwdota16i", &["zve64x"], Check::Always),
    ("zve64d", &["d", "zve64f"], Check::Always),
    ("zve64f", &["zve32f", "zve64x", "zvl64b"], Check::Always),
    ("zve32f", &["f", "zve32x", "zvl32b"], Check::Always),
    ("zve64x", &["zve32x", "zvl64b"], Check::Always),
    ("zve32x", &["zvl32b", "zicsr"], Check::Always),
    ("zvl65536b", &["zvl32768b"], Check::Always),
    ("zvl32768b", &["zvl16384b"], Check::Always),
    ("zvl16384b", &["zvl8192b"], Check::Always),
    ("zvl8192b", &["zvl4096b"], Check::Always),
    ("zvl4096b", &["zvl2048b"], Check::Always),
    ("zvl2048b", &["zvl1024b"], Check::Always),
    ("zvl1024b", &["zvl512b"], Check::Always),
    ("zvl512b", &["zvl256b"], Check::Always),
    ("zvl256b", &["zvl128b"], Check::Always),
    ("zvl128b", &["zvl64b"], Check::Always),
    ("zvl64b", &["zvl32b"], Check::Always),
    ("zicfilp", &["zicsr"], Check::Always),
    ("zicfiss", &["zimop", "zicsr"], Check::Always),
    ("zclsd", &["zca", "zilsd"], Check::Always),
    (
        "sha",
        &[
            "h",
            "ssstateen",
            "shcounterenw",
            "shvstvala",
            "shtvala",
            "shvstvecd",
            "shvsatpa",
            "shgatpa",
        ],
        Check::Always,
    ),
    ("shcounterenw", &["h"], Check::Always),
    ("shgatpa", &["h"], Check::Always),
    ("shtvala", &["h"], Check::Always),
    ("shvsatpa", &["h"], Check::Always),
    ("shvstvala", &["h"], Check::Always),
    ("shvstvecd", &["h"], Check::Always),
    ("h", &["zicsr"], Check::Always),
    ("zhinx", &["zhinxmin"], Check::Always),
    ("zhinxmin", &["zfinx"], Check::Always),
    ("zcd", &["d", "zca"], Check::Always),
    ("zcf", &["f", "zca"], Check::Always),
    ("q", &["d"], Check::Always),
    ("zqinx", &["zdinx"], Check::Always),
    ("d", &["f"], Check::Always),
    ("zdinx", &["zfinx"], Check::Always),
    ("zfa", &["f"], Check::Always),
    ("zfbfmin", &["zfhmin"], Check::Always),
    ("zfh", &["zfhmin"], Check::Always),
    ("zfhmin", &["f"], Check::Always),
    ("zfinx", &["zicsr"], Check::Always),
    ("f", &["zicsr"], Check::Always),
    ("zce", &["zcb", "zcmp", "zcmt"], Check::Always),
    ("zce", &["zcf"], Check::Rv32WithF),
    ("zcb", &["zca"], Check::Always),
    ("zcmp", &["zca"], Check::Always),
    ("zcmop", &["zca"], Check::Always),
    ("zcmt", &["zca", "zicsr"], Check::Always),
    ("c", &["zcf"], Check::Rv32WithF),
    ("c", &["zcd"], Check::WithD),
    ("c", &["zca"], Check::Always),
    ("b", &["zba", "zbb", "zbs"], Check::Always),
    ("zk", &["zkn", "zkr", "zkt"], Check::Always),
    (
        "zkn",
        &["zbkb", "zbkc", "zbkx", "zkne", "zknd", "zknh"],
        Check::Always,
    ),
    (
        "zks",
        &["zbkb", "zbkc", "zbkx", "zksed", "zksh"],
        Check::Always,
    ),
    ("zvbb", &["zvkb"], Check::Always),
    ("zvkng", &["zvkn", "zvkg"], Check::Always),
    ("zvknc", &["zvkn", "zvbc"], Check::Always),
    ("zvkn", &["zvkned", "zvknhb", "zvkb", "zvkt"], Check::Always),
    ("zvksg", &["zvks", "zvkg"], Check::Always),
    ("zvksc", &["zvks", "zvbc"], Check::Always),
    ("zvks", &["zvksed", "zvksh", "zvkb", "zvkt"], Check::Always),
    ("sdtrig", &["zicsr"], Check::Always),
    ("smaia", &["ssaia"], Check::Always),
    ("smcdeleg", &["ssccfg"], Check::Always),
    ("smcsrind", &["sscsrind"], Check::Always),
    ("smcntrpmf", &["zicsr"], Check::Always),
    ("smctr", &["zicsr"], Check::Always),
    ("smrnmi", &["zicsr"], Check::Always),
    ("smstateen", &["ssstateen"], Check::Always),
    ("smepmp", &["zicsr"], Check::Always),
    ("smdbltrp", &["zicsr"], Check::Always),
    ("smnpm", &["zicsr"], Check::Always),
    ("smmpm", &["zicsr"], Check::Always),
    ("ssaia", &["zicsr"], Check::Always),
    ("ssccfg", &["sscsrind"], Check::Always),
    ("sscsrind", &["zicsr"], Check::Always),
    ("sscofpmf", &["zicsr"], Check::Always),
    ("sscounterenw", &["zicsr"], Check::Always),
    ("ssctr", &["zicsr"], Check::Always),
    ("ssstateen", &["zicsr"], Check::Always),
    ("ssstrict", &["zicsr"], Check::Always),
    ("sstc", &["zicsr"], Check::Always),
    ("sstvala", &["zicsr"], Check::Always),
    ("sstvecd", &["zicsr"], Check::Always),
    ("ssu64xl", &["zicsr"], Check::Always),
    ("ssdbltrp", &["zicsr"], Check::Always),
    ("ssnpm", &["zicsr"], Check::Always),
    ("svade", &["zicsr"], Check::Always),
    ("svadu", &["zicsr"], Check::Always),
    ("svbare", &["zicsr"], Check::Always),
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Subset {
    name: String,
    major: i32,
    minor: i32,
}

const CANONICAL_ORDER: &str = "eigmafdqlcbkjtpvnh";

/// `riscv_ext_order`: positive for standard single letters, 0 otherwise.
fn ext_order(c: u8) -> i32 {
    CANONICAL_ORDER
        .bytes()
        .position(|b| b == c)
        .and_then(|i| i32::try_from(i + 1).ok())
        .unwrap_or(0)
}

/// `enum riscv_prefix_ext_class` via `parse_config[]`; `Single` is 5.
fn prefix_class(name: &str) -> i32 {
    if name.starts_with("zxm") {
        3
    } else if name.starts_with('z') {
        1
    } else if name.starts_with('s') {
        2
    } else if name.starts_with('x') {
        4
    } else {
        5
    }
}

/// `riscv_compare_subsets`.
fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let (mut o1, mut o2) = (ext_order(ab[0]), ext_order(bb[0]));
    if o1 > 0 && o2 > 0 {
        return o1.cmp(&o2);
    }
    let (c1, c2) = (prefix_class(a), prefix_class(b));
    if c1 != 5 {
        o1 = -c1;
    }
    if c2 != 5 {
        o2 = -c2;
    }
    if o1 == o2 {
        if c1 == 1 {
            let z1 = ab.get(1).map_or(0, |&c| ext_order(c));
            let z2 = bb.get(1).map_or(0, |&c| ext_order(c));
            if z1 != z2 {
                return z1.cmp(&z2);
            }
        }
        return a[1.min(a.len())..]
            .to_ascii_lowercase()
            .cmp(&b[1.min(b.len())..].to_ascii_lowercase());
    }
    o2.cmp(&o1)
}

/// A subset list kept in canonical order (`riscv_add_subset`).
#[derive(Default)]
struct SubsetList(Vec<Subset>);

impl SubsetList {
    fn find(&self, name: &str) -> Option<&Subset> {
        self.0.iter().find(|s| s.name == name)
    }

    fn has(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    fn add(&mut self, name: &str, major: i32, minor: i32) {
        let at = self
            .0
            .iter()
            .position(|s| compare(&s.name, name).is_gt())
            .unwrap_or(self.0.len());
        self.0.insert(
            at,
            Subset {
                name: name.to_owned(),
                major,
                minor,
            },
        );
    }

    /// `riscv_parse_add_implicit_subsets`, followed by
    /// `riscv_parse_check_conflicts`.
    fn close_and_check(&mut self, xlen: u32) -> Result<(), String> {
        for &(ext, implied, check) in IMPLICIT {
            let Some(subset) = self.find(ext) else {
                continue;
            };
            let applies = match check {
                Check::Always => true,
                Check::IBelow2p1 => subset.major < 2 || (subset.major == 2 && subset.minor < 1),
                Check::Rv32WithF => xlen == 32 && self.has("f"),
                Check::WithD => self.has("d"),
            };
            if applies {
                for name in implied {
                    if !self.has(name) {
                        self.add(name, UNKNOWN, UNKNOWN);
                    }
                }
            }
        }
        let conflict = |msg: String| Err(format!("conflicting RISC-V ISA extensions: {msg}"));
        if self.has("e") && self.has("h") {
            return conflict(format!("rv{xlen}e does not support the `h' extension"));
        }
        if let Some(q) = self.find("q")
            && (q.major < 2 || (q.major == 2 && q.minor < 2))
            && xlen < 64
        {
            return conflict(format!("rv{xlen} does not support the `q' extension"));
        }
        if self.has("zcmp") && self.has("zcd") {
            return conflict("zcmp' is incompatible with `d' and `c', or `zcd".into());
        }
        if self.has("zcf") && xlen > 32 {
            return conflict(format!("rv{xlen} does not support the `zcf' extension"));
        }
        if self.has("zfinx") && self.has("f") {
            return conflict("`zfinx' is conflict with the `f/d/q/zfh/zfhmin' extension".into());
        }
        if self.has("xtheadvector") && self.has("zve32x") {
            return conflict("`xtheadvector' is conflict with the `zve32x' extension".into());
        }
        if self.has("zclsd") && ((self.has("c") && self.has("f")) || self.has("zcf")) {
            return conflict("`zclsd' is conflict with the `c+f'/ `zcf' extension".into());
        }
        for name in ["ssnpm", "smnpm", "smmpm", "sspm", "supm"] {
            if self.has(name) && xlen != 64 {
                return conflict(format!("rv{xlen} does not support the `{name}' extension"));
            }
        }
        let zve = self.0.iter().any(|s| s.name.starts_with("zve"));
        let zvl = self.0.iter().any(|s| s.name.starts_with("zvl"));
        if zvl && !zve {
            return conflict(
                "zvl*b extensions need to enable either `v' or `zve' extension".into(),
            );
        }
        Ok(())
    }

    /// `riscv_arch_str`: the head is always printed; later subsets without a
    /// version (implied ones) and an `i` following `e` are skipped.
    fn arch_string(&self, xlen: u32) -> String {
        let mut out = format!("rv{xlen}");
        let mut prev: Option<&Subset> = None;
        for (i, s) in self.0.iter().enumerate() {
            if i > 0
                && ((prev.is_some_and(|p| p.name == "e") && s.name == "i")
                    || s.major == UNKNOWN
                    || s.minor == UNKNOWN)
            {
                continue;
            }
            let sep = if s.name == "i" || s.name == "e" {
                ""
            } else {
                "_"
            };
            out.push_str(&format!("{sep}{}{}p{}", s.name, s.major, s.minor));
            prev = Some(s);
        }
        out
    }
}

/// `riscv_parsing_subset_version`: `<major>[p<minor>]`; returns the
/// versions and the bytes consumed. No digits at all is the unknown version.
fn parse_version(s: &[u8]) -> (i32, i32, usize) {
    let (mut major, mut minor, mut value, mut major_p) = (0i32, 0i32, 0i32, true);
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'p' => {
                if !s.get(i + 1).is_some_and(u8::is_ascii_digit) {
                    break;
                }
                major = value;
                major_p = false;
                value = 0;
            }
            c if c.is_ascii_digit() => {
                value = value.saturating_mul(10).saturating_add(i32::from(c - b'0'));
            }
            _ => break,
        }
        i += 1;
    }
    if major_p {
        major = value;
    } else {
        minor = value;
    }
    if major == 0 && minor == 0 {
        (UNKNOWN, UNKNOWN, i)
    } else {
        (major, minor, i)
    }
}

/// `riscv_parse_subset` with ld's parameters (no default ISA spec, unknown
/// prefixed extensions accepted). A subset without a version cannot get one
/// at link time and is dropped with bfd's diagnostic; `g` is implicit.
fn parse_arch(
    arch: &str,
    file: &str,
    warnings: &mut Vec<String>,
) -> Result<(u32, SubsetList), String> {
    let bad = |what: &str| Err(format!("{file}: ISA string `{arch}': {what}"));
    if arch.bytes().any(|b| b.is_ascii_uppercase()) {
        return bad("the ISA string must be in lowercase");
    }
    let (xlen, rest) = if let Some(r) = arch.strip_prefix("rv32") {
        (32, r)
    } else if let Some(r) = arch.strip_prefix("rv64") {
        (64, r)
    } else {
        return bad("must begin with rv32 or rv64");
    };
    if !matches!(rest.as_bytes().first(), Some(b'e' | b'i' | b'g')) {
        return bad("first ISA extension must be `e', `i' or `g'");
    }
    let mut list = SubsetList::default();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'_' {
            i += 1;
            continue;
        }
        let (name, major, minor) = if matches!(bytes[i], b'z' | b's' | b'x') {
            let end = bytes[i..]
                .iter()
                .position(|&b| b == b'_')
                .map_or(bytes.len(), |e| i + e);
            let token = &bytes[i..end];
            // The version is the trailing `<digits>[p<digits>]`.
            let mut cut = token.len();
            while cut > 0 && token[cut - 1].is_ascii_digit() {
                cut -= 1;
            }
            if cut > 1
                && cut < token.len()
                && token[cut - 1] == b'p'
                && token[cut - 2].is_ascii_digit()
            {
                cut -= 1;
                while cut > 0 && token[cut - 1].is_ascii_digit() {
                    cut -= 1;
                }
            }
            if cut == 0 {
                return bad("empty extension name");
            }
            let (major, minor, _) = parse_version(&token[cut..]);
            i = end;
            (
                String::from_utf8_lossy(&token[..cut]).into_owned(),
                major,
                minor,
            )
        } else {
            let c = bytes[i];
            if ext_order(c) == 0 {
                return bad(&format!(
                    "unknown standard ISA extension `{}'",
                    char::from(c)
                ));
            }
            let (major, minor, used) = parse_version(&bytes[i + 1..]);
            i += 1 + used;
            (char::from(c).to_string(), major, minor)
        };
        if list.has(&name) {
            return bad(&format!("duplicate ISA extension `{name}'"));
        }
        if name == "g" {
            list.add(&name, UNKNOWN, UNKNOWN);
        } else if major == UNKNOWN || minor == UNKNOWN {
            if name.starts_with('x') {
                warnings.push(format!(
                    "{file}: x ISA extension `{name}' must be set with the versions"
                ));
            } else if name != "zicsr" && name != "zifencei" {
                warnings.push(format!(
                    "{file}: cannot find default versions of the ISA extension `{name}'"
                ));
            }
        } else {
            list.add(&name, major, minor);
        }
    }
    list.close_and_check(xlen)
        .map_err(|e| format!("{file}: {e}"))?;
    Ok((xlen, list))
}

/// `riscv_update_subset_version`: keep the newest version.
fn update_version(from: &Subset, into: &mut Subset) {
    if from.major > into.major
        || (from.major == into.major && from.minor > into.minor)
        || into.major == UNKNOWN
    {
        into.major = from.major;
        into.minor = from.minor;
    }
}

/// `riscv_merge_arch_attr_info`.
fn merge_arch(
    in_arch: &str,
    out_arch: &str,
    file: &str,
    xlen: u32,
    warnings: &mut Vec<String>,
) -> Result<String, String> {
    let (xlen_in, ins) = parse_arch(in_arch, file, warnings)?;
    let (xlen_out, outs) = parse_arch(out_arch, "output", warnings)?;
    if xlen_in != xlen_out {
        return Err(format!(
            "{file}: ISA string of input ({in_arch}) doesn't match output ({out_arch})"
        ));
    }
    let mut merged = SubsetList::default();
    // Standard extensions: the base, then this fixed list (bfd's
    // `riscv_merge_std_ext`; `g` and `k` are not in it).
    let (Some(base_in), Some(base_out)) = (ins.0.first(), outs.0.first()) else {
        return Err(format!("{file}: empty ISA string"));
    };
    for (s, a) in [(base_in, in_arch), (base_out, out_arch)] {
        if s.name != "i" && s.name != "e" {
            return Err(format!(
                "{file}: corrupted ISA string '{a}'.  First letter should be 'i' or 'e' but got '{}'",
                s.name
            ));
        }
    }
    if base_in.name != base_out.name {
        return Err(format!(
            "{file}: mis-matched ISA string to merge '{}' and '{}'",
            base_in.name, base_out.name
        ));
    }
    let mut base = base_out.clone();
    update_version(base_in, &mut base);
    merged.add(&base.name, base.major, base.minor);
    for c in "mafdqlcbjtpvnh".chars() {
        let name = c.to_string();
        let mut chosen = match (ins.find(&name), outs.find(&name)) {
            (None, None) => continue,
            (Some(s), None) | (None, Some(s)) => s.clone(),
            (Some(i), Some(o)) => {
                let mut o = o.clone();
                update_version(i, &mut o);
                o
            }
        };
        chosen.name = name;
        merged.add(&chosen.name, chosen.major, chosen.minor);
    }
    // Multi-letter extensions: a sorted merge of what follows the standard
    // ones (`riscv_merge_multi_letter_ext`).
    let multi = |l: &SubsetList| -> Vec<Subset> {
        l.0.iter()
            .skip(1)
            .skip_while(|s| s.name.len() == 1 && ext_order(s.name.as_bytes()[0]) > 0)
            .cloned()
            .collect()
    };
    let (mi, mo) = (multi(&ins), multi(&outs));
    let (mut a, mut b) = (0, 0);
    while a < mi.len() && b < mo.len() {
        match compare(&mi[a].name, &mo[b].name) {
            std::cmp::Ordering::Less => {
                merged.add(&mi[a].name, mi[a].major, mi[a].minor);
                a += 1;
            }
            std::cmp::Ordering::Greater => {
                merged.add(&mo[b].name, mo[b].major, mo[b].minor);
                b += 1;
            }
            std::cmp::Ordering::Equal => {
                let mut o = mo[b].clone();
                update_version(&mi[a], &mut o);
                merged.add(&o.name, o.major, o.minor);
                a += 1;
                b += 1;
            }
        }
    }
    for s in mi[a..].iter().chain(&mo[b..]) {
        merged.add(&s.name, s.major, s.minor);
    }
    if xlen_in != xlen {
        return Err(format!(
            "{file}: unsupported XLEN ({xlen_in}), you might be using wrong emulation"
        ));
    }
    merged
        .close_and_check(xlen_in)
        .map_err(|e| format!("{file}: {e}"))?;
    Ok(merged.arch_string(xlen))
}

/// Merge one input's attributes into the output (`riscv_merge_attributes`).
fn merge_into(
    out: &mut Attrs,
    inp: &Attrs,
    file: &str,
    xlen: u32,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let tags: Vec<u64> = out
        .keys()
        .chain(inp.keys())
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut priv_done = false;
    for tag in tags {
        match tag {
            TAG_ARCH => {
                let in_arch = match inp.get(&TAG_ARCH) {
                    Some(Value::Str(s)) => Some(String::from_utf8_lossy(s).into_owned()),
                    _ => None,
                };
                let out_arch = match out.get(&TAG_ARCH) {
                    Some(Value::Str(s)) => Some(String::from_utf8_lossy(s).into_owned()),
                    _ => None,
                };
                match (in_arch, out_arch) {
                    (Some(i), None) => {
                        out.insert(TAG_ARCH, Value::Str(i.into_bytes()));
                    }
                    (Some(i), Some(o)) => {
                        let merged = merge_arch(&i, &o, file, xlen, warnings)?;
                        out.insert(TAG_ARCH, Value::Str(merged.into_bytes()));
                    }
                    (None, _) => {}
                }
            }
            TAG_PRIV_SPEC | TAG_PRIV_SPEC_MINOR | TAG_PRIV_SPEC_REVISION => {
                if priv_done {
                    continue;
                }
                priv_done = true;
                let (in_class, out_class) = (priv_class(inp), priv_class(out));
                let copy = if out_class == 0 {
                    true
                } else if in_class != 0 && in_class != out_class {
                    if in_class == 1 || out_class == 1 {
                        warnings.push(
                            "privileged spec version 1.9.1 can not be linked with other spec versions"
                                .to_owned(),
                        );
                    }
                    in_class > out_class
                } else {
                    false
                };
                if copy {
                    for t in [TAG_PRIV_SPEC, TAG_PRIV_SPEC_MINOR, TAG_PRIV_SPEC_REVISION] {
                        set_int(out, t, int(inp, t));
                    }
                }
            }
            TAG_UNALIGNED_ACCESS => {
                set_int(out, tag, int(out, tag) | int(inp, tag));
            }
            TAG_STACK_ALIGN => {
                let (o, i) = (int(out, tag), int(inp, tag));
                if o == 0 {
                    set_int(out, tag, i);
                } else if i != 0 && o != i {
                    return Err(format!(
                        "{file} use {i}-byte stack aligned but the output use {o}-byte stack aligned"
                    ));
                }
            }
            _ => {
                if out.contains_key(&tag) {
                    warnings.push(format!("output: unknown RISCV ABI object attribute {tag}"));
                } else {
                    warnings.push(format!("{file}: unknown RISCV ABI object attribute {tag}"));
                }
                if out.get(&tag) != inp.get(&tag) {
                    out.remove(&tag);
                }
            }
        }
    }
    Ok(())
}

/// Encode the merged attributes in ascending tag order, as bfd writes them.
fn encode(attrs: &Attrs) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    for (&tag, value) in attrs {
        match value {
            Value::Int(0) => continue,
            Value::Int(v) => {
                write_uleb(&mut body, tag);
                write_uleb(&mut body, *v);
            }
            Value::Str(s) => {
                write_uleb(&mut body, tag);
                body.extend_from_slice(s);
                body.push(0);
            }
        }
    }
    if body.is_empty() {
        return None;
    }
    let file_len = u32::try_from(body.len() + 5).ok()?;
    let sub_len = u32::try_from(4 + VENDOR.len() + 1)
        .ok()?
        .checked_add(file_len)?;
    let mut out = vec![b'A'];
    out.extend_from_slice(&sub_len.to_le_bytes());
    out.extend_from_slice(VENDOR);
    out.push(0);
    out.push(1); // Tag_File
    out.extend_from_slice(&file_len.to_le_bytes());
    out.extend_from_slice(&body);
    Some(out)
}

/// Merge the attribute sections of `inputs` (in link order). `None` when no
/// input has one, or when nothing but defaults remains.
fn merge_sections_data(
    inputs: &[(&str, &[u8])],
    xlen: u32,
    warnings: &mut Vec<String>,
) -> Result<Option<Vec<u8>>, String> {
    // The first input's attributes are copied (`_bfd_elf_copy_obj_attributes`)
    // and, like everything else, re-encoded on output.
    let Some(((first_name, first), rest)) = inputs.split_first() else {
        return Ok(None);
    };
    let mut out = parse(first, first_name)?;
    for (name, data) in rest {
        let inp = parse(data, name)?;
        merge_into(&mut out, &inp, name, xlen, warnings)?;
    }
    Ok(encode(&out))
}

/// Replace the merged `.riscv.attributes` section's contents with the bfd
/// merge of every input's section. Diagnostics bfd prints as warnings are
/// returned for the caller to report.
pub fn merge_into_output(
    input_objs: &[(String, ElfObject)],
    merged_sections: &mut [super::relocations::MergedSection],
    merged_map: &crate::common::fx_hash::FxHashMap<String, usize>,
) -> Result<Vec<String>, String> {
    let Some(&idx) = merged_map.get(SECTION) else {
        return Ok(Vec::new());
    };
    let inputs: Vec<(&str, &[u8])> = input_objs
        .iter()
        .filter_map(|(name, obj)| {
            let i = obj.sections.iter().position(|s| s.name == SECTION)?;
            Some((name.as_str(), obj.section_data[i].as_slice()))
        })
        .collect();
    let mut warnings = Vec::new();
    let data = merge_sections_data(&inputs, 64, &mut warnings)?;
    merged_sections[idx].data = data.unwrap_or_default();
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::Str(v.as_bytes().to_vec())
    }

    fn n(v: u64) -> Value {
        Value::Int(v)
    }

    fn section(list: &[(u64, Value)]) -> Vec<u8> {
        encode(&list.iter().cloned().collect()).expect("non-empty attributes")
    }

    fn link(inputs: &[&[(u64, Value)]]) -> Result<Attrs, String> {
        let data: Vec<Vec<u8>> = inputs.iter().map(|l| section(l)).collect();
        let named: Vec<(&str, &[u8])> = data.iter().map(|d| ("t.o", d.as_slice())).collect();
        let mut warnings = Vec::new();
        let out = merge_sections_data(&named, 64, &mut warnings)?;
        Ok(out
            .map(|d| parse(&d, "out").expect("own encoding parses"))
            .unwrap_or_default())
    }

    fn expect(list: &[(u64, Value)]) -> Attrs {
        list.iter().cloned().collect()
    }

    // The three inputs and every expected result below are GNU ld 2.47
    // `ld -r` outputs for objects GAS assembled from `.attribute` directives.
    fn c() -> Vec<(u64, Value)> {
        vec![
            (5, s("rv64i2p1_c2p0")),
            (6, n(1)),
            (8, n(1)),
            (10, n(11)),
            (20, n(7)),
        ]
    }

    fn d() -> Vec<(u64, Value)> {
        vec![
            (4, n(16)),
            (5, s("rv64i2p1_f2p2_d2p2_zicsr2p0")),
            (8, n(1)),
            (10, n(12)),
            (20, n(7)),
            (22, n(3)),
        ]
    }

    fn z() -> Vec<(u64, Value)> {
        vec![
            (5, s("rv64i2p0_m2p0_zmmul1p0_zba1p0_zbb1p0_sstc1p0_xfoo1p0")),
            (8, n(1)),
            (10, n(9)),
            (12, n(1)),
        ]
    }

    #[test]
    fn merges_like_gnu_ld() {
        assert_eq!(
            link(&[&c(), &d()]).unwrap(),
            expect(&[
                (4, n(16)),
                (5, s("rv64i2p1_f2p2_d2p2_c2p0_zicsr2p0")),
                (6, n(1)),
                (8, n(1)),
                (10, n(12)),
                (20, n(7)),
            ])
        );
        assert_eq!(
            link(&[&d(), &c(), &z()]).unwrap(),
            expect(&[
                (4, n(16)),
                (
                    5,
                    s(
                        "rv64i2p1_m2p0_f2p2_d2p2_c2p0_zicsr2p0_zmmul1p0_zba1p0_zbb1p0_sstc1p0_xfoo1p0"
                    ),
                ),
                (6, n(1)),
                (8, n(1)),
                (10, n(12)),
            ])
        );
        // 1.9.1 first, then upgraded to 1.11 (its revision goes with it).
        assert_eq!(
            link(&[&z(), &c()]).unwrap(),
            expect(&[
                (
                    5,
                    s("rv64i2p1_m2p0_c2p0_zmmul1p0_zba1p0_zbb1p0_sstc1p0_xfoo1p0")
                ),
                (6, n(1)),
                (8, n(1)),
                (10, n(11)),
            ])
        );
    }

    #[test]
    fn arch_merge_is_a_versioned_union() {
        let arch = |a: &str, b: &str| {
            let out = link(&[&[(5, s(a))], &[(5, s(b))]]).unwrap();
            match out.get(&TAG_ARCH) {
                Some(Value::Str(v)) => String::from_utf8(v.clone()).unwrap(),
                other => panic!("no arch: {other:?}"),
            }
        };
        // No implied subset is ever printed at link time.
        assert_eq!(
            arch("rv64i2p1_c2p0", "rv64i2p1_f2p2_d2p2"),
            "rv64i2p1_f2p2_d2p2_c2p0"
        );
        assert_eq!(arch("rv64i2p1_f2p2", "rv64i2p1_c2p0"), "rv64i2p1_f2p2_c2p0");
        assert_eq!(
            arch("rv64i2p1_m2p0_zmmul1p0", "rv64i2p1_m3p0"),
            "rv64i2p1_m3p0_zmmul1p0"
        );
        let v = "rv64i2p1_f2p0_d2p0_v1p0_zve32f1p0_zve32x1p0_zve64d1p0_zve64f1p0_zve64x1p0_zvl128b1p0_zvl32b1p0_zvl64b1p0";
        assert_eq!(arch(v, "rv64i2p1"), v);
        assert_eq!(
            arch("rv64i2p1_zca1p0_zcb1p0", "rv64i2p1_m2p0_zmmul1p0"),
            "rv64i2p1_m2p0_zmmul1p0_zca1p0_zcb1p0"
        );
    }

    #[test]
    fn conflicts_are_link_errors() {
        let stack = link(&[&[(4, n(16))], &[(4, n(8))]]).unwrap_err();
        assert!(stack.contains("8-byte stack aligned"), "{stack}");
        let xlen = link(&[&[(5, s("rv64i2p1"))], &[(5, s("rv32i2p1"))]]).unwrap_err();
        assert!(xlen.contains("doesn't match"), "{xlen}");
        let base = link(&[&[(5, s("rv64i2p1"))], &[(5, s("rv64e2p0"))]]).unwrap_err();
        assert!(base.contains("mis-matched ISA string"), "{base}");
        let zfinx =
            link(&[&[(5, s("rv64i2p1_zfinx1p0"))], &[(5, s("rv64i2p1_f2p2"))]]).unwrap_err();
        assert!(zfinx.contains("zfinx"), "{zfinx}");
        // `d` implies `f`: the closure feeds the conflict check.
        let implied =
            link(&[&[(5, s("rv64i2p1_zfinx1p0"))], &[(5, s("rv64i2p1_d2p2"))]]).unwrap_err();
        assert!(implied.contains("zfinx"), "{implied}");
    }

    #[test]
    fn a_lone_input_round_trips_and_order_is_canonical() {
        let one = c();
        assert_eq!(link(&[&one]).unwrap(), expect(&one));
        // Encoded in ascending tag order with ULEB/NTBS values.
        let bytes = section(&[(5, s("rv64i2p1")), (4, n(16))]);
        assert_eq!(bytes[0], b'A');
        let body = &bytes[1 + 4 + 6 + 1 + 4..];
        assert_eq!(body, b"\x04\x10\x05rv64i2p1\0");
    }

    #[test]
    fn canonical_subset_order() {
        let mut l = SubsetList::default();
        for x in [
            "xfoo", "sstc", "zbb", "zicsr", "c", "zba", "m", "i", "zmmul",
        ] {
            l.add(x, 1, 0);
        }
        let names: Vec<&str> = l.0.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "i", "m", "c", "zicsr", "zmmul", "zba", "zbb", "sstc", "xfoo"
            ]
        );
    }
}
