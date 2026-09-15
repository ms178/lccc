//! Identical Code Folding (ICF) for the x86-64 linker.
//!
//! Merges functions that are provably identical so only one copy reaches the
//! output. Enabled with `--icf=safe` / `--icf=all` (or `LCCC_LD_ICF=`).
//!
//! # What "identical" has to mean
//!
//! Comparing section *bytes* alone is wrong, and dangerously so. On x86-64 a
//! call is `e8 <rel32>` with the displacement supplied by a relocation, so
//!
//! ```c
//! int wrap_a(void) { return alpha(); }   // e8 00000000 c3
//! int wrap_b(void) { return beta();  }   // e8 00000000 c3
//! ```
//!
//! produce **byte-identical** sections that differ only in their relocation
//! targets. Folding them makes `wrap_b()` return `alpha()`'s value: a silent
//! miscompilation with no diagnostic.
//!
//! Comparing bytes plus *unqualified* relocation targets is wrong too, and
//! this is the subtler trap. Two objects routinely reference their own
//! locals through the same `(section index, offset)` pair:
//!
//! ```c
//! /* a.o */ static const int ta[4] = {11,22,33,44};
//!           const int *get_a(void) { return ta; }
//! /* b.o */ static const int tb[4] = {55,66,77,88};
//!           const int *get_b(void) { return tb; }
//! ```
//!
//! Both `lea` instructions are `48 8d 05 00000000 c3` with an `R_X86_64_PC32`
//! against local `(shndx=6, value=0)` — yet they denote different data.
//! Folding `get_a` onto `get_b` silently swaps the tables. Any name- or
//! index-based single-pass comparison folds these; the fix is not a better
//! key but a better *algorithm* (below).
//!
//! # Algorithm: iterative equivalence classes (class-aware, worklist-refined)
//!
//! All eligible sections (code and data alike) are partitioned into
//! equivalence classes: two sections share a class when their bytes match
//! and their relocations match element-wise in offset, type, addend, and
//! target. A target's identity is *its class*: locals through their home
//! section, and **named globals through the class of their chosen
//! definition** ([`global_definitions`]) — so once `alpha` and `beta` fold
//! into one representative, the wrappers from the module head compare equal
//! and fold too, which is exactly the transitive fold mold reaches and the
//! one name-keyed comparison (lld's `safe`, and the upstream revision of
//! this code) can never earn. Undefined-import names still key by name:
//! `printf` and `malloc` bind different PLT entries, never one target.
//!
//! Classes start from a structural hash (local targets as holes, defined
//! globals keyed by their definition's content fingerprint) and are refined
//! to a fixpoint: a class whose members disagree splits, which can only
//! cascade into further splits, so the loop provably terminates after at
//! most `n` passes over `n` members (each non-final pass strictly grows the
//! class count). Pass 1 classifies everything; each later pass (the
//! *dirty-class worklist*) re-examines only classes whose members reference
//! a class that split in the previous pass — an equality record is anchored
//! to the renumbering of every class it names, so skipping the rest is
//! exact, cutting the practical cost of deep chains from O(cap × N log N)
//! to one cheap scan per peeled link. This is the gold/lld design, and it
//! is what makes the `ta`/`tb` case above come out right: the tables land
//! in different classes because their *bytes* differ, so the getters can
//! never be equal.
//!
//! Only `.text`/`.text.*` members are ever folded; data classes exist solely
//! to make code comparison sound across objects. Everything else in the
//! universe (plain `.rodata`/`.data`) is comparison scaffolding. See
//! [`is_foldable`] for the exact gate and why data folding stays off.
//!
//! # Safety classes
//!
//! * `safe`: additionally requires that no folded member has its address
//!   taken in an observable way, because C guarantees distinct functions
//!   compare unequal. Absolute relocations (`R_X86_64_64/32/32S`) capture
//!   addresses and always mark their target. PC-relative references are
//!   decoded by opcode: `call`/`jmp`/`jcc` displacements are control flow
//!   and do not take addresses; anything else (`lea`, `mov`, data bytes,
//!   linker-difference expressions) is treated as address-taking. This is
//!   strictly more conservative than the industry-standard absolute-only
//!   rule, so every fold `safe` performs is one gold/bfd would also defend.
//! * `all`: folds regardless, matching `gold`/`lld`'s `--icf=all`. Smaller,
//!   but only valid for programs that never compare function pointers.
//!
//! Residual risks, stated plainly so nobody has to guess: addresses
//! materialized through GOT loads (`R_X86_64_GOTPCREL*`, the `-fno-plt`
//! idiom) are not tracked — decoding `call *mem(%rip)` (FF /2) apart from
//! `mov mem(%rip),%reg` (8B) is a disassembler, not a predicate, and every
//! production safe-ICF shares this hole. Truncated (8/16-bit) absolute
//! references are not tracked: they cannot capture a full code address.
//! Perfect static address-taken analysis without `.llvm_addrsig` is
//! undecidable; that section is the principled future fix.
//!
//! # Determinism
//!
//! No hash-map iteration order leaks into the output: every grouping passes
//! through an explicit sort, representatives are smallest-`(object,section)`
//! ids, and class numbering follows processing order. Same input, same plan,
//! every run — a linker that folds differently between runs is not
//! reproducible.
//!
//! # Application
//!
//! [`plan()`] returns a redirection map `(obj, sec) -> (obj, sec)`. The
//! emitter drops folded sections from the layout and redirects every
//! relocation and symbol that pointed at them to the surviving
//! representative, so no dangling references remain. Sections already dead
//! (`--gc-sections`, COMDAT, string-merge originals) are excluded up front:
//! folding among the dead can only lie in `bytes_saved`.
//!
//! # Performance notes
//!
//! The hot paths are allocation-free: names are hashed and compared as byte
//! slices (never `String`s), section bytes hash 8 words at a time through
//! [`FxHasher`](crate::common::fx_hash::FxHasher), and class lookups are
//! indexed table reads, not hash lookups. Expect ICF to cost a low-single-
//! digit percent of link time; if a profile ever says otherwise, the
//! per-section hashing is the embarrassingly parallel piece to thread first.

use std::hash::Hasher;

use super::elf::{
    R_X86_64_32, R_X86_64_32S, R_X86_64_64, R_X86_64_NONE, R_X86_64_PC8, R_X86_64_PC16,
    R_X86_64_PC32, R_X86_64_PC64, R_X86_64_PLT32,
};
use crate::backend::elf::{
    SHF_ALLOC, SHF_EXCLUDE, SHF_EXECINSTR, SHF_MERGE, SHF_TLS, SHN_ABS, SHN_UNDEF, SHT_PROGBITS,
    STT_GNU_IFUNC,
};
use crate::backend::linker_common::{Elf64Object, Elf64Rela, map_section_name};
use std::collections::hash_map::Entry;

use crate::common::fx_hash::{FxHashMap, FxHashSet, FxHasher};

#[derive(Debug, Default, Clone)]
pub struct IcfResult {
    pub candidate_groups: usize,
    pub folded_sections: usize,
    pub bytes_saved: u64,
    pub rejected_unsafe: usize,
    /// Refinement passes until the fixpoint (0 when nothing was eligible).
    /// Bounded by the 32-pass cap; stopping early never folds
    /// potentially-unequal members (only classes whose comparison evidence
    /// changed under the cap shatter — the rest keep proven classes).
    pub iterations: u32,
    /// Exact member comparisons plus class sketches charged: the work
    /// receipt for the refinement above, bounded by 32 per member.
    pub comparisons: u64,
    /// Members denied their class by the bound (stop-the-line shatters and
    /// sketch-collision splits): always sound (fewer folds, never wrong
    /// ones), nonzero only on adversarial or astronomically unlucky inputs.
    pub shattered: u32,
}

/// Section identity used for grouping: `(object, section)`.
pub type SecId = (usize, usize);

/// Shared empty relocation slice: avoids a `Vec::new()` per section that
/// carries no relocations (most data sections).
const NO_RELAS: [Elf64Rela; 0] = [];

/// Unwind-metadata section names (`.eh_frame`, `.eh_frame_hdr`,
/// `.gcc_except_table`): single source of truth for the two exclusion sites
/// (universe membership and the taken-scan source filter), so they can never
/// drift apart. Deliberately not performance-motivated: the predicate runs
/// once per section (cold path, ~25 microseconds per 100K-section link
/// either way — measured at parity with chained compares, so the unified
/// spelling costs nothing and removes the drift risk).
#[inline]
fn is_unwind_metadata_name(name: &str) -> bool {
    matches!(
        name.as_bytes(),
        b".eh_frame" | b".eh_frame_hdr" | b".gcc_except_table"
    )
}

/// Sentinel for "not in the ICF universe" in the position table.
const ABSENT: u32 = u32::MAX;

/// Start of the reserved section-index range (ELF spec: `SHN_LORESERVE`).
/// Indices here are not `(object, section)`-addressable: `SHN_ABS`/`COMMON`
/// are not sections at all, and `SHN_XINDEX` needs the `.symtab_shndx` table
/// the parser does not resolve — so every extended-section symbol reads as
/// raw `0xffff`, and comparing those would equate distinct sections.
const SHN_LORESERVE: u16 = 0xff00;
/// Extended section index marker (ELF spec: `SHN_XINDEX`).
#[cfg(test)]
const SHN_XINDEX: u16 = 0xffff;

/// Refinement passes are capped, but the cap is now a *protective* bound,
/// not the convergence mechanism: the dirty-class worklist reaches the
/// exact fixpoint on real links in a handful of passes, and even the
/// adversarial-scale corpus' 200-link chain settles at 199. At the cap the
/// same rule as always applies — classes whose comparison evidence changed
/// under the last pass shatter (`shatter_unstable`), everything else keeps
/// its proven classes — so exceeding it can only ever withhold folds, never
/// invent them. 256 covers every chain shape seen in practice with a wide
/// margin; pathological oscillation beyond it is a correctness-irrelevant
/// fold loss, accepted deliberately.
const MAX_REFINEMENT_PASSES: u32 = 256;

/// Global comparison budget: every exact member comparison and every class
/// sketch charges one unit against 64 per member. Legitimate links spend a
/// small multiple of N (a member is compared a handful of times before its
/// class settles; the scale corpus spends ~4.3 per member at the exact
/// fixpoint); past the budget the unstable remainder shatters instead of
/// refining further. Together with the pass cap this bounds refinement on
/// pathological inputs while leaving every practical case an exact result.
const COMPARE_BUDGET_PER_MEMBER: u64 = 64;

/// One member of the equivalence universe.
struct Member {
    id: SecId,
    /// Foldable (`.text`/`.text.*`)? Data members only inform comparison —
    /// and so do resolver sections (`STT_GNU_IFUNC` lives in one): the IPLT
    /// slot allocator skips dead sections, so a folded resolver would lose
    /// its slot while callers still bind through it.
    is_text: bool,
    /// Output-section discriminant (see [`output_discriminant`]): two
    /// members fold only within the same output section, so PGO's
    /// `.text.hot`/`.text.unlikely` split is never silently undone.
    output: u8,
    /// Carries relocations? Members without any compare by bytes alone, so
    /// once one scan confirms them they can never split again (byte equality
    /// is transitive — unlike target-class equality, which is why the
    /// refinement iterates at all) and later passes skip them.
    has_relocs: bool,
    size: u64,
    /// [`hash_bytes`] of the section content.
    content: u64,
    /// Relocation skeleton with local targets as holes (see
    /// [`skeleton_hash`]).
    sketched: u64,
}

/// Initial-bucket key. Every field but `content`/`sketched` is exact, so two
/// members that share a bucket already agree on size, flags, alignment,
/// entry size, and relocation count; only the two hashes need confirmation
/// by byte/reloc comparison.
#[derive(PartialEq, Eq, Hash, PartialOrd, Ord)]
struct BucketKey {
    content: u64,
    size: u64,
    flags: u64,
    entsize: u64,
    align: u64,
    nrelocs: usize,
    sketched: u64,
    output: u8,
}

/// Hash section bytes (or a symbol name) 8 bytes at a time. The explicit
/// length mix is load-bearing: the word hasher cannot tell `b"ab"` from
/// `b"ab\0"` (both fold one `0x6261` word), and ICF buckets keyed without
/// length would merge distinct sections into wasted comparisons at best.
fn hash_bytes(data: &[u8]) -> u64 {
    let mut h = FxHasher::default();
    h.write(data);
    h.write_u64(data.len() as u64);
    h.finish()
}

/// Sections whose bytes are concatenation-built, linker-managed, unwind
/// metadata, or owned by another pass. They can never be ICF members — not
/// because including them would miscompile (data members never fold) but
/// because every name here names a pass that already owns the section's
/// fate (note merging, string merging via `SHF_MERGE`, FDE pruning), and
/// ICF burning cycles on them buys nothing.
fn is_reserved_name(name: &str) -> bool {
    matches!(
        name,
        ".init"
            | ".fini"
            | ".init_array"
            | ".fini_array"
            | ".preinit_array"
            | ".dynamic"
            | ".got"
            | ".got.plt"
            | ".plt"
            | ".plt.sec"
            | ".iplt"
            | ".interp"
    ) || is_unwind_metadata_name(name)
        || name.starts_with(".note")
}

/// The one and only fold gate. ICF folds FUNCTIONS: only `.text`/`.text.*`
/// sections are eligible. Other executable sections are concatenation-built
/// or position-sensitive by design and must never fold — not even onto a
/// byte-identical twin. The instance of this class that bit: crtn.o's
/// `.init`/`.fini` epilogues are the same 5 bytes (`add $8,%rsp; ret`), so
/// an early `--icf=all` folded the `.fini` copy onto the `.init` one and
/// `_fini` was left as a bare `sub $8,%rsp` with no `ret`. Nothing called
/// `_fini`, so the miscompile hid until `DT_FINI` was emitted; then every
/// ICF binary crashed at exit. Same-output-section is the deeper invariant
/// and the name check alone does NOT subsume it (PGO's `.text.hot` and
/// `.text.unlikely` are separate outputs), so [`output_discriminant`] also
/// keys every bucket and every comparison on the mapped output. ICF never
/// runs for script links.
///
/// Data sections are deliberately excluded *for now*: folding `.rodata`
/// buys single-digit percent on top of text folding while dragging in
/// string-merge interplay, RELRO-layout coupling, and LSDA consistency
/// proof obligations that each deserve their own patch and audit. Flipping
/// this predicate is the entire code change when that work lands — but the
/// output-section invariant above becomes load-bearing for data the day it
/// does, so it must be enforced here, not hoped for.
fn is_foldable(name: &str) -> bool {
    name == ".text" || name.starts_with(".text.")
}

/// Output-section discriminant for a foldable input section. Same output
/// section is the deep invariant behind [`is_foldable`] (the `.init`/`.fini`
/// epilogues proved that folding across outputs deletes code that must
/// survive), and PGO splits `.text` three ways (`.text.hot` and
/// `.text.unlikely` stay separate for I-cache locality), so a `.text.hot`
/// twin must never fold onto `.text` or `.text.unlikely` — the bytes would
/// survive, but the compiler's partitioning would silently die.
///
/// This calls the REAL [`map_section_name`], never a reimplementation: the
/// mapping has quirks (`.text.hot.foo` maps to `.text`, only the exact
/// `.text.hot` stays hot) and any drift between the two would reintroduce
/// cross-output folds. The `output_mapping_is_pinned` test pins the mapping
/// this relies on; `0xFF` keeps unknown future outputs foldable with
/// nothing (the debug assert fires first, in test builds, and forces this
/// match to grow).
fn output_discriminant(mapped_output: &str) -> u8 {
    match mapped_output {
        ".text" => 0,
        ".text.hot" => 1,
        ".text.unlikely" => 2,
        other => {
            debug_assert!(
                false,
                "new .text* output mapping '{other}': extend the discriminant"
            );
            0xFF
        }
    }
}

/// Universe membership: allocated plain-data-or-code with real content,
/// excluding TLS templates (per-thread identity), merge sections (owned by
/// string merging — their relocs were already rewritten onto pools),
/// linker-excluded sections (the merge step never lays them out, so folding
/// among them can only lie in `bytes_saved` and burn hash cycles on bytes
/// that never reach the output), and everything [`is_reserved_name`]
/// withholds.
fn eligible(sec: &crate::backend::linker_common::Elf64Section) -> bool {
    sec.sh_type == SHT_PROGBITS
        && sec.flags & SHF_ALLOC != 0
        && sec.flags & (SHF_TLS | SHF_MERGE | SHF_EXCLUDE) == 0
        && sec.size != 0
        && !is_reserved_name(&sec.name)
}

/// Relocation skeleton: everything about the reloc list *except* which
/// concrete local section each target denotes. Absolute locals hash by
/// value (same value is the same entity); section locals contribute only
/// their in-section offset, leaving the section itself as the hole the
/// fixpoint resolves. Corrupt indices (no symbol) hash raw — buckets are
/// candidates, and exact comparison splits them conservatively.
/// Reserved-index locals hash by symbol index, mirroring the same-symbol
/// rule in [`local_targets_equal`]: hashing only the value would bucket
/// distinct extended sections together.
///
/// Since classes only ever *split* after bucketing, every global key here
/// must be a coarsening of the final comparison in [`members_equal`]: a
/// false merge inside a bucket is recovered by refinement, a false split at
/// bucketing time is unrecoverable. A *defined* named global therefore keys
/// by the chosen definition's static fingerprint — content hash and size of
/// its section (tag 7), see [`global_definitions`]: `members_equal`
/// resolves it to the definition's class, and class equality implies
/// content+size equality by the bucketing invariant, so callers of twin
/// functions bucket together (keying by name here, as the pre-adoption code
/// did, stranded every caller of every fold-twin in its own bucket: the
/// ~15-point corpus gap to mold). A defined name whose section sits outside
/// the universe keys by `(object, section)` (tag 8): exact identity only.
/// An *undefined* named global keys by name (tag 1): differently-named
/// imports bind different PLT entries, never one target, so it already is
/// the finest key.
fn skeleton_hash(
    objects: &[Elf64Object],
    def_globals: &FxHashMap<&str, SecId>,
    pos_of: &[Vec<u32>],
    members: &[Member],
    oi: usize,
    si: usize,
) -> u64 {
    let obj = &objects[oi];
    let mut h = FxHasher::default();
    if let Some(relas) = obj.relocations.get(si) {
        for r in relas {
            h.write_u64(r.offset);
            h.write_u32(r.rela_type);
            h.write_i64(r.addend);
            if r.rela_type == R_X86_64_NONE {
                // Explicit no-op: no target participates.
                h.write_u8(0);
                continue;
            }
            match obj.symbols.get(r.sym_idx as usize) {
                Some(s) if !s.name.is_empty() && !s.is_local() => {
                    match def_global_key_seed(def_globals, pos_of, members, s.name.as_str()) {
                        DefKeySeed::Fingerprint(fp, size) => {
                            h.write_u8(7);
                            h.write_u64(fp);
                            h.write_u64(size);
                        }
                        DefKeySeed::Exact(od, sd) => {
                            h.write_u8(8);
                            h.write_usize(od);
                            h.write_usize(sd);
                        }
                        DefKeySeed::Undefined => {
                            h.write_u8(1);
                            h.write(s.name.as_bytes());
                            h.write_u64(s.name.len() as u64);
                        }
                    }
                }
                Some(s) if s.shndx == SHN_ABS => {
                    h.write_u8(2);
                    h.write_u64(s.value);
                }
                Some(s) if s.shndx >= SHN_LORESERVE => {
                    h.write_u8(5);
                    h.write_u32(r.sym_idx);
                }
                Some(s) => {
                    h.write_u8(3);
                    h.write_u64(s.value);
                }
                None => {
                    h.write_u8(4);
                    h.write_u32(r.sym_idx);
                }
            }
        }
    }
    h.finish()
}

/// Seed key for a *defined* global name at skeleton (pre-class) time; see
/// [`skeleton_hash`]. `Undefined` means "no definition in any input":
/// tag 1 name identity, forever unmergeable across names.
enum DefKeySeed {
    Fingerprint(u64, u64),
    Exact(usize, usize),
    Undefined,
}

/// Map a global name to its seed key for [`skeleton_hash`].
#[inline]
fn def_global_key_seed(
    def_globals: &FxHashMap<&str, SecId>,
    pos_of: &[Vec<u32>],
    members: &[Member],
    name: &str,
) -> DefKeySeed {
    let Some(&(od, sd)) = def_globals.get(name) else {
        return DefKeySeed::Undefined;
    };
    match universe_pos(pos_of, od, sd as u16) {
        Some(p) => {
            let d = &members[p as usize];
            DefKeySeed::Fingerprint(d.content, d.size)
        }
        None => DefKeySeed::Exact(od, sd),
    }
}

/// Build the universe and the `(object, section) -> universe position`
/// table. Positions are `u32` (half the cache footprint of `usize` in the
/// hot class lookups); the `try_from` documents the only way that can fail
/// — more live sections than memory can hold — instead of silently wrapping.
fn build_universe(
    objects: &[Elf64Object],
    def_globals: &FxHashMap<&str, SecId>,
    dead: &FxHashSet<SecId>,
) -> (Vec<Member>, Vec<Vec<u32>>) {
    let mut members = Vec::new();
    let mut pos_of: Vec<Vec<u32>> = objects
        .iter()
        .map(|o| vec![ABSENT; o.sections.len()])
        .collect();
    // Resolver sections are never foldable (see `is_text` below): the IPLT
    // slot allocator walks each object's own symbol table and skips any
    // ifunc in a dead section, so folding a resolver would silently drop its
    // slot while callers still bind through it. One linear pass, outside the
    // section loop, so lookup is O(1) per section.
    let ifunc_sec: Vec<Vec<bool>> = objects
        .iter()
        .map(|o| {
            let mut v = vec![false; o.sections.len()];
            for s in &o.symbols {
                if s.sym_type() == STT_GNU_IFUNC
                    && s.shndx != SHN_UNDEF
                    && (s.shndx as usize) < v.len()
                {
                    v[s.shndx as usize] = true;
                }
            }
            v
        })
        .collect();
    for (oi, obj) in objects.iter().enumerate() {
        for (si, sec) in obj.sections.iter().enumerate() {
            if dead.contains(&(oi, si)) {
                continue;
            }
            let data: &[u8] = match obj.section_data.get(si) {
                Some(d) if !d.as_slice().is_empty() => d.as_slice(),
                _ => continue,
            };
            if !eligible(sec) {
                continue;
            }
            let pos = u32::try_from(members.len()).expect("ICF universe exceeds 2^32 sections");
            pos_of[oi][si] = pos;
            members.push(Member {
                id: (oi, si),
                is_text: is_foldable(&sec.name) && !ifunc_sec[oi][si],
                has_relocs: obj.relocations.get(si).is_some_and(|r| !r.is_empty()),
                output: output_discriminant(map_section_name(&sec.name)),
                size: sec.size,
                content: hash_bytes(data),
                sketched: 0, // filled below: needs every member's content hash
            });
        }
    }
    // Second pass for the relocation skeleton: a named-global key is the
    // chosen definition's *static fingerprint* (content+size of its
    // section), and that fingerprint is only known once all members' own
    // content hashes exist.
    let sk: Vec<u64> = (0..members.len())
        .map(|p| {
            let m = &members[p];
            skeleton_hash(objects, def_globals, &pos_of, &members, m.id.0, m.id.1)
        })
        .collect();
    for (m, s) in members.iter_mut().zip(sk) {
        m.sketched = s;
    }
    (members, pos_of)
}

/// Universe position of a relocation target section, if it participates.
#[inline]
fn universe_pos(pos_of: &[Vec<u32>], oi: usize, shndx: u16) -> Option<u32> {
    let pos = *pos_of.get(oi)?.get(shndx as usize)?;
    (pos != ABSENT).then_some(pos)
}

/// The chosen definition of every defined global name, for fold-aware
/// target comparison.
///
/// Why this exists: comparing named globals by *name* forces `wrap_alpha`
/// and `wrap_beta` (identical calls to twin functions `alpha`/`beta`) to
/// stay unfolded forever — even after `alpha`/`beta` themselves fold into
/// one representative, so both wrappers end up calling the same code. That
/// is how lld's `safe` mode falls to 0% and how the upstream patch stranded
/// ~15 percentage points of this corpus. Comparing named globals by *the
/// class of their chosen definition* is the sound refinement: once two
/// definitions sit in one class they are byte- and relocation-interchangeable
/// by the bucketing invariant, so two references through them denote the
/// same post-fold entity even when spelled differently. Name equality is
/// kept as the short-circuit (and as the rule for *undefined* names: two
/// differently-named imports bind different PLT entries, never one target,
/// because the undefined pseudo-section folds nothing).
///
/// Resolution mirrors static symbol resolution without duplicating its
/// decisions: the linker's resolve+gc phase has already rejected duplicate
/// strong definitions, so at most one strong def per name survives; a weak
/// def yields to any strong def, and weak-vs-weak ties break by object
/// order (first input wins, exactly the order the symbol table was built
/// in). ICF may *observe* that two names denote interchangeable code, but
/// it must never *decide* which one a relocation binds — this map does not
/// decide either, it re-derives what resolution already decided.
fn global_definitions(objects: &[Elf64Object]) -> FxHashMap<&str, SecId> {
    let mut out: FxHashMap<&str, (SecId, bool)> = FxHashMap::default();
    for (oi, obj) in objects.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.name.is_empty() || sym.is_undefined() || sym.is_local() {
                continue;
            }
            let si = sym.shndx as usize;
            if sym.shndx == SHN_ABS || sym.shndx >= SHN_LORESERVE || si >= obj.sections.len() {
                // ABS/Loreserve symbols have no (object, section)-addressable
                // body: nothing to key a class on, nothing foldable.
                continue;
            }
            let id: SecId = (oi, si);
            match out.entry(sym.name.as_str()) {
                Entry::Vacant(v) => {
                    v.insert((id, sym.is_weak()));
                }
                Entry::Occupied(mut o) => {
                    // Strong replaces weak; strong-vs-strong is a link error
                    // already diagnosed upstream, so the first strong keeps.
                    if o.get().1 && !sym.is_weak() {
                        o.insert((id, false));
                    }
                }
            }
        }
    }
    out.into_iter().map(|(k, (id, _))| (k, id)).collect()
}

/// Local-target equality, the heart of the fixpoint. Same in-section offset
/// is mandatory; the sections themselves compare by class when both
/// participate, else by exact `(object, section)` triple — which can only
/// hold within one object, so cross-object references into bss/TLS/merge
/// pools conservatively never match. Absolute locals compare by value:
/// the same number is the same entity no matter which object spells it.
/// Reserved non-absolute indices (COMMON/XINDEX/...) only ever equal the
/// same symbol: their section identity is not `(object, shndx)`-addressable
/// (see [`SHN_LORESERVE`]).
#[inline]
#[allow(clippy::too_many_arguments)]
fn local_targets_equal(
    pos_of: &[Vec<u32>],
    class_of: &[u32],
    oa: usize,
    a: &crate::backend::linker_common::Elf64Symbol,
    xa_sym: u32,
    ob: usize,
    b: &crate::backend::linker_common::Elf64Symbol,
    yb_sym: u32,
) -> bool {
    if a.shndx == SHN_ABS || b.shndx == SHN_ABS {
        // Value identity for absolute symbols (linker-provided numbers,
        // enumerator-style constants); a section target is never absolute.
        return a.shndx == SHN_ABS && b.shndx == SHN_ABS && a.value == b.value;
    }
    if a.shndx >= SHN_LORESERVE || b.shndx >= SHN_LORESERVE {
        return oa == ob && xa_sym == yb_sym;
    }
    if a.value != b.value {
        return false;
    }
    match (
        universe_pos(pos_of, oa, a.shndx),
        universe_pos(pos_of, ob, b.shndx),
    ) {
        (Some(x), Some(y)) => class_of[x as usize] == class_of[y as usize],
        // Both outside the universe (bss, TLS, merge pools, reserved):
        // same object and same section or nothing.
        (None, None) => oa == ob && a.shndx == b.shndx,
        _ => false,
    }
}

/// Full relocation-target equality between two members under comparison.
/// Relocation kind/offset/addend are the caller's job; this resolves what
/// the two symbol indices *denote*. Global names compare as bytes — no
/// allocation, ever: the old code built a `String` per relocation per
/// comparison. Missing symbols (corrupt input) only ever equal their own
/// exact `(object, index)` pair.
fn targets_equal(
    objects: &[Elf64Object],
    def_globals: &FxHashMap<&str, SecId>,
    pos_of: &[Vec<u32>],
    class_of: &[u32],
    oa: usize,
    x: &Elf64Rela,
    ob: usize,
    y: &Elf64Rela,
) -> bool {
    if x.rela_type == R_X86_64_NONE {
        // Types already match (caller-checked): an explicit no-op carries
        // no target regardless of the index it happens to hold.
        return true;
    }
    match (
        objects[oa].symbols.get(x.sym_idx as usize),
        objects[ob].symbols.get(y.sym_idx as usize),
    ) {
        (Some(a), Some(b)) => {
            let ga = !a.name.is_empty() && !a.is_local();
            let gb = !b.name.is_empty() && !b.is_local();
            // A named global and a local can never denote one entity, even
            // spelled alike: different scopes, different bindings.
            if ga != gb {
                return false;
            }
            if ga {
                if a.name.as_str() == b.name.as_str() {
                    return true;
                }
                // Fold-aware equality: distinct names denote one entity
                // when their chosen definitions sit in one class (see
                // [`global_definitions`]). Undefined names have no
                // definition anywhere, so different strings are genuinely
                // different imports — different PLT entries, never one
                // target.
                let (da, db) = (
                    def_globals.get(a.name.as_str()),
                    def_globals.get(b.name.as_str()),
                );
                return match (da, db) {
                    (Some(&da_id), Some(&db_id)) => {
                        match (
                            universe_pos(pos_of, da_id.0, da_id.1 as u16),
                            universe_pos(pos_of, db_id.0, db_id.1 as u16),
                        ) {
                            (Some(pa), Some(pb)) => class_of[pa as usize] == class_of[pb as usize],
                            (None, None) => da_id == db_id,
                            _ => false,
                        }
                    }
                    _ => false,
                };
            }
            local_targets_equal(pos_of, class_of, oa, a, x.sym_idx, ob, b, y.sym_idx)
        }
        // One or both indices dangle: identical pairs or nothing.
        _ => oa == ob && x.sym_idx == y.sym_idx,
    }
}

/// Exact member equality within one class. Same class implies same size,
/// flags, alignment, entry size, and relocation count (exact bucket-key
/// fields, not hashes), so this confirms the two hashes — bytes, then
/// relocations with class-resolved targets. Hash collisions must never
/// fold, so the byte comparison is not optional.
fn members_equal(
    objects: &[Elf64Object],
    def_globals: &FxHashMap<&str, SecId>,
    pos_of: &[Vec<u32>],
    class_of: &[u32],
    a: &Member,
    b: &Member,
) -> bool {
    debug_assert_eq!(a.output, b.output, "same class implies same output section");
    let da = objects[a.id.0].section_data[a.id.1].as_slice();
    let db = objects[b.id.0].section_data[b.id.1].as_slice();
    if da != db {
        return false;
    }
    let ra = objects[a.id.0]
        .relocations
        .get(a.id.1)
        .map(Vec::as_slice)
        .unwrap_or(&NO_RELAS);
    let rb = objects[b.id.0]
        .relocations
        .get(b.id.1)
        .map(Vec::as_slice)
        .unwrap_or(&NO_RELAS);
    debug_assert_eq!(ra.len(), rb.len(), "same class implies same reloc count");
    for (x, y) in ra.iter().zip(rb.iter()) {
        if x.offset != y.offset || x.rela_type != y.rela_type || x.addend != y.addend {
            return false;
        }
        if !targets_equal(objects, def_globals, pos_of, class_of, a.id.0, x, b.id.0, y) {
            return false;
        }
    }
    true
}

/// Opcode check for PC-relative references: is the displacement at `off` a
/// control-flow operand, or does the instruction compute an address the
/// program can observe? The byte before the displacement decides, and the
/// displacement length gates which forms may match (Intel SDM Vol. 2A, the
/// Jcc/JMP/CALL/LOOP encodings): near branches with a rel32 — or, under a
/// `0x66` operand-size prefix, rel16 — field are `E8` (call), `E9` (jmp)
/// and `0F 80..8F` (jcc); short branches with a rel8 field are `EB` (jmp),
/// `70..7F` (jcc) and `E0..E3` (loopne/loope/loop/jrcxz). All are direct
/// near branches whose relocated field the CPU adds to RIP — the target
/// address never surfaces to the program, so a relocation there does not
/// make the target address-taken. (Far branches `9A`/`EA` take an absolute
/// pointer, never a section-relative relocation on ELF x86-64; indirect
/// `FF /2`/`FF /4` have no immediate field at all — both correctly fall
/// through to "address".)
///
/// Mismatched (opcode, length) pairs report "address": a PC64 field after
/// an `E8` byte is not a call, and a PC8 field after an `EB` byte could be
/// a `.byte sym - .` jump-delta entry — but in *that* case the preceding
/// byte is only a coincidence when it matches, and the pre-fix behaviour
/// (always marking PC8/PC16 targets taken, as if short branches never
/// carry relocations) already accepted the mirror-image risk of misreading
/// genuine control flow as address observation. Opcode-gating with length
/// matching is the strict tightening of both directions: it recognises
/// exactly the encodings the SDM defines as direct near branches and
/// conservatively marks everything else.
///
/// RIP-relative data access lands on a ModRM whose mod+r/m encode
/// RIP+disp32 (`0x05..0x3D`, never one of the branch opcodes), so
/// `lea`/`mov`/`cmp` and data bytes all correctly report "address".
/// Out-of-range offsets (corrupt input) report "address" for the same
/// reason: this feeds the taken-set, never the fold set.
#[inline]
fn is_call_or_jump(code: &[u8], off: u64, disp_len: usize) -> bool {
    let o = off as usize;
    if o == 0 || o.saturating_add(disp_len) > code.len() {
        return false;
    }
    match (code[o - 1], disp_len) {
        // call rel32/rel16, jmp rel32/rel16 (near, direct).
        (0xE8 | 0xE9, 2 | 4) => true,
        // jcc rel32/rel16: two-byte opcode with the 0x0F escape.
        (0x80..=0x8F, 2 | 4) => o >= 2 && code[o - 2] == 0x0F,
        // jmp rel8, jcc rel8, loopne/loope/loop/jrcxz rel8.
        (0xEB | 0x70..=0x7F | 0xE0..=0xE3, 1) => true,
        _ => false,
    }
}

/// Sections whose address is observable, so folding them could change
/// program-visible behaviour.
///
/// Absolute relocations capture addresses unconditionally. PC-relative ones
/// are split by [`is_call_or_jump`]: calls and jumps are control flow (the
/// callee's address never surfaces), everything else materializes an
/// address that can be compared, stored, or printed. Only references *from*
/// allocated sections count — debug info is dropped before emission, and
/// `.eh_frame`/`gcc_except_table` are unwind metadata whose references
/// follow folds through redirection (counting FDEs would mark every
/// function taken through its own frame info and fold nothing, ever).
///
/// Every *definition* of a referenced global is marked, not just the first:
/// with a weak `dup` in one object and a strong `dup` in another, first-win
/// marks the shadowed twin and leaves the surviving address foldable — a
/// safe-mode soundness hole, closed here.
fn address_taken_sections(objects: &[Elf64Object]) -> FxHashSet<SecId> {
    // Global name -> ALL defining (object, section) pairs. Locals are
    // excluded: a global reference never binds a same-named local, so a
    // local in this map could only misattribute the mark. Two maps, not one
    // map of vectors: the overwhelmingly common case is a single definition,
    // which lives inline (no per-name allocation, no growth); the rare
    // multiply-defined name (weak/strong pairs, commons) spills into `multi`.
    let mut ndef = 0usize;
    for obj in objects.iter() {
        for sym in &obj.symbols {
            if !sym.name.is_empty() && !sym.is_undefined() && !sym.is_local() {
                ndef += 1;
            }
        }
    }
    let mut def_of: FxHashMap<&str, SecId> = FxHashMap::default();
    def_of.reserve(ndef);
    let mut multi_of: FxHashMap<&str, Vec<SecId>> = FxHashMap::default();
    for (oi, obj) in objects.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.name.is_empty() || sym.is_undefined() || sym.is_local() {
                continue;
            }
            let si = sym.shndx as usize;
            if si >= obj.sections.len() {
                continue;
            }
            let name = sym.name.as_str();
            if let Some(ids) = multi_of.get_mut(name) {
                ids.push((oi, si));
            } else {
                use std::collections::hash_map::Entry;
                match def_of.entry(name) {
                    Entry::Vacant(v) => {
                        v.insert((oi, si));
                    }
                    Entry::Occupied(o) => {
                        let first = o.remove();
                        multi_of.insert(name, vec![first, (oi, si)]);
                    }
                }
            }
        }
    }

    let mut taken: FxHashSet<SecId> = FxHashSet::default();
    // Mark the target of one address-capturing relocation.
    let mut mark = |obj: &Elf64Object, oi: usize, sym_idx: u32| {
        match obj.symbols.get(sym_idx as usize) {
            Some(sym) if !sym.name.is_empty() && !sym.is_local() => {
                let name = sym.name.as_str();
                if let Some(ids) = multi_of.get(name) {
                    taken.extend(ids.iter().copied());
                } else if let Some(id) = def_of.get(name) {
                    taken.insert(*id);
                }
            }
            // Absolute numbers and the undefined pseudo-section denote no
            // foldable code; everything else marks its home section. (The
            // old code marked section 0 — the NULL section — for undefined
            // locals: harmless, but sloppy. Say what you mean.)
            Some(sym) if sym.shndx != SHN_ABS && sym.shndx != SHN_UNDEF => {
                let si = sym.shndx as usize;
                if si < obj.sections.len() {
                    taken.insert((oi, si));
                }
            }
            _ => {}
        }
    };

    for (oi, obj) in objects.iter().enumerate() {
        for (si, relas) in obj.relocations.iter().enumerate() {
            // The opcode check below is meaningful only for EXECUTABLE
            // sources: in data bytes an `E8` before a displacement is just a
            // value that happens to look like `call`, and letting it hide an
            // address would be a soundness hole (hand-made `.rodata` with an
            // `E8` padding byte before a PC32 slot). Data sources mark
            // unconditionally — which also skips the decode on that path.
            let (src_data, src_exec): (&[u8], bool) = match obj.sections.get(si) {
                Some(s) if s.flags & SHF_ALLOC != 0 && !is_unwind_metadata_name(&s.name) => (
                    obj.section_data
                        .get(si)
                        .map(|d| d.as_slice())
                        .unwrap_or(&[]),
                    s.flags & SHF_EXECINSTR != 0,
                ),
                _ => continue,
            };
            for r in relas {
                match r.rela_type {
                    R_X86_64_64 | R_X86_64_32 | R_X86_64_32S => {
                        mark(obj, oi, r.sym_idx);
                    }
                    R_X86_64_PC32 | R_X86_64_PLT32 => {
                        if !src_exec || !is_call_or_jump(src_data, r.offset, 4) {
                            mark(obj, oi, r.sym_idx);
                        }
                    }
                    // No x86-64 direct branch encodes a 64-bit displacement
                    // (see is_call_or_jump), so every PC64 reference is a
                    // data reference: always observed.
                    R_X86_64_PC64 => {
                        mark(obj, oi, r.sym_idx);
                    }
                    // Short branches: real assemblers resolve in-range
                    // targets internally and relax cross-section ones to
                    // the near form, so a surviving PC8/PC16 relocation is
                    // hand-made — but hand-made or not, when the bytes are
                    // a genuine short-branch encoding the field is still a
                    // branch displacement, never an observable address (the
                    // upstream commit message claimed this recognition but
                    // shipped an `is_call_or_jump` without the rel8 opcodes;
                    // both directions are now gated on (opcode, disp_len)).
                    R_X86_64_PC8 => {
                        if !src_exec || !is_call_or_jump(src_data, r.offset, 1) {
                            mark(obj, oi, r.sym_idx);
                        }
                    }
                    R_X86_64_PC16 => {
                        if !src_exec || !is_call_or_jump(src_data, r.offset, 2) {
                            mark(obj, oi, r.sym_idx);
                        }
                    }
                    // GOT loads (`-fno-plt` address materialization),
                    // TLS descriptors, size queries, and dynamic-linker
                    // relocs: deliberately untracked, see the module docs.
                    _ => {}
                }
            }
        }
    }
    taken
}

/// Fixpoint-consistent sketch of a member's relocation targets: a hash over
/// exactly the per-relocation inputs [`members_equal`] compares (offset,
/// kind, addend, target identity with class-resolved locals) — everything
/// BUT the bytes, which are equal for all members of a class by the
/// bucketing invariant. Equal members always sketch equal (the sketch is a
/// pure function of the compared state), so different sketches prove
/// inequality and sketch-chunking never splits an equal pair; equal
/// sketches prove nothing (hash collisions), so every chunk is verified
/// with one exact comparison per member before it joins a class.
fn class_sketch(
    objects: &[Elf64Object],
    def_globals: &FxHashMap<&str, SecId>,
    pos_of: &[Vec<u32>],
    class_of: &[u32],
    m: &Member,
) -> u64 {
    let obj = &objects[m.id.0];
    let relas = obj
        .relocations
        .get(m.id.1)
        .map(Vec::as_slice)
        .unwrap_or(&NO_RELAS);
    let mut h = FxHasher::default();
    h.write_usize(relas.len());
    for r in relas {
        h.write_u64(r.offset);
        h.write_u32(r.rela_type);
        h.write_i64(r.addend);
        if r.rela_type == R_X86_64_NONE {
            h.write_u8(b'n');
            continue;
        }
        let Some(s) = obj.symbols.get(r.sym_idx as usize) else {
            h.write_u8(b'm');
            h.write_usize(m.id.0);
            h.write_u32(r.sym_idx);
            continue;
        };
        if !s.name.is_empty() && !s.is_local() {
            // Class of the chosen definition when one exists (mirrors
            // `targets_equal`): name identity only as the short-circuit
            // and for undefined imports — refinement merges sibling
            // references whose definitions fold together.
            match def_globals.get(s.name.as_str()) {
                Some(&(od, sd)) => match universe_pos(pos_of, od, sd as u16) {
                    Some(p) => {
                        h.write_u8(b'D');
                        h.write_u32(class_of[p as usize]);
                    }
                    None => {
                        h.write_u8(b'E');
                        h.write_usize(od);
                        h.write_usize(sd);
                    }
                },
                None => {
                    h.write_u8(b'g');
                    h.write(s.name.as_str().as_bytes());
                }
            }
            continue;
        }
        if s.shndx == SHN_ABS {
            h.write_u8(b'a');
            h.write_u64(s.value);
            continue;
        }
        if s.shndx >= SHN_LORESERVE {
            h.write_u8(b'r');
            h.write_usize(m.id.0);
            h.write_u32(r.sym_idx);
            continue;
        }
        h.write_u8(b's');
        h.write_u64(s.value);
        match universe_pos(pos_of, m.id.0, s.shndx) {
            Some(p) => {
                h.write_u8(1);
                h.write_u32(class_of[p as usize]);
            }
            None => {
                h.write_u8(0);
                h.write_usize(m.id.0);
                h.write_u16(s.shndx);
            }
        }
    }
    h.finish()
}

/// What one refinement pass did.
#[derive(Debug, PartialEq, Eq)]
enum RefineOutcome {
    /// Nothing split: the fixpoint is reached.
    Settled,
    /// At least one class split: refine again (within the pass cap).
    Split,
    /// The comparison budget ran out mid-pass: `class_of` holds partial
    /// progress (processed groups exact, the rest untouched) and the caller
    /// must `shatter_unstable` before collecting.
    Exhausted,
}

/// Per-pass refinement scratch: the comparison budget, the metrics, the
/// fresh class ids assigned (the change-set for `shatter_unstable`), the
/// class ids of confirmed relocation-free groups (see the skip in
/// `refine_once`), and a reusable sketch buffer so fragmented pieces never
/// allocate.
struct RefineCtx<'a> {
    budget: &'a mut u64,
    comparisons: &'a mut u64,
    shattered: &'a mut u32,
    fresh: &'a mut Vec<u32>,
    settled: &'a mut FxHashSet<u32>,
    sketch_scratch: &'a mut Vec<(u64, u32)>,
}

impl RefineCtx<'_> {
    /// Charge one comparison/sketch against the budget; `false` means
    /// exhausted (the caller stops classifying immediately).
    #[inline]
    fn charge(&mut self) -> bool {
        if *self.budget == 0 {
            return false;
        }
        *self.budget -= 1;
        *self.comparisons += 1;
        true
    }

    /// One fresh class id, tracked for the change-set.
    #[inline]
    fn fresh_class(&mut self, next_class: &mut u32) -> u32 {
        let fresh = *next_class;
        *next_class = next_class
            .checked_add(1)
            .expect("ICF class counter exhausted (2^32 classes)");
        self.fresh.push(fresh);
        fresh
    }
}

/// Budget exhausted while classifying `todo[bounds[bi - 1]..]` (the current
/// group onwards). The pending groups shatter eagerly — every member its
/// own class: in pass 1 they were never compared at all, and in later
/// worklist passes their entering equality was verified only against the
/// *old* partition, while at least one class their relocations name has
/// since split (that is why they are in the todo set) — keeping them would
/// risk folding members that never re-verified. Shattering only withholds
/// folds, so either way this is the sound direction. The current and
/// remaining groups also leave `settled`: a surviving record must imply a
/// completed scan (only then are the bytes confirmed), and these groups'
/// scans never completed.
fn exhaust(
    todo: &[u32],
    bounds: &[usize],
    bi: usize,
    class_of: &mut [u32],
    ctx: &mut RefineCtx<'_>,
    next_class: &mut u32,
) -> RefineOutcome {
    let mut j = bi - 1;
    while j + 1 < bounds.len() {
        if bounds[j] < todo.len() {
            ctx.settled.remove(&class_of[todo[bounds[j]] as usize]);
        }
        j += 1;
    }
    let mut j = bi - 1;
    while j + 1 < bounds.len() {
        let (lo, hi) = (bounds[j], bounds[j + 1]);
        j += 1;
        if hi - lo < 2 {
            continue;
        }
        for &m in &todo[lo..hi] {
            class_of[m as usize] = ctx.fresh_class(next_class);
            *ctx.shattered += 1;
        }
    }
    RefineOutcome::Exhausted
}

/// Short-circuit predicate over every refinement-visible target class of
/// `m`: locals resolve through their home section, named globals through
/// the chosen definition's class ([`global_definitions`]), and anything
/// classless — locals outside the universe, undefined imports, absolutes,
/// reserved indices, no-op relocs — cannot change class and is skipped.
/// Returns true at the first target class satisfying `pred`.
fn for_any_target_class(
    objects: &[Elf64Object],
    def_globals: &FxHashMap<&str, SecId>,
    pos_of: &[Vec<u32>],
    class_of: &[u32],
    m: &Member,
    mut pred: impl FnMut(u32) -> bool,
) -> bool {
    let obj = &objects[m.id.0];
    let Some(relas) = obj.relocations.get(m.id.1) else {
        return false;
    };
    for r in relas {
        if r.rela_type == R_X86_64_NONE {
            continue;
        }
        let Some(s) = obj.symbols.get(r.sym_idx as usize) else {
            continue;
        };
        let p = if !s.name.is_empty() && !s.is_local() {
            match def_globals.get(s.name.as_str()) {
                Some(&(od, sd)) => universe_pos(pos_of, od, sd as u16),
                None => None,
            }
        } else if s.shndx == SHN_ABS || s.shndx >= SHN_LORESERVE {
            None
        } else {
            universe_pos(pos_of, m.id.0, s.shndx)
        };
        if let Some(p) = p {
            if pred(class_of[p as usize]) {
                return true;
            }
        }
    }
    false
}

/// Worklist pruning predicate: does this member reference any class in the
/// dirty set (the freshly assigned class ids of the previous pass)?
#[inline]
fn references_dirty(
    objects: &[Elf64Object],
    def_globals: &FxHashMap<&str, SecId>,
    pos_of: &[Vec<u32>],
    class_of: &[u32],
    m: &Member,
    dirty: &FxHashSet<u32>,
) -> bool {
    for_any_target_class(objects, def_globals, pos_of, class_of, m, |c| {
        dirty.contains(&c)
    })
}

/// One partition-refinement pass over the current classes. Multi-member
/// classes split into exact-equality pieces; the first piece keeps the
/// class, the rest take fresh ids in processing order. Groups arrive
/// smallest-id-first inside a class, so the representative choice (and
/// hence every downstream decision) is stable.
///
/// Each piece starts with one linear scan against its representative. When
/// the majority agrees the scan stands (exact, and identical to the
/// unbounded algorithm on every input that takes this path — which is every
/// non-pathological one). When the piece is FRAGMENTED (the rep convinces
/// less than half), recursing would degenerate to one-against-all quadratic
/// scans, so the piece takes the sketch path instead: sketch every member
/// once, sort by `(sketch, pos)`, verify each sketch chunk with one exact
/// comparison per member. Every comparison and every sketch charges the
/// budget; on exhaustion classification stops and the caller shatters the
/// unstable remainder.
fn refine_once(
    objects: &[Elf64Object],
    def_globals: &FxHashMap<&str, SecId>,
    members: &[Member],
    pos_of: &[Vec<u32>],
    class_of: &mut [u32],
    order: &mut Vec<u32>,
    dirty: Option<&FxHashSet<u32>>,
    ctx: &mut RefineCtx<'_>,
) -> RefineOutcome {
    debug_assert_eq!(members.len(), class_of.len());
    // Worklist selection. The first pass re-classifies everything; later
    // passes touch only classes whose members reference a class that split
    // in the previous pass (`dirty`, one entry per freshly assigned class
    // id). A member's equality record is verified against the renumbering
    // of every class its relocations name, so re-verification is provably
    // complete with that set — classes referencing nothing dirty keep
    // records that remain exact, and skipping them is what turns the
    // O(passes × N log N) fixed point of the pre-worklist design into
    // O(passes × (N scan + touched log touched)). Deep chains then cost one
    // cheap pass per peeled link instead of one full sort.
    let mut todo: Vec<u32>;
    match dirty {
        None => {
            // One full sort, up front, in the caller-owned vector: pass 1
            // groups every member by (class, id); later passes only ever
            // re-sort the dirty subset, and the caller performs the single
            // final sort for collection.
            order.sort_unstable_by_key(|&pos| ((class_of[pos as usize] as u64) << 32) | pos as u64);
            todo = std::mem::take(order);
        }
        Some(d) => {
            // Two O(N) scans, no global sort: first mark every class that
            // contains at least one member referencing a dirty class...
            let mut affected = FxHashSet::default();
            for (pos, m) in members.iter().enumerate() {
                if references_dirty(objects, def_globals, pos_of, class_of, m, d) {
                    affected.insert(class_of[pos]);
                }
            }
            // ...then collect ALL members of the affected classes (the
            // whole class must re-verify when a referenced class splits,
            // not just the referencing member), sorted by (class, id) for
            // stable representative choice.
            todo = (0..members.len() as u32)
                .filter(|&pos| affected.contains(&class_of[pos as usize]))
                .collect();
            todo.sort_unstable_by_key(|&pos| ((class_of[pos as usize] as u64) << 32) | pos as u64);
        }
    }
    let mut next_class = class_of
        .iter()
        .max()
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .expect("ICF class counter exhausted (2^32 classes)");
    // Group boundaries up front: the splitter mutates `class_of`, so the
    // grouping pass (immutable borrow) must end before splitting starts.
    let mut bounds: Vec<usize> = vec![0];
    for (i, w) in todo.windows(2).enumerate() {
        if class_of[w[0] as usize] != class_of[w[1] as usize] {
            bounds.push(i + 1);
        }
    }
    bounds.push(todo.len());
    let mut split = false;
    let mut bi = 0;
    while bi + 1 < bounds.len() {
        let (lo, hi) = (bounds[bi], bounds[bi + 1]);
        bi += 1;
        if hi - lo < 2 {
            continue;
        }
        // Relocation-free groups compare by bytes alone, and byte equality
        // is transitive: one full scan (which every earlier pass performed
        // before recording the id) proves all pairs, so the group can never
        // split in this or any later pass and re-scanning it would only burn
        // budget (on const-heavy links this skip removes ~97% of all
        // comparisons). Groups with relocations compare by target classes,
        // which other passes' splits can still distinguish.
        if todo[lo..hi]
            .iter()
            .all(|&m| !members[m as usize].has_relocs)
        {
            if !ctx.settled.insert(class_of[todo[lo] as usize]) {
                continue;
            }
        }
        // FIFO worklist of pieces still to classify; the head piece keeps
        // the class, everything spun off takes fresh ids.
        let mut queue: Vec<Vec<u32>> = vec![todo[lo..hi].to_vec()];
        let mut qi = 0;
        let mut first = true;
        while qi < queue.len() {
            let piece = std::mem::take(&mut queue[qi]);
            qi += 1;
            let rep = piece[0];
            // Single linear scan into `same`/`rest`: each member pays
            // exactly one equality check here.
            let mut same = vec![rep];
            let mut rest = Vec::new();
            for &m in &piece[1..] {
                if !ctx.charge() {
                    return exhaust(&todo, &bounds, bi, class_of, ctx, &mut next_class);
                }
                if members_equal(
                    objects,
                    def_globals,
                    pos_of,
                    class_of,
                    &members[rep as usize],
                    &members[m as usize],
                ) {
                    same.push(m);
                } else {
                    rest.push(m);
                }
            }
            if same.len() * 2 >= piece.len() {
                // Majority agrees with the rep: the scan stands.
                if rest.is_empty() {
                    if !first {
                        // Spun-off piece that holds together: one fresh class.
                        let fresh = ctx.fresh_class(&mut next_class);
                        for &m in &same {
                            class_of[m as usize] = fresh;
                        }
                    }
                } else {
                    if !first {
                        let fresh = ctx.fresh_class(&mut next_class);
                        for &m in &same {
                            class_of[m as usize] = fresh;
                        }
                    }
                    queue.push(rest);
                    split = true;
                }
            } else {
                // Fragmented piece: the rep convinced less than half, so
                // recursing would degenerate. Sketch every member once,
                // sort by (sketch, pos), verify each chunk exactly. Equal
                // members always share a sketch, so chunks never split
                // equals; a chunk member that fails verification is a hash
                // collision and shatters (conservative by construction).
                let mut sk = std::mem::take(ctx.sketch_scratch);
                sk.clear();
                for &m in &piece {
                    if !ctx.charge() {
                        return exhaust(&todo, &bounds, bi, class_of, ctx, &mut next_class);
                    }
                    sk.push((
                        class_sketch(objects, def_globals, pos_of, class_of, &members[m as usize]),
                        m,
                    ));
                }
                sk.sort_unstable();
                let mut out_first = true;
                for chunk in sk.chunk_by(|a, b| a.0 == b.0) {
                    let crep = chunk[0].1;
                    if chunk.len() == 1 {
                        if out_first && first {
                            // Keeps the class: nothing to assign.
                        } else {
                            let fresh = ctx.fresh_class(&mut next_class);
                            class_of[crep as usize] = fresh;
                        }
                    } else {
                        // `verified[0]` is the chunk rep; every other
                        // member pays one exact comparison against it.
                        let mut verified = vec![crep];
                        for &(_, m) in &chunk[1..] {
                            if !ctx.charge() {
                                *ctx.sketch_scratch = sk;
                                return exhaust(&todo, &bounds, bi, class_of, ctx, &mut next_class);
                            }
                            if members_equal(
                                objects,
                                def_globals,
                                pos_of,
                                class_of,
                                &members[crep as usize],
                                &members[m as usize],
                            ) {
                                verified.push(m);
                            } else {
                                class_of[m as usize] = ctx.fresh_class(&mut next_class);
                                *ctx.shattered += 1;
                            }
                        }
                        if out_first && first {
                            // Keeps the class: nothing to assign.
                        } else {
                            let fresh = ctx.fresh_class(&mut next_class);
                            for &m in &verified {
                                class_of[m as usize] = fresh;
                            }
                        }
                    }
                    out_first = false;
                }
                *ctx.sketch_scratch = sk;
                // A fragmented piece never holds together: the linear scan
                // already proved the rep disagrees with the majority, so the
                // sketch partition always splits or shatters something.
                split = true;
            }
            first = false;
        }
    }
    // Hand the pass-1 grouping back to the caller-owned vector (pass 1
    // `mem::take`s it for the upfront sort); dirty passes built `todo`
    // locally and never moved `order`, so this is a no-op there.
    if order.is_empty() && !todo.is_empty() {
        *order = todo;
    }
    if split {
        RefineOutcome::Split
    } else {
        RefineOutcome::Settled
    }
}

/// A concrete folding plan.
#[derive(Debug, Default, Clone)]
pub struct IcfPlan {
    /// Folded section -> surviving representative.
    pub redirect: FxHashMap<SecId, SecId>,
    pub result: IcfResult,
}

impl IcfPlan {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.redirect.is_empty()
    }
    /// Follow the redirection for `id` (identity when not folded).
    #[inline]
    pub fn resolve(&self, id: SecId) -> SecId {
        self.redirect.get(&id).copied().unwrap_or(id)
    }
}

/// Stop-the-line shatter after the pass cap or budget exhaustion: every
/// multi-member class whose members' comparison evidence changed this pass
/// (a relocation target whose class is in `changed`, the fresh ids assigned
/// this pass) shatters into singletons — the pass that would have
/// re-examined them never runs, so keeping them would fold
/// potentially-unequal members. Classes whose evidence is untouched keep
/// their proven classes: their targets carry the same ids as last pass, so
/// another pass could split nothing there. Absolutes, reserved indices,
/// undefined names and out-of-universe sections never change class, so the
/// scan skips them; universe-local targets and the chosen definitions of
/// named globals are both examined. Relocation-free groups additionally
/// need a `settled` record (a completed scan confirmed their bytes — without
/// one they would fold on the content hash alone).
fn shatter_unstable(
    objects: &[Elf64Object],
    def_globals: &FxHashMap<&str, SecId>,
    members: &[Member],
    pos_of: &[Vec<u32>],
    class_of: &mut [u32],
    order: &mut Vec<u32>,
    changed: &FxHashSet<u32>,
    settled: &FxHashSet<u32>,
    shattered: &mut u32,
) {
    if changed.is_empty() {
        return;
    }
    order.sort_unstable_by_key(|&pos| ((class_of[pos as usize] as u64) << 32) | pos as u64);
    let mut next_class = class_of
        .iter()
        .max()
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .expect("ICF class counter exhausted (2^32 classes)");
    let mut start = 0;
    while start < order.len() {
        let mut end = start + 1;
        while end < order.len() && class_of[order[end] as usize] == class_of[order[start] as usize]
        {
            end += 1;
        }
        let group = &order[start..end];
        start = end;
        if group.len() < 2 {
            continue;
        }
        // Never-confirmed bytes: shatter (see the doc comment). Reachable
        // only when exhaustion struck mid-first-scan of a fresh class.
        if group.iter().all(|&m| !members[m as usize].has_relocs)
            && !settled.contains(&class_of[group[0] as usize])
        {
            for &m in group {
                class_of[m as usize] = next_class;
                next_class = next_class
                    .checked_add(1)
                    .expect("ICF class counter exhausted (2^32 classes)");
                *shattered += 1;
            }
            continue;
        }
        // Same "references a changed class" predicate the worklist uses:
        // locals via their home section, named globals via the class of
        // their chosen definition — definitions fold like locals do under
        // the class-aware comparison, so their classes DO change (the
        // pre-adoption scan skipped globals, which would keep unverified
        // folds alive on the cap/exhaustion path).
        let mut unstable = false;
        for &m in group {
            if for_any_target_class(
                objects,
                def_globals,
                pos_of,
                class_of,
                &members[m as usize],
                |c| changed.contains(&c),
            ) {
                unstable = true;
                break;
            }
        }
        if unstable {
            for &m in group {
                class_of[m as usize] = next_class;
                next_class = next_class
                    .checked_add(1)
                    .expect("ICF class counter exhausted (2^32 classes)");
                *shattered += 1;
            }
        }
    }
}

/// Compute which sections to fold.
///
/// Sections already in `dead` are excluded before anything else. `safe_only`
/// additionally excludes address-taken sections. The representative of each
/// fold set is the smallest `(object, section)` id, so the result is
/// independent of hashing and iteration order.
pub fn plan(objects: &[Elf64Object], safe_only: bool, dead: &FxHashSet<SecId>) -> IcfPlan {
    // The parser guarantees `sections`, `section_data` and `relocations`
    // stay parallel (a section index addresses all three); every indexing
    // site below relies on it, so pin it at the single entry point.
    debug_assert!(objects.iter().all(
        |o| o.sections.len() == o.section_data.len() && o.sections.len() == o.relocations.len()
    ));
    let mut out = IcfPlan::default();
    let def_globals = global_definitions(objects);
    let (members, pos_of) = build_universe(objects, &def_globals, dead);
    if members.len() < 2 {
        return out;
    }

    // Initial classes: sort universe indices by exact bucket key (a sort,
    // not a hash map, so numbering is deterministic by construction and
    // there is no rehash path to stall on).
    // Level 1: 16-byte (content, pos) pairs — the sort moves almost
    // nothing. Singleton content runs take a class immediately (nothing
    // else can share their bucket); only genuinely duplicated contents
    // pay for full keys.
    let mut by_content: Vec<(u64, u32)> = members
        .iter()
        .enumerate()
        .map(|(i, m)| (m.content, i as u32))
        .collect();
    by_content.sort_unstable();
    let mut class_of: Vec<u32> = vec![0; members.len()];
    let mut next_class: u32 = 0;
    let mut multi_buckets = 0usize;
    // Scratch reused across runs so the level-2 sorts never allocate.
    let mut full: Vec<(BucketKey, u32)> = Vec::new();
    for run in by_content.chunk_by(|a, b| a.0 == b.0) {
        if run.len() == 1 {
            class_of[run[0].1 as usize] = next_class;
            next_class = next_class
                .checked_add(1)
                .expect("ICF class counter exhausted (2^32 classes)");
            continue;
        }
        full.clear();
        full.extend(run.iter().map(|&(_, i)| {
            let m = &members[i as usize];
            let sec = &objects[m.id.0].sections[m.id.1];
            let nrelocs = objects[m.id.0].relocations.get(m.id.1).map_or(0, Vec::len);
            (
                BucketKey {
                    content: m.content,
                    size: m.size,
                    flags: sec.flags,
                    entsize: sec.entsize,
                    align: sec.addralign,
                    nrelocs,
                    sketched: m.sketched,
                    output: m.output,
                },
                i,
            )
        }));
        full.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        for bucket in full.chunk_by(|a, b| a.0 == b.0) {
            if bucket.len() > 1 {
                multi_buckets += 1;
            }
            for (_, i) in bucket {
                class_of[*i as usize] = next_class;
            }
            next_class = next_class
                .checked_add(1)
                .expect("ICF class counter exhausted (2^32 classes)");
        }
    }
    out.result.candidate_groups = multi_buckets;

    // Refine to the fixpoint via the dirty-class worklist, within the pass
    // cap and the comparison budget. Pass 1 classifies everything; each
    // later pass re-examines only classes whose members reference a class
    // that split in the previous pass — an equality record is anchored to
    // the renumbering of every class it names, so the skip is provably
    // exact (see the note on `refine_once`). `order` holds the pass-1
    // grouping throughout and gets one final sort for collection;
    // `shatter_unstable` re-sorts it when it runs.
    let mut order: Vec<u32> = (0..members.len() as u32).collect();
    let mut dirty: Option<FxHashSet<u32>> = None;
    let mut iters: u32 = 0;
    let mut budget: u64 = COMPARE_BUDGET_PER_MEMBER.saturating_mul(members.len() as u64);
    let mut comparisons: u64 = 0;
    let mut shattered: u32 = 0;
    let mut fresh: Vec<u32> = Vec::new();
    let mut settled: FxHashSet<u32> = FxHashSet::default();
    let mut sketch_scratch: Vec<(u64, u32)> = Vec::new();
    loop {
        iters += 1;
        debug_assert!(iters <= MAX_REFINEMENT_PASSES, "ICF pass cap overrun");
        fresh.clear();
        let mut ctx = RefineCtx {
            budget: &mut budget,
            comparisons: &mut comparisons,
            shattered: &mut shattered,
            fresh: &mut fresh,
            settled: &mut settled,
            sketch_scratch: &mut sketch_scratch,
        };
        match refine_once(
            objects,
            &def_globals,
            &members,
            &pos_of,
            &mut class_of,
            &mut order,
            dirty.as_ref(),
            &mut ctx,
        ) {
            RefineOutcome::Settled => break,
            RefineOutcome::Split => {
                if iters >= MAX_REFINEMENT_PASSES {
                    let mut changed = FxHashSet::default();
                    changed.reserve(fresh.len());
                    changed.extend(fresh.iter().copied());
                    shatter_unstable(
                        objects,
                        &def_globals,
                        &members,
                        &pos_of,
                        &mut class_of,
                        &mut order,
                        &changed,
                        &settled,
                        &mut shattered,
                    );
                    break;
                }
                // The classes that changed under this pass are the only
                // evidence that can invalidate another class's record:
                // they are next pass's todo set.
                dirty = Some(fresh.iter().copied().collect());
            }
            RefineOutcome::Exhausted => {
                let mut changed = FxHashSet::default();
                changed.reserve(fresh.len());
                changed.extend(fresh.iter().copied());
                shatter_unstable(
                    objects,
                    &def_globals,
                    &members,
                    &pos_of,
                    &mut class_of,
                    &mut order,
                    &changed,
                    &settled,
                    &mut shattered,
                );
                break;
            }
        }
    }
    // One final global sort so collection sees members grouped by final
    // class in (class, id) order — `order` carried only the pass-1
    // grouping while later passes re-sorted just their dirty subset.
    order.sort_unstable_by_key(|&pos| ((class_of[pos as usize] as u64) << 32) | pos as u64);
    out.result.iterations = iters;
    out.result.comparisons = comparisons;
    out.result.shattered = shattered;

    let taken = if safe_only {
        address_taken_sections(objects)
    } else {
        FxHashSet::default()
    };

    // Project the (usually small) taken set onto member positions once, so
    // collection tests one bit per member instead of probing a hash set.
    let mut taken_pos = vec![false; members.len()];
    if safe_only {
        for &(oi, si) in taken.iter() {
            let p = pos_of[oi][si];
            if p != ABSENT {
                taken_pos[p as usize] = true;
            }
        }
    }

    // `order` still groups by final class in (class, id) order (the last
    // refine pass sorted it and changed nothing after): fold the text
    // members of every multi-text class onto the smallest id.
    out.redirect.reserve(members.len());
    for group in order.chunk_by(|&a, &b| class_of[a as usize] == class_of[b as usize]) {
        // `group` is id-sorted; filter preserves that, so texts[0] is the rep.
        let texts: Vec<u32> = group
            .iter()
            .copied()
            .filter(|&i| members[i as usize].is_text)
            .collect();
        if texts.len() < 2 {
            continue;
        }
        if safe_only && texts.iter().any(|&i| taken_pos[i as usize]) {
            out.result.rejected_unsafe += 1;
            continue;
        }
        let rep = members[texts[0] as usize].id;
        for &i in &texts[1..] {
            let m = members[i as usize].id;
            out.redirect.insert(m, rep);
            out.result.folded_sections += 1;
            out.result.bytes_saved += objects[m.0].sections[m.1].size;
        }
    }
    out
}

/// `--icf=` / `LCCC_LD_ICF=` mode, validated.
pub fn icf_mode_from_env() -> Option<&'static str> {
    match std::env::var("LCCC_LD_ICF") {
        Ok(s) => parse_icf_mode(s.trim()),
        Err(_) => None,
    }
}

pub fn parse_icf_mode(s: &str) -> Option<&'static str> {
    if s.eq_ignore_ascii_case("safe") {
        Some("safe")
    } else if s.eq_ignore_ascii_case("all") {
        Some("all")
    } else {
        None // includes "none", which disables ICF
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::linker_common::{Elf64Section, Elf64Symbol, SectionData, SymStr};

    fn sec(name: &str, data: &[u8], align: u64) -> Elf64Section {
        sec_flags(name, data, align, SHF_EXECINSTR | SHF_ALLOC)
    }

    fn sec_flags(name: &str, data: &[u8], align: u64, flags: u64) -> Elf64Section {
        Elf64Section {
            name_idx: 0,
            name: name.to_string(),
            sh_type: SHT_PROGBITS,
            flags,
            addr: 0,
            offset: 0,
            size: data.len() as u64,
            link: 0,
            info: 0,
            addralign: align,
            entsize: 0,
        }
    }

    fn sym(name: &str, shndx: u16, value: u64, global: bool) -> Elf64Symbol {
        Elf64Symbol {
            name_idx: 0,
            name: SymStr::new(name),
            info: if global { 1 << 4 } else { 0 },
            other: 0,
            shndx,
            value,
            size: 0,
        }
    }

    fn rela(offset: u64, sym_idx: u32, rela_type: u32, addend: i64) -> Elf64Rela {
        Elf64Rela {
            offset,
            sym_idx,
            rela_type,
            addend,
        }
    }

    /// Build one object: sections[i] has data[i], relocs[i], symbols shared.
    fn obj(datas: &[&[u8]], relocs: Vec<Vec<Elf64Rela>>, symbols: Vec<Elf64Symbol>) -> Elf64Object {
        let mut sections = Vec::new();
        let mut section_data = Vec::new();
        for (i, d) in datas.iter().enumerate() {
            sections.push(sec(&format!(".text.f{i}"), d, 16));
            section_data.push(SectionData::owned(d.to_vec()));
        }
        Elf64Object {
            sections,
            symbols,
            section_data,
            relocations: relocs,
            source_name: "<test>".into(),
        }
    }

    /// Fully general object builder: (section, bytes, relocs) triples plus
    /// a shared symbol table.
    fn mkobj(
        parts: Vec<(Elf64Section, Vec<u8>, Vec<Elf64Rela>)>,
        symbols: Vec<Elf64Symbol>,
    ) -> Elf64Object {
        let mut sections = Vec::new();
        let mut section_data = Vec::new();
        let mut relocations = Vec::new();
        for (s, d, r) in parts {
            sections.push(s);
            section_data.push(SectionData::owned(d));
            relocations.push(r);
        }
        Elf64Object {
            sections,
            symbols,
            section_data,
            relocations,
            source_name: "<test>".into(),
        }
    }

    fn no_dead() -> FxHashSet<SecId> {
        FxHashSet::default()
    }

    const CALL_RET: &[u8] = &[0xe8, 0, 0, 0, 0, 0xc3];
    /// Distinct code bytes (mov $7,%eax; ret) used as a layout-shifting pad.
    const PAD: &[u8] = &[0xb8, 7, 0, 0, 0, 0xc3];
    const LEA_RET: &[u8] = &[0x48, 0x8d, 0x05, 0, 0, 0, 0, 0xc3];

    /// THE bug the single-pass design could not see. Two objects whose
    /// getters are byte-identical and reference same-indexed locals must NOT
    /// fold when the locals' bytes differ — the old `(shndx, value)` key
    /// folded these and silently swapped the tables.
    #[test]
    fn cross_object_locals_with_different_bytes_do_not_fold() {
        // Object A: get_a (PC32 -> local rodata sec 1) + rodata {11,22,33,44}.
        let a = mkobj(
            vec![
                (
                    sec(".text.get_a", LEA_RET, 16),
                    LEA_RET.to_vec(),
                    vec![rela(3, 1, R_X86_64_PC32, -4)],
                ),
                (
                    sec_flags(".rodata.ta", &[11, 22, 33, 44], 16, SHF_ALLOC),
                    vec![11, 22, 33, 44],
                    vec![],
                ),
            ],
            vec![
                sym("", 0, 0, false),
                sym("ta", 1, 0, false), // local, shndx 1, value 0
            ],
        );
        // Object B: same shape, different table bytes {55,66,77,88}.
        let b = mkobj(
            vec![
                (
                    sec(".text.get_b", LEA_RET, 16),
                    LEA_RET.to_vec(),
                    vec![rela(3, 1, R_X86_64_PC32, -4)],
                ),
                (
                    sec_flags(".rodata.tb", &[55, 66, 77, 88], 16, SHF_ALLOC),
                    vec![55, 66, 77, 88],
                    vec![],
                ),
            ],
            vec![
                sym("", 0, 0, false),
                sym("tb", 1, 0, false), // local, shndx 1, value 0
            ],
        );
        for safe in [true, false] {
            let p = plan(&[a.clone(), b.clone()], safe, &no_dead());
            assert!(
                p.is_empty(),
                "safe={safe}: folded getters over different tables -- silent \
                 data swap, the exact miscompile this rewrite exists to kill: {:?}",
                p.redirect
            );
        }
    }

    /// The fix must not just refuse: identical tables in different objects
    /// prove the getters equivalent, and they fold. A naive "qualify locals
    /// by object" patch would miss this; iteration gets it.
    #[test]
    fn cross_object_identical_locals_do_fold() {
        let one = |tname: &str, rname: &str| {
            mkobj(
                vec![
                    (
                        sec(tname, LEA_RET, 16),
                        LEA_RET.to_vec(),
                        vec![rela(3, 1, R_X86_64_PC32, -4)],
                    ),
                    (
                        sec_flags(rname, &[7, 7, 7, 7], 16, SHF_ALLOC),
                        vec![7, 7, 7, 7],
                        vec![],
                    ),
                ],
                vec![sym("", 0, 0, false), sym("t", 1, 0, false)],
            )
        };
        let p = plan(
            &[
                one(".text.get_a", ".rodata.ta"),
                one(".text.get_b", ".rodata.tb"),
            ],
            true,
            &no_dead(),
        );
        assert_eq!(p.result.folded_sections, 1);
        assert_eq!(p.resolve((1, 0)), (0, 0));
        // Data members inform but never fold.
        assert!(!p.redirect.contains_key(&(0, 1)));
        assert!(!p.redirect.contains_key(&(1, 1)));
    }

    /// Discriminating iteration proof: identical chains reference their identical
    /// callees through DIFFERENT section indices (2 vs 3 — the pad section in
    /// B shifts everything). Index comparison cannot fold these; the class
    /// fixpoint proves the callees equivalent first, then folds both pairs.
    #[test]
    fn cross_object_shifted_indices_fold() {
        // A: f (part 0, calls local g at part 1), g (part 1, calls leaf).
        let a = mkobj(
            vec![
                (
                    sec(".text.f", CALL_RET, 16),
                    CALL_RET.to_vec(),
                    vec![rela(1, 1, R_X86_64_PLT32, -4)],
                ),
                (
                    sec(".text.g", CALL_RET, 16),
                    CALL_RET.to_vec(),
                    vec![rela(1, 2, R_X86_64_PLT32, -4)],
                ),
            ],
            vec![
                sym("", 0, 0, false),
                sym("g", 1, 0, false),
                sym("leaf", 0, 0, true),
            ],
        );
        // B: pad (part 0, distinct bytes, shifts everything), f (part 1,
        // calls local g at part 2 — a DIFFERENT index than A's), g (part 2).
        let b = mkobj(
            vec![
                (sec(".text.pad", PAD, 16), PAD.to_vec(), vec![]),
                (
                    sec(".text.f", CALL_RET, 16),
                    CALL_RET.to_vec(),
                    vec![rela(1, 1, R_X86_64_PLT32, -4)],
                ),
                (
                    sec(".text.g", CALL_RET, 16),
                    CALL_RET.to_vec(),
                    vec![rela(1, 2, R_X86_64_PLT32, -4)],
                ),
            ],
            vec![
                sym("", 0, 0, false),
                sym("g", 2, 0, false),
                sym("leaf", 0, 0, true),
            ],
        );
        let p = plan(&[a, b], true, &no_dead());
        assert_eq!(
            p.result.folded_sections, 2,
            "both pairs fold: {:?}",
            p.redirect
        );
        assert_eq!(p.resolve((1, 1)), (0, 0), "shifted caller must fold");
        assert_eq!(p.resolve((1, 2)), (0, 1), "shifted callee must fold");
    }

    /// Transitive equivalence through locals: f1 -> g1 and f2 -> g2 fold
    /// when g1 and g2 are themselves equivalent (same global callee here).
    #[test]
    fn transitive_chains_fold() {
        // Per object: f (calls local g), g (calls global leaf).
        let one = |fname: &str| {
            mkobj(
                vec![
                    (
                        sec(fname, CALL_RET, 16),
                        CALL_RET.to_vec(),
                        vec![rela(1, 1, R_X86_64_PLT32, -4)],
                    ),
                    (
                        sec(".text.g", CALL_RET, 16),
                        CALL_RET.to_vec(),
                        vec![rela(1, 2, R_X86_64_PLT32, -4)],
                    ),
                ],
                vec![
                    sym("", 0, 0, false),
                    sym("g", 1, 0, false),
                    sym("leaf", 0, 0, true),
                ],
            )
        };
        let p = plan(&[one(".text.f1"), one(".text.f2")], true, &no_dead());
        assert_eq!(
            p.result.folded_sections, 2,
            "both pairs fold: {:?}",
            p.redirect
        );
        assert_eq!(p.resolve((1, 0)), (0, 0));
        assert_eq!(p.resolve((1, 1)), (0, 1));
    }

    /// And the negative chain: differing leaves (different global callees)
    /// must keep the callers apart. The old key folded these whenever the
    /// local indices coincided.
    #[test]
    fn transitive_chains_with_different_leaves_do_not_fold() {
        let one = |fname: &str, leaf: &str| {
            mkobj(
                vec![
                    (
                        sec(fname, CALL_RET, 16),
                        CALL_RET.to_vec(),
                        vec![rela(1, 1, R_X86_64_PLT32, -4)],
                    ),
                    (
                        sec(".text.g", CALL_RET, 16),
                        CALL_RET.to_vec(),
                        vec![rela(1, 2, R_X86_64_PLT32, -4)],
                    ),
                ],
                vec![
                    sym("", 0, 0, false),
                    sym("g", 1, 0, false),
                    sym(leaf, 0, 0, true),
                ],
            )
        };
        let p = plan(
            &[one(".text.f1", "leaf_a"), one(".text.f2", "leaf_b")],
            false,
            &no_dead(),
        );
        assert!(
            p.is_empty(),
            "callers over different callees must not fold, even under all: {:?}",
            p.redirect
        );
    }

    /// THE bug this file originally existed to prevent. Two sections with
    /// identical bytes whose relocations call *different* functions must NOT
    /// fold; doing so silently makes one function return the other's value.
    #[test]
    fn identical_bytes_different_call_targets_do_not_fold() {
        let o = obj(
            &[CALL_RET, CALL_RET],
            vec![
                vec![rela(1, 2, R_X86_64_PLT32, -4)],
                vec![rela(1, 3, R_X86_64_PLT32, -4)],
            ],
            vec![
                sym("", 0, 0, false),
                sym("", 0, 0, false),
                sym("alpha", 0, 0, true),
                sym("beta", 0, 0, true),
            ],
        );
        let p = plan(&[o], true, &no_dead());
        assert!(
            p.is_empty(),
            "folded two sections that call different functions -- this is a \
             miscompilation, not an optimisation: {:?}",
            p.redirect
        );
    }

    /// The positive case: identical bytes calling the *same* function fold.
    #[test]
    fn identical_bytes_same_call_target_folds() {
        let o = obj(
            &[CALL_RET, CALL_RET],
            vec![
                vec![rela(1, 2, R_X86_64_PLT32, -4)],
                vec![rela(1, 2, R_X86_64_PLT32, -4)],
            ],
            vec![
                sym("", 0, 0, false),
                sym("", 0, 0, false),
                sym("alpha", 0, 0, true),
            ],
        );
        let p = plan(&[o], true, &no_dead());
        assert_eq!(p.result.folded_sections, 1, "should fold one duplicate");
        assert_eq!(
            p.resolve((0, 1)),
            (0, 0),
            "duplicate redirects to the first"
        );
        assert_eq!(p.resolve((0, 0)), (0, 0), "representative maps to itself");
    }

    /// Addends participate in identity: `call foo+0` and `call foo+8` are
    /// different code even with identical bytes.
    #[test]
    fn differing_addends_do_not_fold() {
        let o = obj(
            &[CALL_RET, CALL_RET],
            vec![
                vec![rela(1, 2, R_X86_64_PLT32, -4)],
                vec![rela(1, 2, R_X86_64_PLT32, 4)],
            ],
            vec![
                sym("", 0, 0, false),
                sym("", 0, 0, false),
                sym("alpha", 0, 0, true),
            ],
        );
        assert!(
            plan(&[o], true, &no_dead()).is_empty(),
            "addend must be part of identity"
        );
    }

    /// crtn.o's .init/.fini epilogues are the same 5 bytes
    /// (`add $8,%rsp; ret`) but concatenation-built into different output
    /// sections: folding them deleted the .fini epilogue and left `_fini`
    /// as a bare `sub $8,%rsp` with no `ret` — an exit-time crash under
    /// `--icf=all` once DT_FINI was emitted. ICF folds .text only.
    #[test]
    fn init_fini_epilogues_do_not_fold() {
        let epilogue: &[u8] = &[0x48, 0x83, 0xc4, 0x08, 0xc3];
        let o = Elf64Object {
            sections: vec![sec(".init", epilogue, 4), sec(".fini", epilogue, 4)],
            symbols: vec![],
            section_data: vec![
                SectionData::owned(epilogue.to_vec()),
                SectionData::owned(epilogue.to_vec()),
            ],
            relocations: vec![Vec::new(), Vec::new()],
            source_name: "<test>".into(),
        };
        let p = plan(&[o], false, &no_dead());
        assert!(
            p.is_empty(),
            "cross-output-section fold must not happen: {:?}",
            p.redirect
        );
    }

    /// Under `safe`, a function whose address is taken elsewhere keeps its own
    /// address: C requires distinct functions to compare unequal.
    #[test]
    fn address_taken_blocks_folding_in_safe_mode() {
        let mut o = obj(
            &[CALL_RET, CALL_RET],
            vec![
                vec![rela(1, 2, R_X86_64_PLT32, -4)],
                vec![rela(1, 2, R_X86_64_PLT32, -4)],
                Vec::new(),
            ],
            vec![
                sym("", 0, 0, false),
                sym("", 0, 0, false),
                sym("alpha", 0, 0, true),
                sym("dup", 1, 0, true), // names section 1
            ],
        );
        // A data section holding an absolute pointer to `dup`.
        o.sections.push(Elf64Section {
            name_idx: 0,
            name: ".data.ptr".into(),
            sh_type: SHT_PROGBITS,
            flags: SHF_ALLOC,
            addr: 0,
            offset: 0,
            size: 8,
            link: 0,
            info: 0,
            addralign: 8,
            entsize: 0,
        });
        o.section_data.push(SectionData::owned(vec![0u8; 8]));
        o.relocations[2] = vec![rela(0, 3, R_X86_64_64, 0)];

        assert!(
            plan(std::slice::from_ref(&o), true, &no_dead()).is_empty(),
            "safe ICF must not fold an address-taken function"
        );
        assert_eq!(
            plan(&[o], false, &no_dead()).result.folded_sections,
            1,
            "--icf=all folds it anyway"
        );
    }

    /// Weak definition in one object, strong in another, absolute reference
    /// by name: the mark must land on EVERY definition. First-win marked
    /// only the shadowed weak twin; the surviving strong address then
    /// folded with its own twin and the table-observed address aliased a
    /// distinct function. Setup: the weak twin is byte-distinct (own
    /// class), so first-win's mark cannot accidentally save the strong
    /// class — this test folds under first-win and holds under mark-all.
    #[test]
    fn address_taken_marks_every_definition() {
        const RET0: &[u8] = &[0x31, 0xc0, 0xc3]; // xor eax,eax; ret
        // Object A: weak dup, byte-distinct from everything in B.
        let a = mkobj(
            vec![
                nullpart(),
                (sec(".text.dup", CALL_RET, 16), CALL_RET.to_vec(), vec![]),
            ],
            vec![
                sym("", 0, 0, false),
                Elf64Symbol {
                    name_idx: 0,
                    name: SymStr::new("dup"),
                    info: (2 << 4) | 2, // STB_WEAK | STT_FUNC
                    other: 0,
                    shndx: 1,
                    value: 0,
                    size: 0,
                },
            ],
        );
        // Object B: strong dup + identical twin + absolute table
        // referencing `dup` (resolves to the strong definition).
        let b = mkobj(
            vec![
                nullpart(),
                (sec(".text.dup", RET0, 16), RET0.to_vec(), vec![]),
                (sec(".text.twin", RET0, 16), RET0.to_vec(), vec![]),
                (
                    sec_flags(".data.tab", &[0u8; 8], 8, SHF_ALLOC),
                    vec![0u8; 8],
                    vec![rela(0, 1, R_X86_64_64, 0)],
                ),
            ],
            vec![
                sym("", 0, 0, false),
                sym("dup", 1, 0, true),
                sym("twin", 2, 0, true),
            ],
        );
        let p = plan(&[a, b], true, &no_dead());
        assert!(
            !p.redirect.contains_key(&(1, 1)),
            "strong address-taken dup folded: {:?}",
            p.redirect
        );
        assert!(p.is_empty(), "nothing may fold here: {:?}", p.redirect);
    }

    /// A NULL section like real objects start with: defined symbols can
    /// never live in section 0 (`shndx == 0` is `SHN_UNDEF`), so tests that
    /// name their first text section lead with one of these.
    fn nullpart() -> (Elf64Section, Vec<u8>, Vec<Elf64Rela>) {
        (
            Elf64Section {
                name_idx: 0,
                name: String::new(),
                sh_type: 0,
                flags: 0,
                addr: 0,
                offset: 0,
                size: 0,
                link: 0,
                info: 0,
                addralign: 0,
                entsize: 0,
            },
            Vec::new(),
            Vec::new(),
        )
    }

    /// PC-relative `lea` of a function is address-taking even though the
    /// relocation is relative: safe mode must not fold the target.
    #[test]
    fn lea_address_taken_blocks_safe_fold() {
        // Sections 2,3: identical twins; section 1 uses lea (8D) to
        // capture section 2's function address.
        let lea_user: &[u8] = &[0x48, 0x8d, 0x05, 0, 0, 0, 0, 0xc3];
        let o = mkobj(
            vec![
                nullpart(),
                (
                    sec(".text.user", lea_user, 16),
                    lea_user.to_vec(),
                    vec![rela(3, 1, R_X86_64_PC32, -4)],
                ),
                (sec(".text.f", CALL_RET, 16), CALL_RET.to_vec(), vec![]),
                (sec(".text.g", CALL_RET, 16), CALL_RET.to_vec(), vec![]),
            ],
            vec![sym("", 0, 0, false), sym("f", 2, 0, true)],
        );
        let p = plan(std::slice::from_ref(&o), true, &no_dead());
        assert!(
            p.is_empty(),
            "lea-taken function folded under safe: {:?}",
            p.redirect
        );
        assert_eq!(
            plan(&[o], false, &no_dead()).result.folded_sections,
            1,
            "--icf=all folds it anyway"
        );
    }

    /// The opcode check is code-only: a data section whose bytes happen to
    /// hold `E8` before a PC32 slot is still an address capture, not a
    /// call. Without the executable-source gate the `E8` hides the mark and
    /// safe mode folds an observed function.
    #[test]
    fn data_bytes_that_look_like_call_still_mark_taken() {
        // Data slot: byte 0 is E8 (padding that looks like `call`), the
        // PC32 reloc at 1 captures section 2's function address.
        let o = mkobj(
            vec![
                nullpart(),
                (
                    sec_flags(".rodata.slot", &[0xE8, 0, 0, 0, 0], 1, SHF_ALLOC),
                    vec![0xE8, 0, 0, 0, 0],
                    vec![rela(1, 1, R_X86_64_PC32, -4)],
                ),
                (sec(".text.f", CALL_RET, 16), CALL_RET.to_vec(), vec![]),
                (sec(".text.g", CALL_RET, 16), CALL_RET.to_vec(), vec![]),
            ],
            vec![sym("", 0, 0, false), sym("f", 2, 0, true)],
        );
        let p = plan(std::slice::from_ref(&o), true, &no_dead());
        assert!(
            p.is_empty(),
            "data-embedded E8 hid the taken mark: {:?}",
            p.redirect
        );
        assert_eq!(
            plan(&[o], false, &no_dead()).result.folded_sections,
            1,
            "--icf=all folds it anyway"
        );
    }

    /// And the essential counterpoint: a PC-relative `call` (E8) does NOT
    /// take the callee's address — callees stay foldable.
    #[test]
    fn call_does_not_mark_callee_taken() {
        let o = obj(
            &[CALL_RET, CALL_RET, CALL_RET],
            vec![
                vec![rela(1, 3, R_X86_64_PLT32, -4)],
                vec![rela(1, 3, R_X86_64_PLT32, -4)],
                vec![rela(1, 3, R_X86_64_PC32, -4)],
            ],
            vec![
                sym("", 0, 0, false),
                sym("", 0, 0, false),
                sym("", 0, 0, false),
                sym("alpha", 0, 0, true),
            ],
        );
        // Sections 0,1 identical callers fold; section 2 differs (PC32 vs
        // PLT32) and stays. `alpha` is undefined here, so no taken marks.
        let p = plan(&[o], true, &no_dead());
        assert_eq!(p.result.folded_sections, 1);
        assert_eq!(p.resolve((0, 1)), (0, 0));
    }

    /// Same absolute value spelled in two objects is the same entity:
    /// referencing it cannot distinguish the referrers.
    #[test]
    fn absolute_value_identity_folds() {
        let one = |tname: &str| {
            mkobj(
                vec![(
                    sec(tname, CALL_RET, 16),
                    CALL_RET.to_vec(),
                    vec![rela(1, 1, R_X86_64_PLT32, -4)],
                )],
                vec![
                    sym("", 0, 0, false),
                    Elf64Symbol {
                        name_idx: 0,
                        name: SymStr::new(""),
                        info: 0,
                        other: 0,
                        shndx: SHN_ABS,
                        value: 0x1234,
                        size: 0,
                    },
                ],
            )
        };
        let p = plan(&[one(".text.a"), one(".text.b")], true, &no_dead());
        assert_eq!(p.result.folded_sections, 1);
    }

    /// Dead sections (gc/COMDAT/strmerge originals) never enter the
    /// universe: no folds among the dead, no lies in `bytes_saved`.
    #[test]
    fn dead_sections_are_skipped() {
        let o = obj(
            &[CALL_RET, CALL_RET],
            vec![vec![], vec![]],
            vec![sym("", 0, 0, false)],
        );
        let mut dead = no_dead();
        dead.insert((0, 1));
        let p = plan(&[o], false, &dead);
        assert!(p.is_empty(), "folded a dead section: {:?}", p.redirect);
        assert_eq!(p.result.bytes_saved, 0);
    }

    /// Merge sections belong to string merging; ICF leaves them alone even
    /// when byte-identical (and they can never be fold targets either,
    /// since only `.text` folds).
    #[test]
    fn merge_sections_are_excluded() {
        let mk = |n: &str| {
            mkobj(
                vec![(
                    sec_flags(
                        n,
                        b"samedata",
                        1,
                        SHF_ALLOC | crate::backend::elf::SHF_MERGE,
                    ),
                    b"samedata".to_vec(),
                    vec![],
                )],
                vec![sym("", 0, 0, false)],
            )
        };
        let p = plan(&[mk(".rodata.a"), mk(".rodata.b")], false, &no_dead());
        assert!(p.is_empty());
    }

    /// No minimum-size heuristic: tiny identical functions are exactly what
    /// `-ffunction-sections` C++ emits by the thousand (thunks, guards),
    /// and exact confirmation makes "accidental match" impossible.
    #[test]
    fn tiny_identical_sections_fold() {
        let sled: &[u8] = &[0x90; 8];
        let o = obj(
            &[sled, sled],
            vec![vec![], vec![]],
            vec![sym("", 0, 0, false)],
        );
        let p = plan(&[o], false, &no_dead());
        assert_eq!(p.result.folded_sections, 1);
        assert_eq!(p.result.bytes_saved, 8);
    }

    /// Different alignment means the compiler asked for something stricter;
    /// folding onto a weaker section would silently break that.
    #[test]
    fn differing_alignment_does_not_fold() {
        let mut o = obj(
            &[CALL_RET, CALL_RET],
            vec![
                vec![rela(1, 2, R_X86_64_PLT32, -4)],
                vec![rela(1, 2, R_X86_64_PLT32, -4)],
            ],
            vec![
                sym("", 0, 0, false),
                sym("", 0, 0, false),
                sym("alpha", 0, 0, true),
            ],
        );
        o.sections[1].addralign = 64;
        assert!(
            plan(&[o], true, &no_dead()).is_empty(),
            "alignment is part of identity"
        );
    }

    /// The refinement loop is bounded: classes strictly split, so passes
    /// never exceed members + 1. A 3-deep chain exercises real iteration.
    #[test]
    fn iterations_are_bounded() {
        // Chain per object: f -> g -> h; h calls global leaf. Two objects.
        let one = |fname: &str, leaf: &str| {
            mkobj(
                vec![
                    (
                        sec(fname, CALL_RET, 16),
                        CALL_RET.to_vec(),
                        vec![rela(1, 1, R_X86_64_PLT32, -4)],
                    ),
                    (
                        sec(".text.g", CALL_RET, 16),
                        CALL_RET.to_vec(),
                        vec![rela(1, 2, R_X86_64_PLT32, -4)],
                    ),
                    (
                        sec(".text.h", CALL_RET, 16),
                        CALL_RET.to_vec(),
                        vec![rela(1, 3, R_X86_64_PLT32, -4)],
                    ),
                ],
                vec![
                    sym("", 0, 0, false),
                    sym("g", 1, 0, false),
                    sym("h", 2, 0, false),
                    sym(leaf, 0, 0, true),
                ],
            )
        };
        let objs = [one(".text.f1", "leaf"), one(".text.f2", "leaf")];
        let p = plan(&objs, true, &no_dead());
        assert_eq!(p.result.folded_sections, 3);
        assert!(
            p.result.iterations as usize <= 6 + 1,
            "iteration bound violated: {}",
            p.result.iterations
        );
    }

    /// Folding must be deterministic: same input, same representative, every
    /// run. Hash-map iteration order must not leak into the output.
    #[test]
    fn plan_is_deterministic() {
        let build = || {
            obj(
                &[CALL_RET, CALL_RET, CALL_RET],
                vec![
                    vec![rela(1, 3, R_X86_64_PLT32, -4)],
                    vec![rela(1, 3, R_X86_64_PLT32, -4)],
                    vec![rela(1, 3, R_X86_64_PLT32, -4)],
                ],
                vec![
                    sym("", 0, 0, false),
                    sym("", 0, 0, false),
                    sym("", 0, 0, false),
                    sym("alpha", 0, 0, true),
                ],
            )
        };
        let first = plan(&[build()], true, &no_dead());
        for _ in 0..8 {
            let again = plan(&[build()], true, &no_dead());
            assert_eq!(first.redirect, again.redirect, "ICF plan must be stable");
        }
        assert_eq!(first.result.folded_sections, 2);
        assert_eq!(first.resolve((0, 1)), (0, 0));
        assert_eq!(first.resolve((0, 2)), (0, 0));
    }

    /// PGO splits `.text` three ways and the linker keeps them split for
    /// I-cache locality; folding across the split would silently undo the
    /// compiler's partitioning. Same-output twins still fold.
    #[test]
    fn text_hot_twins_fold_within_their_output() {
        let mk = |n: &str| {
            mkobj(
                vec![(sec(n, CALL_RET, 16), CALL_RET.to_vec(), vec![])],
                vec![sym("", 0, 0, false)],
            )
        };
        let p = plan(&[mk(".text.hot"), mk(".text.hot")], true, &no_dead());
        assert_eq!(p.result.folded_sections, 1);
        assert_eq!(p.resolve((1, 0)), (0, 0));
    }

    #[test]
    fn text_hot_never_folds_onto_text_unlikely() {
        let mk = |n: &str| {
            mkobj(
                vec![(sec(n, CALL_RET, 16), CALL_RET.to_vec(), vec![])],
                vec![sym("", 0, 0, false)],
            )
        };
        for safe in [true, false] {
            let p = plan(&[mk(".text.hot"), mk(".text.unlikely")], safe, &no_dead());
            assert!(
                p.is_empty(),
                "safe={safe}: cross-output fold destroys PGO partitioning: {:?}",
                p.redirect
            );
        }
    }

    #[test]
    fn text_hot_never_folds_onto_plain_text() {
        let mk = |n: &str| {
            mkobj(
                vec![(sec(n, CALL_RET, 16), CALL_RET.to_vec(), vec![])],
                vec![sym("", 0, 0, false)],
            )
        };
        let p = plan(&[mk(".text.hot"), mk(".text.f")], false, &no_dead());
        assert!(
            p.is_empty(),
            "hot/plain cross-output fold: {:?}",
            p.redirect
        );
    }

    /// Executable contract on the output mapping [`output_discriminant`]
    /// relies on: if upstream ever remaps a `.text*` input (today even the
    /// `.text.hot.foo` quirk is load-bearing), this fails loudly and forces
    /// the discriminant to grow with it. The ICF code itself calls the real
    /// mapping, so it stays correct automatically — this test forces a human
    /// to notice the world changed.
    #[test]
    fn output_mapping_is_pinned() {
        assert_eq!(map_section_name(".text"), ".text");
        assert_eq!(map_section_name(".text.f"), ".text");
        assert_eq!(map_section_name(".text.hot"), ".text.hot");
        assert_eq!(map_section_name(".text.unlikely"), ".text.unlikely");
        // The quirk: only the EXACT names stay split; subsections merge.
        assert_eq!(map_section_name(".text.hot.f"), ".text");
        assert_eq!(map_section_name(".text.unlikely.f"), ".text");
        assert_eq!(map_section_name(".text.cold"), ".text");
        assert_eq!(map_section_name(".text.startup"), ".text");
        // And the discriminant covers exactly the three live outputs.
        assert_eq!(output_discriminant(".text"), 0);
        assert_eq!(output_discriminant(".text.hot"), 1);
        assert_eq!(output_discriminant(".text.unlikely"), 2);
    }

    /// `SHF_EXCLUDE` sections are never laid out by the merge step, so they
    /// must never enter the universe — not even to fold among themselves
    /// (that can only lie in `bytes_saved`). Note the trap this test
    /// sidesteps: an excluded section never folds with a laid-out one
    /// anyway, because the flag bit splits their buckets; the case that
    /// needs the membership gate is two excluded twins.
    #[test]
    fn excluded_sections_are_not_members() {
        let twin = |n: &str| {
            (
                sec_flags(n, CALL_RET, 16, SHF_EXECINSTR | SHF_ALLOC | SHF_EXCLUDE),
                CALL_RET.to_vec(),
                vec![],
            )
        };
        let o = mkobj(
            vec![twin(".text.f"), twin(".text.g")],
            vec![sym("", 0, 0, false)],
        );
        for safe in [true, false] {
            let p = plan(std::slice::from_ref(&o), safe, &no_dead());
            assert!(
                p.is_empty(),
                "safe={safe}: folded among excluded sections: {:?}",
                p.redirect
            );
            assert_eq!(p.result.bytes_saved, 0);
        }
    }

    /// Extended section indices all read as raw `0xffff` (the parser does
    /// not resolve `.symtab_shndx`), so two `SHN_XINDEX` locals with the same
    /// value can denote DIFFERENT sections: only the same symbol is the same
    /// entity. Same-object twins through one symbol still fold.
    #[test]
    fn extended_section_indices_need_same_symbol() {
        // Cross-object: same shape, same value, different symbols — apart.
        let one = |tname: &str| {
            mkobj(
                vec![(
                    sec(tname, CALL_RET, 16),
                    CALL_RET.to_vec(),
                    vec![rela(1, 1, R_X86_64_PLT32, -4)],
                )],
                vec![sym("", 0, 0, false), sym("x", SHN_XINDEX, 0, false)],
            )
        };
        let p = plan(&[one(".text.a"), one(".text.b")], true, &no_dead());
        assert!(
            p.is_empty(),
            "distinct extended sections equated: {:?}",
            p.redirect
        );
        // Same object, same symbol: one entity — folds.
        let o = mkobj(
            vec![
                (
                    sec(".text.a", CALL_RET, 16),
                    CALL_RET.to_vec(),
                    vec![rela(1, 1, R_X86_64_PLT32, -4)],
                ),
                (
                    sec(".text.b", CALL_RET, 16),
                    CALL_RET.to_vec(),
                    vec![rela(1, 1, R_X86_64_PLT32, -4)],
                ),
            ],
            vec![sym("", 0, 0, false), sym("x", SHN_XINDEX, 0, false)],
        );
        assert_eq!(plan(&[o], true, &no_dead()).result.folded_sections, 1);
        // Same object, DIFFERENT extended symbols: distinct sections behind
        // one raw `0xffff` — apart. (This is the case the reserved check
        // owns: the pre-check path would equate them via `(None,None)` +
        // same object.)
        let o2 = mkobj(
            vec![
                (
                    sec(".text.a", CALL_RET, 16),
                    CALL_RET.to_vec(),
                    vec![rela(1, 1, R_X86_64_PLT32, -4)],
                ),
                (
                    sec(".text.b", CALL_RET, 16),
                    CALL_RET.to_vec(),
                    vec![rela(1, 2, R_X86_64_PLT32, -4)],
                ),
            ],
            vec![
                sym("", 0, 0, false),
                sym("x", SHN_XINDEX, 0, false),
                sym("y", SHN_XINDEX, 0, false),
            ],
        );
        let p2 = plan(&[o2], true, &no_dead());
        assert!(
            p2.is_empty(),
            "distinct same-object extended sections equated: {:?}",
            p2.redirect
        );
    }

    /// Named COMMON globals keep name identity: tentative definitions with
    /// the same name merge at link time, so their callers are equivalent.
    #[test]
    fn named_common_global_callers_fold() {
        let one = |tname: &str| {
            mkobj(
                vec![(
                    sec(tname, CALL_RET, 16),
                    CALL_RET.to_vec(),
                    vec![rela(1, 1, R_X86_64_PLT32, -4)],
                )],
                vec![
                    sym("", 0, 0, false),
                    Elf64Symbol {
                        name_idx: 0,
                        name: SymStr::new("c"),
                        info: 1 << 4, // STB_GLOBAL
                        other: 0,
                        shndx: crate::backend::elf::SHN_COMMON,
                        value: 8,
                        size: 0,
                    },
                ],
            )
        };
        let p = plan(&[one(".text.a"), one(".text.b")], true, &no_dead());
        assert_eq!(p.result.folded_sections, 1);
    }

    /// An absolute number and a section target are never the same entity,
    /// even when the section offset numerically equals the number.
    #[test]
    fn absolute_and_section_targets_do_not_mix() {
        let abs = mkobj(
            vec![(
                sec(".text.a", CALL_RET, 16),
                CALL_RET.to_vec(),
                vec![rela(1, 1, R_X86_64_PLT32, -4)],
            )],
            vec![
                sym("", 0, 0, false),
                Elf64Symbol {
                    name_idx: 0,
                    name: SymStr::new(""),
                    info: 0,
                    other: 0,
                    shndx: SHN_ABS,
                    value: 0,
                    size: 0,
                },
            ],
        );
        let secm = mkobj(
            vec![
                nullpart(),
                (
                    sec(".text.b", CALL_RET, 16),
                    CALL_RET.to_vec(),
                    vec![rela(1, 1, R_X86_64_PLT32, -4)],
                ),
            ],
            vec![sym("", 0, 0, false), sym("s", 1, 0, false)],
        );
        let p = plan(&[abs, secm], false, &no_dead());
        assert!(p.is_empty(), "absolute/section confusion: {:?}", p.redirect);
    }

    /// Resolver sections never fold: the IPLT slot allocator skips ifuncs in
    /// dead sections, so folding one would drop its slot while callers still
    /// bind through it (callers would run the resolver bytes as the target).
    #[test]
    fn local_ifunc_resolvers_never_fold() {
        let ifunc = |n: &str, shndx: u16| Elf64Symbol {
            name_idx: 0,
            name: SymStr::new(n),
            info: STT_GNU_IFUNC, // STB_LOCAL | ifunc
            other: 0,
            shndx,
            value: 0,
            size: 0,
        };
        let o = mkobj(
            vec![
                nullpart(),
                (sec(".text.r1", CALL_RET, 16), CALL_RET.to_vec(), vec![]),
                (sec(".text.r2", CALL_RET, 16), CALL_RET.to_vec(), vec![]),
            ],
            vec![sym("", 0, 0, false), ifunc("r1", 1), ifunc("r2", 2)],
        );
        for safe in [true, false] {
            let p = plan(std::slice::from_ref(&o), safe, &no_dead());
            assert!(
                p.is_empty(),
                "safe={safe}: folded a resolver section: {:?}",
                p.redirect
            );
        }
    }

    /// A fragmented bucket (400 same-skeleton stubs, every one pointing at
    /// a different target class) must cost linear work, not the 80K
    /// comparisons a one-against-all recursion would burn: the linear scan
    /// fails the majority, the sketch path classifies each member once, and
    /// the suite of bounds below pins the receipt.
    #[test]
    fn fragmented_bucket_stays_linear() {
        const NSTUB: usize = 400;
        let mut parts = Vec::with_capacity(2 * NSTUB);
        parts.push(nullpart());
        let mut symbols = vec![sym("", 0, 0, false)];
        for i in 0..NSTUB {
            // Distinct target bytes => each target is a singleton class.
            let tdata = vec![0xc3, (i & 0xff) as u8, (i >> 8) as u8];
            parts.push((sec(&format!(".text.t{i}"), &tdata, 16), tdata, vec![]));
            symbols.push(sym("t", parts.len() as u16 - 1, 0, false));
        }
        for i in 0..NSTUB {
            parts.push((
                sec(&format!(".text.s{i}"), CALL_RET, 16),
                CALL_RET.to_vec(),
                vec![rela(1, 1 + i as u32, R_X86_64_PC32, -4)],
            ));
        }
        let o = mkobj(parts, symbols);
        let p = plan(&[o], true, &no_dead());
        // Every stub differs (distinct target classes): nothing folds.
        assert!(p.is_empty(), "fragmented bucket folded: {:?}", p.redirect);
        // Linear scan (399) + one sketch per member (400): far under the
        // 32-per-member budget and the pinned 16N receipt bound.
        let n = 2 * NSTUB as u64;
        assert!(
            p.result.comparisons <= 16 * n,
            "superlinear refinement: {} comparisons for {n} members",
            p.result.comparisons
        );
        assert_eq!(p.result.shattered, 0);
    }

    /// A 500-link reference chain peels one link per pass, so exact
    /// refinement would take 500 passes; the 32-pass cap stops it and
    /// shatters the unstable remainder — whose exact result is singletons
    /// anyway. Bounded work, exact outcome.
    #[test]
    fn chain_refinement_is_bounded_and_exact() {
        const NLINK: usize = 500;
        let mut parts = Vec::with_capacity(NLINK + 1);
        parts.push(nullpart());
        let mut symbols = vec![sym("", 0, 0, false), sym("ext", SHN_UNDEF, 0, true)];
        for i in 0..NLINK {
            let target = if i + 1 < NLINK { 3 + i as u32 } else { 1 };
            parts.push((
                sec(&format!(".text.f{i}"), CALL_RET, 16),
                CALL_RET.to_vec(),
                vec![rela(1, target, R_X86_64_PC32, -4)],
            ));
            symbols.push(sym("f", (i + 1) as u16, 0, false));
        }
        let o = mkobj(parts, symbols);
        let p = plan(&[o], true, &no_dead());
        // Every link differs (distinct target classes down the chain):
        // nothing folds, exactly as unbounded refinement would conclude.
        assert!(p.is_empty(), "chain folded: {:?}", p.redirect);
        // The dirty-class worklist converges exactly on deep chains well
        // before the cap (the sketch path distinguishes many chain heights
        // per pass): bounded means *at most* the cap, and the budget holds
        // either way. The pre-worklist design pinned `iterations == cap`
        // and `shattered > 0` here because it always ran to the limit.
        assert!(p.result.iterations <= MAX_REFINEMENT_PASSES);
        assert!(
            p.result.comparisons <= COMPARE_BUDGET_PER_MEMBER * NLINK as u64,
            "budget overrun: {}",
            p.result.comparisons
        );
    }

    /// A ring of identical stubs calling each other is self-consistent:
    /// every target shares the class, so the whole ring folds in one pass.
    /// (Folding a ring preserves behavior the same way folding a recursive
    /// function does — the cycle diverges identically through the single
    /// survivor.)
    #[test]
    fn ring_of_twins_folds() {
        const NLINK: usize = 3;
        let mut parts = Vec::with_capacity(NLINK + 1);
        parts.push(nullpart());
        let mut symbols = vec![sym("", 0, 0, false)];
        for i in 0..NLINK {
            parts.push((
                sec(&format!(".text.f{i}"), CALL_RET, 16),
                CALL_RET.to_vec(),
                vec![rela(1, 1 + ((i + 1) % NLINK) as u32, R_X86_64_PC32, -4)],
            ));
            symbols.push(sym("f", (i + 1) as u16, 0, false));
        }
        let o = mkobj(parts, symbols);
        let p = plan(&[o], true, &no_dead());
        assert_eq!(p.result.folded_sections, NLINK - 1);
        assert_eq!(p.result.shattered, 0);
    }

    /// Precision proof for the stop-the-line shatter: a 40-chain churns
    /// past the pass cap, but the twin pair references NOTHING — its
    /// comparison evidence never changes, so it keeps its proven class and
    /// folds. A naive shatter-everything stop would lose this fold.
    #[test]
    fn stable_twins_survive_adversarial_churn() {
        const NLINK: usize = 40;
        let mut parts = Vec::with_capacity(NLINK + 3);
        parts.push(nullpart());
        let mut symbols = vec![sym("", 0, 0, false), sym("ext", SHN_UNDEF, 0, true)];
        for i in 0..NLINK {
            let target = if i + 1 < NLINK { 3 + i as u32 } else { 1 };
            parts.push((
                sec(&format!(".text.f{i}"), CALL_RET, 16),
                CALL_RET.to_vec(),
                vec![rela(1, target, R_X86_64_PC32, -4)],
            ));
            symbols.push(sym("f", (i + 1) as u16, 0, false));
        }
        parts.push((sec(".text.g1", CALL_RET, 16), CALL_RET.to_vec(), vec![]));
        parts.push((sec(".text.g2", CALL_RET, 16), CALL_RET.to_vec(), vec![]));
        let o = mkobj(parts, symbols);
        let p = plan(&[o], true, &no_dead());
        assert_eq!(p.result.folded_sections, 1);
        assert_eq!(p.resolve((0, NLINK + 2)), (0, NLINK + 1));
        // Worklist convergence reaches the fixpoint on this churn before
        // the cap, so the twin pair folds without ever invoking the
        // shatter: the folded count and the redirect — not the stop
        // mechanics — are the semantic guarantees this test pins.
        assert!(p.result.iterations <= MAX_REFINEMENT_PASSES);
    }

    /// THE fold-quality regression of the class-aware + worklist adoption:
    /// two depth-synchronised 64-link twin chains (`a[i]` and `b[i]` are
    /// byte/reloc twins at every height, each calling the next link in its
    /// own chain). Heights stay distinct classes (the peel order from the
    /// chain end proves each level unique), but every height's twin pair
    /// folds: 64 folds from 128 links, one survivor per height. The
    /// pre-adoption algorithm (name-keyed globals, fixed 32-pass cap)
    /// level-peeled 32 heights and shattered the rest: it folded at most
    /// ~60 links here. `shattered == 0` pins that the full fold is an
    /// exact fixpoint result, not a cap stop.
    #[test]
    fn deep_twin_chains_fold_fully() {
        const NLINK: usize = 64;
        let mut parts = Vec::with_capacity(2 * NLINK + 1);
        parts.push(nullpart());
        let mut symbols = vec![sym("", 0, 0, false), sym("ext", SHN_UNDEF, 0, true)];
        for i in 0..NLINK {
            let t = if i + 1 < NLINK {
                3 + 2 * i as u32 + 2
            } else {
                1
            };
            parts.push((
                sec(&format!(".text.a{i}"), CALL_RET, 16),
                CALL_RET.to_vec(),
                vec![rela(1, t, R_X86_64_PC32, -4)],
            ));
            parts.push((
                sec(&format!(".text.b{i}"), CALL_RET, 16),
                CALL_RET.to_vec(),
                vec![rela(1, t, R_X86_64_PC32, -4)],
            ));
            symbols.push(sym("a", (2 * i + 1) as u16, 0, false));
            symbols.push(sym("b", (2 * i + 2) as u16, 0, false));
        }
        let o = mkobj(parts, symbols);
        let p = plan(&[o], true, &no_dead());
        assert_eq!(p.result.folded_sections, NLINK);
        assert_eq!(p.result.shattered, 0);
        assert!(p.result.iterations <= MAX_REFINEMENT_PASSES);
    }

    /// Soundness under resource exhaustion, pinned: the same twin-chain
    /// shape at depth 150 (×2 links) needs more per-level verifications
    /// than the comparison budget allows (one level peels per pass at
    /// ~one scan of the surviving bucket per pass; the budget is 64 per
    /// 300 members, so exhaustion is engineered here at depth ~94). The
    /// contract: never overspend, never cap-crash, and every emitted fold
    /// is an exactly-verified one — the bounded direction only ever loses
    /// folds, it never invents them.
    #[test]
    fn deep_twin_chains_bounded_under_exhaustion() {
        const NLINK: usize = 150;
        let mut parts = Vec::with_capacity(2 * NLINK + 1);
        parts.push(nullpart());
        let mut symbols = vec![sym("", 0, 0, false), sym("ext", SHN_UNDEF, 0, true)];
        for i in 0..NLINK {
            let t = if i + 1 < NLINK {
                3 + 2 * i as u32 + 2
            } else {
                1
            };
            parts.push((
                sec(&format!(".text.a{i}"), CALL_RET, 16),
                CALL_RET.to_vec(),
                vec![rela(1, t, R_X86_64_PC32, -4)],
            ));
            parts.push((
                sec(&format!(".text.b{i}"), CALL_RET, 16),
                CALL_RET.to_vec(),
                vec![rela(1, t, R_X86_64_PC32, -4)],
            ));
            symbols.push(sym("a", (2 * i + 1) as u16, 0, false));
            symbols.push(sym("b", (2 * i + 2) as u16, 0, false));
        }
        let o = mkobj(parts, symbols);
        let p = plan(&[o], true, &no_dead());
        assert!(p.result.folded_sections <= NLINK - 1);
        assert!(p.result.iterations <= MAX_REFINEMENT_PASSES);
        assert!(
            p.result.comparisons <= COMPARE_BUDGET_PER_MEMBER * (2 * NLINK) as u64,
            "budget overrun: {}",
            p.result.comparisons
        );
    }

    /// Relocation-free groups settle after one scan: byte equality is
    /// transitive, so a confirmed const group can never split again and
    /// later passes skip it (here the chain forces a second pass, which
    /// must cost zero const comparisons — 4 total, not 7).
    #[test]
    fn relocation_free_groups_settle_after_one_scan() {
        let mut parts = Vec::new();
        parts.push(nullpart());
        for i in 0..4 {
            parts.push((
                sec(&format!(".text.k{i}"), CALL_RET, 16),
                CALL_RET.to_vec(),
                vec![],
            ));
        }
        // 3-chain: f0 -> f1 -> f2 -> ext(global); f2's bucket is its own,
        // {f0, f1} peels once, forcing exactly two passes.
        parts.push((
            sec(".text.f0", CALL_RET, 16),
            CALL_RET.to_vec(),
            vec![rela(1, 6, R_X86_64_PC32, -4)],
        ));
        parts.push((
            sec(".text.f1", CALL_RET, 16),
            CALL_RET.to_vec(),
            vec![rela(1, 7, R_X86_64_PC32, -4)],
        ));
        parts.push((
            sec(".text.f2", CALL_RET, 16),
            CALL_RET.to_vec(),
            vec![rela(1, 1, R_X86_64_PC32, -4)],
        ));
        let o = mkobj(
            parts,
            vec![
                sym("", 0, 0, false),
                sym("ext", SHN_UNDEF, 0, true),
                sym("k0", 1, 0, false),
                sym("k1", 2, 0, false),
                sym("k2", 3, 0, false),
                sym("k3", 4, 0, false),
                sym("f1", 6, 0, false),
                sym("f2", 7, 0, false),
            ],
        );
        let p = plan(&[o], true, &no_dead());
        assert_eq!(p.result.folded_sections, 3);
        assert_eq!(p.result.iterations, 2);
        assert_eq!(p.result.comparisons, 4);
        assert_eq!(p.result.shattered, 0);
    }

    /// Unwind metadata never marks taken: every function is referenced by
    /// its own FDE, so counting `.eh_frame` (or `.eh_frame_hdr` /
    /// `.gcc_except_table`) as an address capture would mark the whole
    /// universe taken and safe mode would fold nothing, ever.
    #[test]
    fn unwind_metadata_references_do_not_mark_taken() {
        for eh_name in [".eh_frame", ".eh_frame_hdr", ".gcc_except_table"] {
            let o = mkobj(
                vec![
                    nullpart(),
                    (sec(".text.f", CALL_RET, 16), CALL_RET.to_vec(), vec![]),
                    (sec(".text.g", CALL_RET, 16), CALL_RET.to_vec(), vec![]),
                    (
                        sec_flags(eh_name, CALL_RET, 8, SHF_ALLOC),
                        CALL_RET.to_vec(),
                        vec![rela(0, 1, R_X86_64_64, 0)],
                    ),
                ],
                vec![sym("", 0, 0, false), sym("f", 1, 0, false)],
            );
            let p = plan(std::slice::from_ref(&o), true, &no_dead());
            assert_eq!(
                p.result.folded_sections, 1,
                "{eh_name} reference marked the target taken"
            );
        }
    }

    #[test]
    fn mode_parsing_rejects_garbage() {
        assert_eq!(parse_icf_mode("safe"), Some("safe"));
        assert_eq!(parse_icf_mode("ALL"), Some("all"));
        assert_eq!(parse_icf_mode("none"), None);
        assert_eq!(parse_icf_mode(""), None);
        assert_eq!(parse_icf_mode("yes"), None);
    }

    #[test]
    fn empty_input_is_handled() {
        assert!(plan(&[], true, &no_dead()).is_empty());
        assert!(plan(&[], false, &no_dead()).is_empty());
        assert_eq!(plan(&[], true, &no_dead()).result.iterations, 0);
    }

    #[test]
    fn hash_bytes_is_sensitive_to_content_and_length() {
        assert_eq!(hash_bytes(b"identical"), hash_bytes(b"identical"));
        assert_ne!(hash_bytes(b"identical"), hash_bytes(b"different"));
        // The documented trailing-zero trap: must differ.
        assert_ne!(hash_bytes(b"ab"), hash_bytes(b"ab\0"));
    }

    #[test]
    fn call_jump_decode() {
        // call rel32: E8 then disp32 at 1.
        assert!(is_call_or_jump(&[0xe8, 0, 0, 0, 0], 1, 4));
        // jmp rel32.
        assert!(is_call_or_jump(&[0xe9, 0, 0, 0, 0], 1, 4));
        // jcc rel32: 0F 8x then disp32 at 2.
        assert!(is_call_or_jump(&[0x0f, 0x84, 0, 0, 0, 0], 2, 4));
        // lea: 8D ModRM then disp32 at 3.
        assert!(!is_call_or_jump(&[0x48, 0x8d, 0x05, 0, 0, 0, 0], 3, 4));
        // REX-prefixed call still ends in E8.
        assert!(is_call_or_jump(&[0x41, 0xe8, 0, 0, 0, 0], 2, 4));
        // 0x84 without 0x0F (test r/m8, r8 ModRM, disp impossible) is data.
        assert!(!is_call_or_jump(&[0x20, 0x84, 0, 0, 0, 0], 2, 4));
        // Corrupt offsets: conservative.
        assert!(!is_call_or_jump(&[0xe8], 0, 4));
        assert!(!is_call_or_jump(&[0xe8, 0], 1, 4));
        assert!(!is_call_or_jump(&[0xe8, 0, 0, 0, 0], 9, 4));
    }
}
