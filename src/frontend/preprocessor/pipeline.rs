//! Full C preprocessor implementation.
//!
//! Struct definition, core preprocessing pipeline, directive dispatch,
//! and public configuration API. Predefined macros and target configuration
//! live in `predefined_macros`, pragma handling in `pragmas`, and text
//! processing (comment stripping, line joining) in `text_processing`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::fmt::Write;
use std::path::PathBuf;
use std::rc::Rc;

use super::builtin_macros::define_builtin_macros;
use super::conditionals::{ConditionalStack, evaluate_condition};
use super::macro_defs::{MacroDef, MacroTable, parse_define};
use super::text_processing::{
    CommentSegments, LineResolver, LineResolverData, LineSegments, split_first_word,
    strip_line_comment,
};
use super::utils::{is_ident_cont, is_ident_start};

/// Deduplicate a list of macro names, preserving order (first occurrence wins).
/// Used to remove duplicate names from nested macro expansions.
fn dedup_macro_names(names: Vec<String>) -> Vec<String> {
    let mut unique = Vec::with_capacity(8);
    for name in names {
        if !unique.contains(&name) {
            unique.push(name);
        }
    }
    unique
}

/// Maximum number of newlines to accumulate while joining lines for unbalanced
/// parentheses in macro arguments. Prevents runaway accumulation when a source
/// file has a genuinely unbalanced parenthesis. Must be large enough to handle
/// real-world macro calls that span many lines (e.g., QEMU's qapi-introspect.c
/// has a single QLIT_QLIST() macro invocation spanning ~32000 lines).
const MAX_PENDING_NEWLINES: usize = 100_000;

/// A preprocessor diagnostic with source location information.
///
/// Stores the file name, line number, and column where the diagnostic was
/// generated, enabling GCC-compatible `file:line:col: error:` / `file:line:col: warning:`
/// output for `#error`, `#warning`, and missing `#include` errors.
#[derive(Debug, Clone)]
pub struct PreprocessorDiagnostic {
    /// The source file where the diagnostic originated.
    pub file: String,
    /// The 1-based line number in the source file.
    pub line: usize,
    /// The 1-based column number in the source file.
    pub col: usize,
    /// The diagnostic message text (e.g., `#error "bad config"`).
    pub message: String,
}

pub struct Preprocessor {
    pub(super) macros: MacroTable,
    conditionals: ConditionalStack,
    pub(super) includes: Vec<String>,
    pub(super) filename: String,
    pub(super) errors: Vec<PreprocessorDiagnostic>,
    /// Collected preprocessor warnings (e.g., #warning directives).
    pub(super) warnings: Vec<PreprocessorDiagnostic>,
    /// Include search paths (from -I flags)
    pub(super) include_paths: Vec<PathBuf>,
    /// Quote include paths (from -iquote flags), searched only for #include "..."
    pub(super) quote_include_paths: Vec<PathBuf>,
    /// System include paths explicitly added (from -isystem flags)
    pub(super) isystem_include_paths: Vec<PathBuf>,
    /// After include paths (from -idirafter flags), searched last
    pub(super) after_include_paths: Vec<PathBuf>,
    /// System include paths (default search paths)
    pub(super) system_include_paths: Vec<PathBuf>,
    /// Files currently being processed (for recursion detection)
    pub(super) include_stack: Vec<PathBuf>,
    /// Files that have been included with #pragma once (path fallback).
    pub(super) pragma_once_files: FxHashSet<PathBuf>,
    /// Device/inode identities for #pragma once files.  This avoids double
    /// inclusion via different paths/symlinks/hardlinks on Linux filesystems.
    pub(super) pragma_once_identities: FxHashSet<(u64, u64)>,
    /// Whether to actually resolve includes (can be disabled for testing)
    pub(super) resolve_includes: bool,
    /// Declarations to inject into the output (from #include processing).
    pub(super) pending_injections: Vec<String>,
    /// Stack for #pragma push_macro / pop_macro.
    /// Maps macro name -> stack of saved definitions (None = was undefined).
    pub(super) macro_save_stack: FxHashMap<String, Vec<Option<MacroDef>>>,
    /// Line offset set by #line directive: effective_line = line_offset + (source_line - line_offset_base)
    /// When None, no #line has been issued and __LINE__ uses the source line directly.
    line_override: Option<(usize, usize)>, // (target_line, source_line_at_directive)
    /// Set by a `#line` (or GNU linemarker) directive until the line marker
    /// that makes diagnostics follow it is emitted: the marker's trailing
    /// flags (`""` for `#line`, e.g. `" 1"` for `# 1 "x.h" 1`).
    line_marker_due: Option<String>,
    /// #pragma weak directives: (symbol, optional_alias_target)
    /// - (symbol, None) means "mark symbol as weak"
    /// - (symbol, Some(target)) means "symbol is a weak alias for target"
    pub weak_pragmas: Vec<(String, Option<String>)>,
    /// Any active STDC FENV_ACCESS ON in this TU, including _Pragma and
    /// force-includes. Monotone: later OFF cannot erase earlier obligations.
    pub fenv_access_requested: bool,
    /// #pragma redefine_extname directives: (old_name, new_name)
    pub redefine_extname_pragmas: Vec<(String, String)>,
    /// Accumulated output from force-included files (-include).
    /// Prepended to the main source's preprocessed output so that pragma
    /// synthetic tokens (e.g., visibility push/pop) take effect.
    force_include_output: String,
    /// Cache for include path resolution.
    /// Maps (include_path, is_system, current_dir_key) to the resolved filesystem path.
    /// This avoids repeated `stat()` calls when the same header is included from
    /// multiple locations with the same include search path configuration.
    /// The current_dir_key is the parent directory of the including file for quoted
    /// includes (since resolution depends on it), or empty for system includes.
    pub(super) include_resolve_cache: FxHashMap<(String, bool, PathBuf), Option<PathBuf>>,
    /// Include guard detection: maps file paths to their guard macro names.
    ///
    /// After preprocessing an included file, we scan the raw source to detect if
    /// the entire file is wrapped in a classic include guard pattern:
    ///   #ifndef GUARD_MACRO
    ///   #define GUARD_MACRO
    ///   ...
    ///   #endif
    ///
    /// On subsequent #include of the same file, if the guard macro is still defined,
    /// we skip re-processing entirely (same optimization as GCC/Clang).
    pub(super) include_guard_macros: FxHashMap<PathBuf, String>,
    /// Reusable FxHashSet for directive-level macro expansion (handle_if, handle_elif,
    /// handle_line_directive, #error). Avoids allocating a new FxHashSet per directive.
    directive_expanding: FxHashSet<String>,
    /// Macro expansion metadata: maps preprocessed output line numbers to
    /// the macros expanded on that line. Populated during preprocessing and
    /// passed to the SourceManager for diagnostic rendering.
    macro_expansion_info: Vec<crate::common::source::MacroExpansionInfo>,
    /// Make-dependency tracking (-MD/-MMD/-M/-MM): every file the
    /// preprocessor actually opened, in first-open order, paired with a
    /// "resolved inside a system directory" verdict (GCC's -MMD/-MM omit
    /// system-directory headers; -MD/-M list them).  kbuild's fixdep greps
    /// these .d files for CONFIG_* tokens to drive config-flip rebuilds —
    /// headers missing from deps silently starve rebuilds and surface as
    /// undefined-symbol link errors far away from the cause.
    pub(super) dep_files: Vec<(PathBuf, bool)>,
    /// Dedup set for dep_files (first-open order preserved by the Vec).
    pub(super) dep_seen: FxHashSet<PathBuf>,
}

impl Preprocessor {
    pub fn new() -> Self {
        // A fresh preprocessor starts in the GCC 16.2 default dialect
        // (gnu23); an explicit older `-std=` switches it via `set_c23`.
        super::utils::set_c23_dialect(true);
        let mut pp = Self {
            macros: MacroTable::new(),
            conditionals: ConditionalStack::new(),
            includes: Vec::new(),
            filename: String::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            include_paths: Vec::new(),
            quote_include_paths: Vec::new(),
            isystem_include_paths: Vec::new(),
            after_include_paths: Vec::new(),
            system_include_paths: Self::default_system_include_paths(),
            include_stack: Vec::new(),
            pragma_once_files: FxHashSet::default(),
            pragma_once_identities: FxHashSet::default(),
            resolve_includes: true,
            pending_injections: Vec::new(),
            macro_save_stack: FxHashMap::default(),
            line_override: None,
            line_marker_due: None,
            weak_pragmas: Vec::new(),
            fenv_access_requested: false,
            redefine_extname_pragmas: Vec::new(),
            force_include_output: String::new(),
            include_resolve_cache: FxHashMap::default(),
            include_guard_macros: FxHashMap::default(),
            directive_expanding: FxHashSet::default(),
            macro_expansion_info: Vec::new(),
            dep_files: Vec::new(),
            dep_seen: FxHashSet::default(),
        };
        pp.define_predefined_macros();
        define_builtin_macros(&mut pp.macros);
        pp
    }

    /// Process source code, expanding macros and handling conditionals.
    /// Returns the preprocessed source with embedded line markers for source tracking.
    pub fn preprocess(&mut self, source: &str) -> String {
        // Emit initial line marker for the main file
        let line_marker = format!("# 1 \"{}\"\n", self.filename);
        let main_output = self.preprocess_source(source, false);
        // Prepend any output from force-included files (e.g., pragma synthetic tokens)
        let (result, prefix_lines) = if self.force_include_output.is_empty() {
            let prefix_lines = line_marker
                .as_bytes()
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as u32;
            let mut result = line_marker;
            result.push_str(&main_output);
            (result, prefix_lines)
        } else {
            let mut result = std::mem::take(&mut self.force_include_output);
            result.push_str(&line_marker);
            let prefix_lines = result.as_bytes().iter().filter(|&&b| b == b'\n').count() as u32;
            result.push_str(&main_output);
            (result, prefix_lines)
        };
        // Adjust macro expansion line numbers to account for prepended content
        // (line markers, force-include output) that shifts all line numbers.
        if prefix_lines > 0 {
            for info in &mut self.macro_expansion_info {
                info.pp_line += prefix_lines;
            }
        }
        result
    }

    /// Process source code from an included file. Same pipeline as preprocess()
    /// but saves/restores the conditional stack and skips pending_injections.
    pub(super) fn preprocess_included(&mut self, source: &str) -> String {
        self.preprocess_source(source, true)
    }

    /// Unified preprocessing pipeline for both top-level and included sources.
    ///
    /// When `is_include` is true:
    /// - Saves and restores the conditional stack (each file gets its own)
    /// - Does not emit pending_injections (those only apply to top-level)
    /// - Only processes directives when no multi-line accumulation is pending
    fn preprocess_source(&mut self, source: &str, is_include: bool) -> String {
        // Per C standard (C11 5.1.1.2), translation phases are:
        // Phase 2: Line splicing (backslash-newline removal)
        // Phase 3: Comment replacement
        // So we must join continued lines BEFORE stripping comments.
        //
        // Both transforms return per-line source-position maps so __LINE__
        // can resolve the PHYSICAL line of each token: splicing and comment
        // collapse shift line numbers, and one output line may even contain
        // bytes from several physical lines (tokens after a comment's
        // closing `*/` or after a continuation splice).
        let (joined, join_map) = self.join_continued_lines(source);
        let (source, comment_map) = Self::strip_block_comments(&joined);
        // Share the join map across every output line. `Rc::from(&[T])` copies
        // the slice; doing that *per line* is Θ(N²) on amalgamations that have
        // both a line-continuation and a multi-line block comment (sqlite3.c
        // ~270k lines previously hung in `build_line_resolver` for minutes).
        let join_map_rc: Rc<[LineSegments]> = match join_map.as_deref() {
            Some(m) if !m.is_empty() => Rc::from(m),
            _ => Rc::from([]),
        };
        let mut output = String::with_capacity(source.len() + source.len() / 4);
        // Absolute offset of the current line's first byte within the
        // comment-stripped text: the base that translates the macro
        // expander's line-relative offsets into the resolver's absolute
        // coordinate space.
        let mut stripped_line_start: usize = 0;
        // Absolute offset of each JOINED-text line start. The join map's
        // breakpoints live in joined-text coordinates; the macro expander
        // queries in stripped-text coordinates (comments collapsed lines /
        // removed bytes). Per-line delta = stripped_start - joined_start.
        let joined_starts: Vec<usize> = {
            let mut v = Vec::with_capacity(64);
            let mut off = 0usize;
            for jl in joined.lines() {
                v.push(off);
                off += jl.len() + 1;
            }
            v
        };

        // For included files, save and reset the conditional stack and line override
        let saved_conditionals = if is_include {
            Some(std::mem::take(&mut self.conditionals))
        } else {
            None
        };
        let saved_line_override = if is_include {
            self.line_override.take()
        } else {
            None
        };
        let saved_line_marker_due = if is_include {
            self.line_marker_due.take()
        } else {
            None
        };

        // Buffer for accumulating multi-line macro invocations
        let mut pending_line = String::with_capacity(256);
        let mut pending_newlines: usize = 0;
        // (offset in pending_line, physical line) of each accumulated line.
        let mut pending_starts: Vec<(usize, usize)> = Vec::new();

        // Physical start line of the most recently processed line; used for
        // the flat __LINE__ resolver when flushing accumulated multi-line
        // macro invocations (their text spans lines, so per-line offsets do
        // not apply).
        let mut last_source_line_num: usize = 0;
        // Track current line number in the preprocessed output for macro expansion metadata.
        // Incremented whenever a newline is appended to the output string.
        let mut pp_output_line: u32 = 0;

        // Reusable FxHashSet for macro expansion, avoiding per-line allocation.
        // This set tracks which macros are currently being expanded (to prevent
        // infinite recursion per C11 §6.10.3.4). It's cleared before each use
        // by expand_line_reuse().
        let mut expanding = crate::common::fx_hash::FxHashSet::default();

        // Enable macro expansion tracking for diagnostic "in expansion of macro" notes.
        // Only track at top level (not within included files) to avoid duplicate entries.
        if !is_include {
            self.macros.set_track_expansions(true);
        }

        for (line_num, line) in source.lines().enumerate() {
            // Execute `_Pragma` operators queued by the previous line's macro
            // expansion BEFORE processing this line, so their effects (C99
            // §6.10.9) apply in source order relative to directives.
            self.drain_expansion_pragmas();
            let trimmed = line.trim();

            // Build this output line's source-position resolver: comment
            // breakpoints (if any) composed with the join map give the
            // physical source line of every byte of the line.
            let line_resolver = self.build_line_resolver(
                line_num,
                stripped_line_start,
                &joined_starts,
                &comment_map,
                &join_map_rc,
            );
            let source_line_num = line_resolver.line_at(stripped_line_start);
            // Every iteration contributes exactly one output line, so a line
            // starting more than one physical line after its predecessor
            // follows a line that spanned several (backslash splices, or a
            // block comment collapsed inside a directive). Re-synchronize
            // with blank lines -- inside a multi-line macro invocation by
            // counting them -- so tokens and diagnostics on this and later
            // lines keep their physical line, as `cpp` does.
            if line_num > 0 {
                let gap = source_line_num.saturating_sub(last_source_line_num + 1);
                if gap > 0 {
                    if pending_line.is_empty() {
                        output.extend(std::iter::repeat_n('\n', gap));
                        if !is_include {
                            pp_output_line = pp_output_line
                                .saturating_add(u32::try_from(gap).unwrap_or(u32::MAX));
                        }
                    } else {
                        Self::pend_blank(&mut pending_line, &mut pending_newlines, gap);
                    }
                }
            }
            last_source_line_num = source_line_num;
            self.macros
                .set_line_resolver(line_resolver, self.line_override, stripped_line_start);
            stripped_line_start += line.len() + 1;

            // Directive handling: #if/#ifdef/#ifndef/#elif/#else/#endif must always
            // be processed regardless of pending multi-line accumulation. Other
            // directives (#include, #define, etc.) are only processed when there's
            // no pending line in included files.
            let is_directive = trimmed.starts_with('#');
            let is_conditional_directive = if is_directive {
                let after_hash = trimmed[1..].trim_start();
                after_hash.starts_with("if")
                    || after_hash.starts_with("elif")
                    || after_hash.starts_with("else")
                    || after_hash.starts_with("endif")
            } else {
                false
            };

            // When we're accumulating a multi-line macro invocation (pending_line
            // is non-empty) and hit a conditional directive, we must process the
            // directive to update the conditional stack but NOT flush the pending
            // line. The macro argument collection must continue across
            // #ifdef/#endif boundaries. This handles cases like:
            //   FOO(1, #ifdef BAR 2, #endif 3)
            // where the #ifdef/#endif must be evaluated but the macro args keep
            // accumulating until the closing ')'.
            if is_conditional_directive && !pending_line.is_empty() {
                // Process the conditional directive (updates conditional stack)
                // Column of the directive keyword (1-based, after '#')
                let hash_col = line.find('#').map(|i| i + 2).unwrap_or(1);
                self.process_directive(trimmed, source_line_num + 1, hash_col);
                // Don't add the directive line to pending_line, don't flush.
                // Just count a newline for line numbering preservation.
                Self::pend_blank(&mut pending_line, &mut pending_newlines, 1);
                continue;
            }

            // Similarly, when accumulating a multi-line macro invocation in an
            // included file, #define/#undef directives must be processed (to
            // register/remove macros) without interrupting the accumulation.
            // Without this, #define directives inside #ifdef blocks in included
            // files would leak into the output as literal text. This mirrors the
            // conditional directive handling above.
            let is_define_undef_directive = if is_directive && !is_conditional_directive {
                let after_hash = trimmed[1..].trim_start();
                after_hash.starts_with("define") || after_hash.starts_with("undef")
            } else {
                false
            };
            if is_define_undef_directive && is_include && !pending_line.is_empty() {
                if self.conditionals.is_active() {
                    let hash_col = line.find('#').map(|i| i + 2).unwrap_or(1);
                    self.process_directive(trimmed, source_line_num + 1, hash_col);
                }
                Self::pend_blank(&mut pending_line, &mut pending_newlines, 1);
                continue;
            }

            // Handle #line directives during multi-line accumulation.
            // Generated code (e.g. gforth's prim.i) can have #line directives
            // between every line of code, including in the middle of multi-line
            // expressions. These must be consumed (updating line tracking state)
            // without being appended to the pending accumulation buffer.
            let is_line_directive = if is_directive && !is_conditional_directive {
                let after_hash = trimmed[1..].trim_start();
                after_hash.starts_with("line ")
                    || after_hash.starts_with("line\t")
                    || after_hash
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_digit())
            } else {
                false
            };
            if is_line_directive && !pending_line.is_empty() {
                if self.conditionals.is_active() {
                    let hash_col = line.find('#').map(|i| i + 2).unwrap_or(1);
                    self.process_directive(trimmed, source_line_num + 1, hash_col);
                }
                Self::pend_blank(&mut pending_line, &mut pending_newlines, 1);
                continue;
            }

            let process_directive = is_directive && (!is_include || pending_line.is_empty());

            if process_directive {
                // When accumulating a multi-line expression (pending_line is non-empty)
                // and a #define or #undef directive appears, we must expand macros in
                // the accumulated text BEFORE the directive modifies the macro table.
                // This ensures tokens like D(0) are expanded using the macro definition
                // that was active when those tokens were encountered, not the definition
                // (or lack thereof) after the directive. Per C standard, directives take
                // effect for tokens that follow them, not tokens that precede them.
                let output_len_before_directive = output.len();
                if !pending_line.is_empty() && !is_include {
                    let after_hash = trimmed[1..].trim_start();
                    if after_hash.starts_with("define") || after_hash.starts_with("undef") {
                        // The accumulated text spans several physical lines;
                        // expand it against a flat resolver (the current
                        // line) rather than per-line offsets.
                        // Flat resolver: the accumulated text spans several
                        // physical lines, so per-offset resolution does not
                        // apply; the offset base is irrelevant.
                        self.macros.set_line_resolver(
                            Self::accumulated_resolver(&pending_starts),
                            self.line_override,
                            0,
                        );
                        let expanded = self.macros.expand_line_reuse(&pending_line, &mut expanding);
                        self.drain_expansion_pragmas();
                        pending_line.clear();
                        pending_line.push_str(&expanded);
                        // Already expanded: positions no longer matter.
                        pending_starts.clear();
                        pending_starts.push((0, source_line_num));
                    } else if after_hash.starts_with("include") {
                        // When #include appears in the middle of a multi-line expression
                        // (e.g. a function call with args spanning lines), flush the
                        // pending tokens to output first. The included content must appear
                        // after the preceding tokens, not before them.
                        // Flat resolver: the accumulated text spans several
                        // physical lines, so per-offset resolution does not
                        // apply; the offset base is irrelevant.
                        self.macros.set_line_resolver(
                            Self::accumulated_resolver(&pending_starts),
                            self.line_override,
                            0,
                        );
                        let expanded = self.macros.expand_line_reuse(&pending_line, &mut expanding);
                        self.drain_expansion_pragmas();
                        let expanded =
                            self.resolve_has_macros_in_code(&expanded, source_line_num + 1);
                        Self::push_accumulated(&mut output, &expanded, pending_newlines);
                        pending_line.clear();
                        pending_newlines = 0;
                    }
                }
                // Column of the directive keyword (1-based, after '#')
                let hash_col = line.find('#').map(|i| i + 2).unwrap_or(1);
                let include_result = self.process_directive(trimmed, source_line_num + 1, hash_col);
                let emitted_include_content =
                    include_result.as_deref().is_some_and(|c| !c.is_empty());
                if let Some(included_content) = include_result {
                    output.push_str(&included_content);
                    // After included content, emit a return-to-parent line marker
                    // so the source manager can map subsequent lines back to the
                    // correct file and line number. source_line_num is 0-based,
                    // and the next line of the parent file is source_line_num + 2
                    // (since the #include directive itself was source_line_num + 1).
                    // After a #line, the presumed line and file (as GCC).
                    let parent_file = if self.line_override.is_some() {
                        self.presumed_file_operand()
                    } else {
                        let file = self
                            .include_stack
                            .last()
                            .map(|p| super::includes::format_path_for_line_directive(p))
                            .unwrap_or_else(|| self.filename.clone());
                        format!("\"{file}\"")
                    };
                    // Flag 2 indicates returning from an include file (GCC convention)
                    let next_line = self.presumed_line(source_line_num + 1);
                    let _ = writeln!(output, "# {next_line} {parent_file} 2");
                } else if is_include {
                    // Included files always emit a newline for non-include
                    // directives (a #line's marker is one).
                    match self.take_line_marker(source_line_num + 1) {
                        Some(marker) => output.push_str(&marker),
                        None => output.push('\n'),
                    }
                }
                // Top-level: emit injected declarations from #include processing
                if !is_include && !self.pending_injections.is_empty() {
                    for decl in std::mem::take(&mut self.pending_injections) {
                        output.push_str(&decl);
                    }
                }
                // Preserve line numbering during multi-line accumulation
                if !is_include {
                    // ...unless the directive already preserved it.  A
                    // #include writes the included text plus a
                    // return-to-parent marker, and that marker names the
                    // parent's NEXT line -- so the directive's own line is
                    // accounted for.  Emitting a newline as well inserted a
                    // phantom line after every include, and every diagnostic
                    // in the rest of the file was then reported one line too
                    // high (`#include <stddef.h>\nint bad = ;` blamed line 3).
                    if !emitted_include_content {
                        if !pending_line.is_empty() {
                            Self::pend_blank(&mut pending_line, &mut pending_newlines, 1);
                        } else if let Some(marker) = self.take_line_marker(source_line_num + 1) {
                            // Stands for the directive's line.
                            output.push_str(&marker);
                        } else {
                            output.push('\n');
                        }
                    }
                    // Update pp_output_line for directive path
                    let added = &output.as_bytes()[output_len_before_directive..];
                    pp_output_line += added.iter().filter(|&&b| b == b'\n').count() as u32;
                }
            } else if self.conditionals.is_active() {
                // Regular line (or directive during include with pending line) -
                // expand macros, handling multi-line macro invocations
                let output_len_before = output.len();
                // C23 inline #embed: splice the byte-list expansion into the
                // line BEFORE macro expansion so prefix/suffix parameter
                // tokens participate in it like any other text.
                let spliced_line;
                let line: &str = match self.splice_inline_embed(line, source_line_num) {
                    Some(replaced) => {
                        spliced_line = replaced;
                        &spliced_line
                    }
                    None => line,
                };
                self.accumulate_and_expand(
                    line,
                    source_line_num,
                    &mut pending_starts,
                    &mut pending_line,
                    &mut pending_newlines,
                    &mut output,
                    &mut expanding,
                );
                // A #line inside the invocation: its marker goes before the
                // flushed text's last line, which comes from this physical
                // line (the invocation's line breaks are re-emitted). Only
                // that line is moved, never the whole output.
                if pending_line.is_empty() && output.len() > output_len_before {
                    if let Some(marker) = self.take_line_marker(source_line_num) {
                        let flushed = &output[output_len_before..output.len() - 1];
                        let last_line =
                            output_len_before + flushed.rfind('\n').map_or(0, |k| k + 1);
                        let last = output.split_off(last_line);
                        output.push_str(&marker);
                        output.push_str(&last);
                    }
                }
                // Collect macro expansion info for diagnostics.
                // If expansion added content to the output, check if macros were used.
                if !is_include && output.len() > output_len_before {
                    let expanded_names = self.macros.take_expanded_macros();
                    if !expanded_names.is_empty() {
                        let unique_names = dedup_macro_names(expanded_names);
                        self.macro_expansion_info
                            .push(crate::common::source::MacroExpansionInfo {
                                pp_line: pp_output_line,
                                macro_names: unique_names,
                            });
                    }
                }
                // Update pp_output_line by counting newlines in the newly added content
                if !is_include {
                    let added = &output.as_bytes()[output_len_before..];
                    pp_output_line += added.iter().filter(|&&b| b == b'\n').count() as u32;
                }
            } else {
                // Inactive conditional block - skip the line but preserve numbering
                if !pending_line.is_empty() {
                    Self::pend_blank(&mut pending_line, &mut pending_newlines, 1);
                } else {
                    output.push('\n');
                    if !is_include {
                        pp_output_line += 1;
                    }
                }
            }
        }

        // Pragmas queued by the final line's expansion.
        self.drain_expansion_pragmas();

        // Flush any remaining pending line
        if !pending_line.is_empty() {
            self.macros.set_line_resolver(
                Self::accumulated_resolver(&pending_starts),
                self.line_override,
                0,
            );
            let expanded = self.macros.expand_line_reuse(&pending_line, &mut expanding);
            self.drain_expansion_pragmas();
            let expanded = self.resolve_has_macros_in_code(&expanded, last_source_line_num + 1);
            output.push_str(&expanded);
            output.push('\n');
            // Collect macro expansion info for the flushed line
            if !is_include {
                let expanded_names = self.macros.take_expanded_macros();
                if !expanded_names.is_empty() {
                    let unique_names = dedup_macro_names(expanded_names);
                    self.macro_expansion_info
                        .push(crate::common::source::MacroExpansionInfo {
                            pp_line: pp_output_line,
                            macro_names: unique_names,
                        });
                }
            }
        }

        // Disable macro expansion tracking
        if !is_include {
            self.macros.set_track_expansions(false);
        }

        // Restore conditional stack and line override for included files
        if let Some(saved) = saved_conditionals {
            self.conditionals = saved;
        }
        if is_include {
            self.line_override = saved_line_override;
            self.line_marker_due = saved_line_marker_due;
        }

        output
    }

    /// The presumed (`#line`-adjusted) 1-based number of 0-based physical
    /// line `phys`, as `MacroTable::resolve_line` computes it.
    fn presumed_line(&self, phys: usize) -> usize {
        match self.line_override {
            Some((target, base)) => target + phys.saturating_sub(base),
            None => phys + 1,
        }
    }

    /// The presumed file name as a line-marker operand: `__FILE__`'s
    /// spelling, which `#line N "name"` sets and includes save and restore.
    fn presumed_file_operand(&self) -> String {
        self.macros
            .get_file_body()
            .map_or_else(|| format!("\"{}\"", self.filename), str::to_string)
    }

    /// The line marker a preceding `#line` made due, for the output line
    /// that comes from 0-based physical line `next_phys`; it makes the
    /// source manager (diagnostics, line tables) follow the directive.
    fn take_line_marker(&mut self, next_phys: usize) -> Option<String> {
        let flags = self.line_marker_due.take()?;
        Some(format!(
            "# {} {}{}\n",
            self.presumed_line(next_phys),
            self.presumed_file_operand(),
            flags
        ))
    }

    /// Build the `__LINE__` resolver for one output line: comment-strip
    /// breakpoints composed with the join map, or the flat fast path.
    ///
    /// `line_num` is the output (comment-stripped) line index; `comment_map`
    /// is indexed by output line and carries `(stripped_offset,
    /// joined_offset, joined_line)` breakpoints (absolute text coordinates);
    /// `join_map` is indexed by joined line and carries
    /// `(joined_offset, physical_line)` splice breakpoints in ABSOLUTE
    /// joined-text coordinates. `stripped_line_start` is the output line's
    /// absolute start in the stripped text and `joined_starts` holds every
    /// joined line's absolute start; their difference is the coordinate
    /// delta that translates splice breakpoints into the expander's
    /// stripped-text coordinate space. Composition gives the physical line
    /// of any byte.
    fn build_line_resolver(
        &self,
        line_num: usize,
        stripped_line_start: usize,
        joined_starts: &[usize],
        comment_map: &Option<Vec<CommentSegments>>,
        join_map: &Rc<[LineSegments]>,
    ) -> LineResolver {
        let jm: &[LineSegments] = join_map.as_ref();
        // Absolute joined-text offset of the joined line this output line
        // starts at. (The joined-line index itself is recovered from the
        // comment map / join map in the match below; only the offset is
        // needed for the splice-breakpoint delta.)
        let j_start: usize = match comment_map.as_deref().and_then(|c| c.get(line_num)) {
            Some(CommentSegments::Mapped(pts)) => match pts.first() {
                Some(&(_, jo, _)) => jo,
                None => joined_starts.get(line_num).copied().unwrap_or(0),
            },
            Some(CommentSegments::Flat(jl)) => joined_starts.get(*jl).copied().unwrap_or(0),
            None => joined_starts.get(line_num).copied().unwrap_or(0),
        };
        // Stripped start - joined start: comment stripping only removes
        // bytes, so the stripped text never runs AHEAD of the joined text
        // and the splice breakpoints shift LEFT by |delta|.
        let delta = stripped_line_start as isize - j_start as isize;
        // Build splice breakpoints in stripped coordinates: (o + delta, o,
        // joined_line). Between breakpoints on this line the stripped and
        // joined offsets advance in lockstep, so line_at()'s
        // joined = jo + (abs - so) lands inside the same splice segment.
        //
        // The join map is a *shared* Rc cloned in O(1). Never `Rc::from`
        // the whole slice here: that copy is Θ(file lines) per output line.
        let splice_points = |pts: &[(usize, usize)], jl: usize| -> LineResolver {
            LineResolver::Mapped(Rc::new(LineResolverData {
                points: pts
                    .iter()
                    .map(|&(o, _)| {
                        let so = (o as isize + delta).max(0) as usize;
                        (so, o, jl)
                    })
                    .collect::<Vec<_>>()
                    .into(),
                join_map: Rc::clone(join_map),
            }))
        };
        match comment_map.as_deref() {
            // No multi-line comments anywhere: the join map is the only
            // possible source of line shifts (single-line comment removal
            // shifts offsets, which the delta absorbs).
            None => match jm.get(line_num) {
                Some(LineSegments::Mapped(pts)) => splice_points(pts, line_num),
                // Flat(phys): a standalone joined line. When earlier lines
                // were spliced, phys is the PHYSICAL line and generally
                // differs from the joined/output line number line_num.
                Some(LineSegments::Flat(phys)) => LineResolver::Flat(*phys),
                None => LineResolver::Flat(line_num),
            },
            Some(c) => {
                match c.get(line_num) {
                    // Comment breakpoints on this line: they already carry
                    // (stripped, joined) absolute pairs.
                    Some(CommentSegments::Mapped(pts)) => {
                        LineResolver::Mapped(Rc::new(LineResolverData {
                            points: pts.clone(),
                            join_map: Rc::clone(join_map),
                        }))
                    }
                    // No comment breakpoints on this line; the joined line it
                    // came from decides via the join map's splice breakpoints.
                    Some(CommentSegments::Flat(jl)) => match jm.get(*jl) {
                        Some(LineSegments::Mapped(pts)) => splice_points(pts, *jl),
                        Some(LineSegments::Flat(phys)) => LineResolver::Flat(*phys),
                        None => LineResolver::Flat(*jl),
                    },
                    None => LineResolver::Flat(line_num),
                }
            }
        }
    }

    /// Accumulate lines for multi-line macro invocations (unbalanced parens)
    /// and expand when complete.
    ///
    /// The `expanding` parameter is a reusable FxHashSet that avoids per-line
    /// allocation for macro expansion tracking.
    fn accumulate_and_expand(
        &mut self,
        line: &str,
        source_line_num: usize,
        pending_starts: &mut Vec<(usize, usize)>,
        pending_line: &mut String,
        pending_newlines: &mut usize,
        output: &mut String,
        expanding: &mut crate::common::fx_hash::FxHashSet<String>,
    ) {
        if pending_line.is_empty() {
            if Self::has_unbalanced_parens(line) {
                *pending_line = line.to_string();
                *pending_newlines = 1;
                pending_starts.clear();
                pending_starts.push((0, source_line_num));
            } else if self.ends_with_funclike_macro(line) {
                // Line ends with a function-like macro name without '(' on same line.
                // Per C standard, whitespace (including newlines) between macro name
                // and '(' is allowed, so accumulate to check next line for '('.
                *pending_line = line.to_string();
                *pending_newlines = 1;
                pending_starts.clear();
                pending_starts.push((0, source_line_num));
            } else {
                let expanded = self.macros.expand_line_reuse(line, expanding);
                // _Pragma operators queued by this expansion are drained by
                // the caller (this method holds &self; pragma execution needs
                // &mut). They take effect before the next line is processed.
                // After expansion, check if the expanded result ends with a
                // function-like macro name. This handles chained macros like:
                //   #define STEP1(x) x STEP2
                //   #define STEP2(y) y STEP3
                //   STEP1(int)    <- expands to "int STEP2"
                //   (foo)         <- should be STEP2's argument
                // The original source line doesn't end with a func-like macro,
                // but the expanded result does. We need to accumulate the next
                // line so STEP2 can find its '(' argument.
                if self.ends_with_funclike_macro(&expanded) {
                    *pending_line = line.to_string();
                    *pending_newlines = 1;
                    pending_starts.clear();
                    pending_starts.push((0, source_line_num));
                } else {
                    let expanded = self.resolve_has_macros_in_code(&expanded, source_line_num + 1);
                    output.push_str(&expanded);
                    output.push('\n');
                }
            }
        } else {
            // Check if this continuation line starts with '(' (after whitespace)
            // when we were accumulating for a trailing function-like macro name.
            let needs_more = Self::has_unbalanced_parens(pending_line);
            pending_line.push('\n');
            pending_starts.push((pending_line.len(), source_line_num));
            pending_line.push_str(line);
            *pending_newlines += 1;

            if needs_more {
                // Was accumulating for unbalanced parens
                if !Self::has_unbalanced_parens(pending_line)
                    || *pending_newlines > MAX_PENDING_NEWLINES
                {
                    // Accumulated text spans several physical lines: expand
                    // against a flat resolver (the current line).
                    // Flat resolver: per-offset resolution does not apply
                    // to the multi-line accumulated buffer; the offset base
                    // is irrelevant.
                    self.macros.set_line_resolver(
                        Self::accumulated_resolver(pending_starts),
                        self.line_override,
                        0,
                    );
                    let expanded = self.macros.expand_line_reuse(pending_line, expanding);
                    // Check if the expanded result ends with a function-like macro
                    // that needs args from the next line (chained macros).
                    if self.ends_with_funclike_macro(&expanded)
                        && *pending_newlines <= MAX_PENDING_NEWLINES
                    {
                        // Don't clear pending_line - keep accumulating
                    } else {
                        let expanded =
                            self.resolve_has_macros_in_code(&expanded, source_line_num + 1);
                        Self::push_accumulated(output, &expanded, *pending_newlines);
                        pending_line.clear();
                        *pending_newlines = 0;
                    }
                }
            } else {
                // Was accumulating for trailing function-like macro name.
                // Now we have the next line joined. Check if parens are balanced.
                if Self::has_unbalanced_parens(pending_line)
                    && *pending_newlines <= MAX_PENDING_NEWLINES
                {
                    // The joined text has unbalanced parens (macro args span more lines)
                    // Keep accumulating.
                } else {
                    // Parens balanced or next line didn't start with '(' - expand now.
                    // Accumulated text spans several physical lines: expand
                    // against a flat resolver (the current line).
                    // Flat resolver: per-offset resolution does not apply
                    // to the multi-line accumulated buffer; the offset base
                    // is irrelevant.
                    self.macros.set_line_resolver(
                        Self::accumulated_resolver(pending_starts),
                        self.line_override,
                        0,
                    );
                    let expanded = self.macros.expand_line_reuse(pending_line, expanding);
                    // Check if the expanded result itself ends with a function-like
                    // macro name that needs args from the next line (chained macros).
                    if self.ends_with_funclike_macro(&expanded)
                        && *pending_newlines <= MAX_PENDING_NEWLINES
                    {
                        // Don't clear pending_line - keep accumulating
                    } else {
                        let expanded =
                            self.resolve_has_macros_in_code(&expanded, source_line_num + 1);
                        Self::push_accumulated(output, &expanded, *pending_newlines);
                        pending_line.clear();
                        *pending_newlines = 0;
                    }
                }
            }
        }
    }

    /// Account for `n` source lines that belong to an accumulated multi-line
    /// invocation without contributing text (directives, inactive lines,
    /// lines merged into the previous one): as empty lines, so the line
    /// breaks the expander re-emits after the invocation include them.
    fn pend_blank(pending_line: &mut String, pending_newlines: &mut usize, n: usize) {
        pending_line.extend(std::iter::repeat_n('\n', n));
        *pending_newlines += n;
    }

    /// Resolver for an accumulated multi-line buffer from the start offset
    /// and physical line of each of its lines. Comments and splices on
    /// ordinary lines no longer merge lines, so a line's start is exact for
    /// its bytes.
    fn accumulated_resolver(starts: &[(usize, usize)]) -> LineResolver {
        match starts {
            [] => LineResolver::Flat(0),
            [(_, line)] => LineResolver::Flat(*line),
            _ => LineResolver::Mapped(Rc::new(LineResolverData {
                points: starts.iter().map(|&(off, line)| (off, off, line)).collect(),
                // Empty: the points carry physical lines.
                join_map: Rc::from([]),
            })),
        }
    }

    /// Append the expansion of an accumulated multi-line invocation that
    /// stood for `lines` physical lines, ending in exactly `lines` newlines:
    /// the line breaks that survive expansion (those outside macro
    /// arguments) count, and blank lines make up the rest, so the next line
    /// keeps its physical line number.
    fn push_accumulated(output: &mut String, expanded: &str, lines: usize) {
        output.push_str(expanded);
        let kept = expanded.bytes().filter(|&b| b == b'\n').count();
        output.extend(std::iter::repeat_n('\n', lines.saturating_sub(kept).max(1)));
    }

    /// Check if a line ends with an identifier that is a defined function-like macro.
    /// This is used to detect cases where the macro arguments '(' might be on the next line.
    fn ends_with_funclike_macro(&self, line: &str) -> bool {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            return false;
        }
        // Extract the last identifier from the line
        let bytes = trimmed.as_bytes();
        let end = bytes.len();
        // Walk backwards to find end of last identifier
        if !is_ident_cont(bytes[end - 1] as char) {
            return false;
        }
        let mut start = end - 1;
        while start > 0 && is_ident_cont(bytes[start - 1] as char) {
            start -= 1;
        }
        // Check that the identifier starts with a valid start character
        if !is_ident_start(bytes[start] as char) {
            return false;
        }
        let ident = &trimmed[start..end];
        // Check if this identifier is a defined function-like macro
        if let Some(mac) = self.macros.get(ident) {
            mac.is_function_like
        } else {
            false
        }
    }

    /// Enable assembly preprocessing mode. In this mode, '$' is not treated
    /// as an identifier character during macro expansion, so that AT&T assembly
    /// immediates like `$MACRO_NAME` correctly expand the macro.
    pub fn set_asm_mode(&mut self, asm_mode: bool) {
        self.macros.asm_mode = asm_mode;
    }

    /// Select C23 preprocessing: `true` and `false` are keywords there, and
    /// in an `#if` they evaluate to 1 and 0 (C23 6.10.2p6). Before C23 they
    /// are ordinary identifiers, which evaluate to 0 unless `<stdbool.h>`
    /// defined them as macros (measured on GCC 16.2 and Clang 23.1.3).
    pub fn set_c23(&mut self, c23: bool) {
        super::utils::set_c23_dialect(c23);
    }

    /// Set the filename for __FILE__ and __BASE_FILE__ macros and set as the base include directory.
    pub fn set_filename(&mut self, filename: &str) {
        self.filename = filename.to_string();
        self.macros.set_file(format!("\"{}\"", filename));
        // __BASE_FILE__ always expands to the main input file name,
        // unlike __FILE__ which changes during #include processing.
        self.macros.define(MacroDef {
            name: "__BASE_FILE__".to_string(),
            is_function_like: false,
            params: Vec::new(),
            is_variadic: false,
            has_named_variadic: false,
            body: format!("\"{}\"", filename),
        });
        // Push the file path onto the include stack for relative includes.
        // Use make_absolute (not canonicalize) to preserve symlinks, matching GCC
        // behavior: #include "..." searches relative to the directory where the
        // including file was found (through symlinks), not the symlink target.
        let path = PathBuf::from(filename);
        let abs = super::includes::make_absolute(&path);
        self.include_stack.push(abs);
    }

    /// Get preprocessing errors.
    pub fn errors(&self) -> &[PreprocessorDiagnostic] {
        &self.errors
    }

    /// Get preprocessing warnings.
    pub fn warnings(&self) -> &[PreprocessorDiagnostic] {
        &self.warnings
    }

    /// Get the current source file name for diagnostic purposes.
    /// Returns the top of the include stack (the file currently being preprocessed),
    /// falling back to `self.filename` for the main translation unit.
    pub(super) fn current_file(&self) -> String {
        self.include_stack
            .last()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| self.filename.clone())
    }

    /// Take macro expansion metadata collected during preprocessing.
    /// This metadata maps preprocessed output line numbers to the macros
    /// that were expanded on each line, for use in diagnostic rendering.
    /// Take the collected make-dependency file list (first-open order,
    /// deduped, each with its system-directory verdict), leaving the
    /// preprocessor with an empty list.
    pub fn take_dep_files(&mut self) -> Vec<(PathBuf, bool)> {
        std::mem::take(&mut self.dep_files)
    }

    pub fn take_macro_expansion_info(&mut self) -> Vec<crate::common::source::MacroExpansionInfo> {
        std::mem::take(&mut self.macro_expansion_info)
    }

    /// Dump all macro definitions in GCC `-dM` format.
    ///
    /// Returns a string with one `#define` per line for every currently-defined
    /// macro, sorted alphabetically. Function-like macros include their parameter
    /// lists. This output is used by build systems like Meson to detect compiler
    /// features via preprocessor defines (e.g., `__GNUC__`, `__GNUC_MINOR__`).
    pub fn dump_defines(&self) -> String {
        let mut defs: Vec<String> = self
            .macros
            .iter()
            .map(|def| {
                if def.is_function_like {
                    let params = def.params.join(",");
                    if def.body.is_empty() {
                        format!("#define {}({})", def.name, params)
                    } else {
                        format!("#define {}({}) {}", def.name, params, def.body)
                    }
                } else if def.body.is_empty() {
                    format!("#define {}", def.name)
                } else {
                    format!("#define {} {}", def.name, def.body)
                }
            })
            .collect();
        defs.sort();
        defs.join("\n")
    }

    /// Define a macro from a command-line -D flag.
    /// Takes a name and value (e.g., name="FOO", value="1").
    ///
    /// `-D name(args)=body` is a function-like definition: GCC processes the
    /// `-D` text "as if it appeared in a `#define` directive", so
    /// `-D'P(x)=L_##x'` defines the two-token-pasting macro `P`, not an
    /// object-like macro literally named `P(x)` (which could never be
    /// invoked and left every `P(...)` in the source unexpanded).
    pub fn define_macro(&mut self, name: &str, value: &str) {
        if name.contains('(') {
            if let Some(def) = parse_define(&format!("{name} {value}")) {
                self.macros.define(def);
                return;
            }
        }
        self.macros.define(MacroDef {
            name: name.to_string(),
            is_function_like: false,
            params: Vec::new(),
            is_variadic: false,
            has_named_variadic: false,
            body: value.to_string(),
        });
    }

    /// Undefine a macro by name.
    pub fn undefine_macro(&mut self, name: &str) {
        self.macros.undefine(name);
    }

    /// Add an include search path for #include directives (-I flag).
    /// Adds regardless of whether the directory currently exists.
    pub fn add_include_path(&mut self, path: &str) {
        self.include_paths.push(PathBuf::from(path));
    }

    /// Add a quote-only include search path (-iquote flag).
    /// These paths are searched only for `#include "file"`, not `#include <file>`.
    /// Searched after current directory, before -I paths.
    pub fn add_quote_include_path(&mut self, path: &str) {
        self.quote_include_paths.push(PathBuf::from(path));
    }

    /// Add a system include path (-isystem flag).
    /// These paths are searched after -I paths, before default system paths.
    pub fn add_system_include_path(&mut self, path: &str) {
        self.isystem_include_paths.push(PathBuf::from(path));
    }

    /// Add an after include path (-idirafter flag).
    /// These paths are searched last, after all other include paths.
    pub fn add_after_include_path(&mut self, path: &str) {
        self.after_include_paths.push(PathBuf::from(path));
    }

    /// Process a force-included file (-include flag). This preprocesses the file content
    /// as if it were #include'd at the very beginning of the main source file.
    /// All #define directives in the file take effect, and the preprocessed output
    /// is discarded (macros/typedefs persist in the preprocessor state).
    pub fn preprocess_force_include(&mut self, content: &str, resolved_path: &str) {
        let resolved = PathBuf::from(resolved_path);

        // GCC lists -include files in make dependencies even when a guard
        // or #pragma once skips reprocessing (the file WAS opened), as a
        // user-directory dependency.
        self.record_dep_file(resolved.clone(), false);

        // Check for #pragma once (path and device/inode identity).
        if self.is_pragma_once_file(&resolved) {
            return;
        }

        // Check for include guard
        if let Some(guard) = self.include_guard_macros.get(&resolved) {
            if self.macros.is_defined(guard) {
                return;
            }
        }

        // Push onto include stack
        self.include_stack.push(resolved.clone());

        // Save and set __FILE__ (uses set_file to avoid full MacroDef allocation)
        let old_file = self.macros.get_file_body().map(|s| s.to_string());
        self.macros.set_file(format!("\"{}\"", resolved.display()));

        // Preprocess the included content (macros persist; any pragma synthetic tokens
        // like __ccc_visibility_push_hidden are collected and prepended to main output)
        let output = self.preprocess_included(content);
        // Collect any non-whitespace output (e.g., pragma synthetic tokens) for prepending
        // to the main source's preprocessed output. This ensures that pragmas like
        // #pragma GCC visibility push(hidden) in force-included files take effect.
        let trimmed = output.trim();
        if !trimmed.is_empty() {
            self.force_include_output.push_str(trimmed);
            self.force_include_output.push('\n');
        }

        // Restore __FILE__
        if let Some(old) = old_file {
            self.macros.set_file(old);
        }

        // Pop include stack
        self.include_stack.pop();
    }

    /// Process a preprocessor directive line.
    /// Returns Some(content) if an #include was processed and should be inserted.
    /// `line_num` is the 1-based source line number, `col` is the 1-based column
    /// of the `#` character (used for diagnostic source locations).
    /// Execute `_Pragma("...")` operators queued by the last macro expansion
    /// (C99 §6.10.9: `_Pragma("foo")` behaves like `#pragma foo` at that
    /// point). Called after every expansion pass so pragma effects
    /// (push_macro/pop_macro, weak, visibility, ...) apply in source order.
    fn drain_expansion_pragmas(&mut self) {
        for (content, line) in self.macros.take_pending_pragmas() {
            let content = content.trim().to_string();
            if content.is_empty() {
                continue;
            }
            // The line was resolved at enqueue time against the expansion
            // site's own line resolver (C99 §6.10.9: the pragma behaves
            // as if written where the `_Pragma` operator appears).
            self.handle_pragma(&content, line, 0);
        }
    }

    fn process_directive(&mut self, line: &str, line_num: usize, col: usize) -> Option<String> {
        // Strip leading # and whitespace
        let after_hash = line.trim_start_matches('#').trim();

        // Strip trailing comments (// style)
        let after_hash = strip_line_comment(after_hash);

        // Get directive keyword
        let (keyword, rest) = split_first_word(&after_hash);

        // Handle #include<file> and #include"file" (no space between include and path)
        // This handles the common C pattern: #include<stdio.h>
        let (keyword, rest) = if keyword.starts_with("include<") || keyword.starts_with("include\"")
        {
            ("include", &after_hash["include".len()..])
        } else if keyword.starts_with("include_next<") || keyword.starts_with("include_next\"") {
            ("include_next", &after_hash["include_next".len()..])
        } else {
            (keyword, rest)
        };

        // Some directives are processed even in inactive conditional blocks
        match keyword {
            "ifdef" | "ifndef" | "if" => {
                if !self.conditionals.is_active() {
                    // In an inactive block, just push a nested inactive conditional
                    self.conditionals.push_if(false);
                    return None;
                }
            }
            "elif" => {
                self.handle_elif(rest, line_num);
                return None;
            }
            // C23 6.10.2; GCC accepts both in every dialect.
            "elifdef" | "elifndef" => {
                self.handle_elifdef(rest, keyword == "elifndef");
                return None;
            }
            "else" => {
                self.conditionals.handle_else();
                return None;
            }
            "endif" => {
                self.conditionals.handle_endif();
                return None;
            }
            _ => {
                // Other directives only processed in active blocks
                if !self.conditionals.is_active() {
                    return None;
                }
            }
        }

        // Process directive in active block
        match keyword {
            "include" => {
                return self.handle_include(rest, line_num, col);
            }
            "include_next" => {
                // GCC extension: include_next searches from the next path after the
                // current file's directory in the include search list
                return self.handle_include_next(rest, line_num, col);
            }
            "embed" => {
                // C23 #embed: the directive line is replaced by the byte-list
                // token sequence.  The caller treats Some(_) as injected
                // content and appends the return line marker, so end the
                // expansion with a newline to keep the marker on its own line.
                return self.expand_embed(rest, line_num, col).map(|mut t| {
                    t.push('\n');
                    t
                });
            }
            "define" => self.handle_define(rest, line_num, col),
            "undef" => self.handle_undef(rest),
            "ifdef" => self.handle_ifdef(rest, false),
            "ifndef" => self.handle_ifdef(rest, true),
            "if" => self.handle_if(rest, line_num),
            "pragma" => {
                return self.handle_pragma(rest, line_num, col);
            }
            "error" => {
                // C11 6.10.5p1 / C23 6.10.6p1: preprocessing tokens in #error
                // are not subject to macro expansion.
                self.errors.push(PreprocessorDiagnostic {
                    file: self.current_file(),
                    line: line_num,
                    col,
                    message: format!("#error {}", rest),
                });
            }
            "warning" => {
                // GCC extension, collect warning for structured diagnostic output
                self.warnings.push(PreprocessorDiagnostic {
                    file: self.current_file(),
                    line: line_num,
                    col,
                    message: format!("#warning {}", rest),
                });
            }
            "line" => {
                self.handle_line_directive(rest, line_num);
            }
            "" => {
                // Empty # directive (null directive), valid in C
            }
            _ => {
                // Handle GNU linemarker: # <digit-sequence> ["filename" [flags]]
                // This is equivalent to #line <digit-sequence> ["filename"]
                if keyword.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    let line_rest = format!("{} {}", keyword, rest);
                    self.handle_line_directive(&line_rest, line_num);
                } else {
                    // Unknown directives are valid preprocessing diagnostics, not
                    // hard errors.  Warn so typos like `#defin` and `#inclde`
                    // are visible while preserving compatibility with projects
                    // that intentionally use vendor-specific directives.
                    self.warnings.push(PreprocessorDiagnostic {
                        file: self.current_file(),
                        line: line_num,
                        col,
                        message: format!("unknown preprocessing directive #{}", keyword),
                    });
                }
            }
        }

        None
    }

    /// C23 6.10.15 makes `#embed` the one directive that may appear MID
    /// line (`unsigned char d[] = {#embed "logo.bin"};`) — the standard
    /// form, not an extension. When the line holds such an occurrence,
    /// replace it (directive text runs to end of line) and hand the
    /// spliced line to the ordinary macro-expansion path. Returns None
    /// when the line has no inline `#embed`.
    fn splice_inline_embed(&mut self, line: &str, source_line_num: usize) -> Option<String> {
        // Fast path: no candidate text at all.
        if !line.contains("embed") {
            return None;
        }
        // SINGLE scan: the first match is reused, not discarded and
        // re-found on a cloned line as before.
        let (first_hash, first_kw_end) = Self::find_inline_embed(line)?;
        // A line may carry SEVERAL inline embeds
        // (`{#embed "a"}, {#embed "b"}`); splice them left to right.
        let mut out = String::with_capacity(line.len());
        out.push_str(&line[..first_hash]);
        // `rest` owns the scannable tail from the FIRST `#` onward (the
        // splice loop needs an owned buffer because expand_embed_inline
        // hands back the unconsumed suffix by value); `base` is its offset
        // in the original line so diagnostic columns stay exact across
        // splice boundaries.
        let mut rest = line[first_hash..].to_string();
        let mut base = first_hash;
        // First occurrence already located at (0, first_kw_end - first_hash)
        // within `rest`; subsequent occurrences found in the remainder.
        let mut next = Some((0usize, first_kw_end - first_hash));
        while let Some((hash_pos, kw_end)) = next {
            out.push_str(&rest[..hash_pos]);
            // Detach the spec text: expand_embed_inline needs &mut self.
            let spec_text = rest[kw_end..].to_string();
            match self.expand_embed_inline(&spec_text, source_line_num + 1, base + hash_pos + 1) {
                Some((t, remainder)) => {
                    out.push_str(&t);
                    // Continue scanning ONLY the unconsumed source tail.
                    // The expansion is data: never re-scan it (a prefix or
                    // suffix containing `#embed` must not recurse).
                    // `remainder` is a suffix of the spec text, so the new
                    // buffer offset follows from the length difference.
                    base += rest.len() - remainder.len();
                    rest = remainder;
                    next = Self::find_inline_embed(&rest);
                }
                None => {
                    // Spec error diagnosed: the directive swallows the rest
                    // of the line, so drop the tail like a whole-line
                    // directive would.
                    return Some(out);
                }
            }
        }
        out.push_str(&rest);
        Some(out)
    }

    /// Locate an inline `#embed` occurrence: returns (hash_pos, kw_end)
    /// with kw_end one past the `embed` keyword. String/char literals are
    /// skipped (a `#embed` inside a literal is ordinary text); `#embedded`
    /// and friends do not match (keyword boundary check).
    fn find_inline_embed(text: &str) -> Option<(usize, usize)> {
        let bytes = text.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'"' || c == b'\'' {
                let quote = c;
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                continue;
            }
            if c == b'#' {
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                if j + 5 <= bytes.len()
                    && &bytes[j..j + 5] == b"embed"
                    && (j + 5 == bytes.len()
                        || !(bytes[j + 5].is_ascii_alphanumeric() || bytes[j + 5] == b'_'))
                {
                    return Some((i, j + 5));
                }
            }
            i += 1;
        }
        None
    }

    fn handle_define(&mut self, rest: &str, line_num: usize, col: usize) {
        if let Some(def) = parse_define(rest) {
            self.validate_macro_stringize(&def, line_num, col);
            self.macros.define(def);
        }
    }

    /// C (and C++) constrain the `#` stringize operator: in a function-like
    /// macro it must be followed by one of the macro's parameters.  GCC
    /// diagnoses this at DEFINE time ("'#' is not followed by a macro
    /// parameter"), and the corpus pins exactly that — silently accepting it
    /// would let garbage bodies survive to expansion.
    fn validate_macro_stringize(&mut self, def: &MacroDef, line_num: usize, col: usize) {
        if !def.is_function_like {
            return;
        }
        let bytes = def.body.as_bytes();
        let len = bytes.len();
        let mut i = 0usize;
        while i < len {
            let c = bytes[i];
            // Skip string and char literals: a '#' inside a literal is
            // ordinary text, not the stringize operator (#define F(x) "#"
            // and #define G(x) '#' are both valid).
            if c == b'"' || c == b'\'' {
                let quote = c;
                i += 1;
                while i < len && bytes[i] != quote {
                    if bytes[i] == b'\\' && i + 1 < len {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                continue;
            }
            if c != b'#' {
                i += 1;
                continue;
            }
            // '##' is token pasting, not stringize: skip both.
            if i + 1 < len && bytes[i + 1] == b'#' {
                i += 2;
                continue;
            }
            let mut j = i + 1;
            while j < len && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            let start = j;
            while j < len && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let operand = &def.body[start..j];
            let valid = !operand.is_empty()
                && (def.params.iter().any(|p| p == operand)
                    || (def.is_variadic && operand == "__VA_ARGS__"));
            if !valid {
                self.errors.push(PreprocessorDiagnostic {
                    file: self.current_file(),
                    line: line_num,
                    col,
                    message: "'#' is not followed by a macro parameter".to_string(),
                });
                return; // one diagnostic per definition, like GCC
            }
            i = j;
        }
    }

    fn handle_undef(&mut self, rest: &str) {
        let name = rest.split_whitespace().next().unwrap_or("");
        if !name.is_empty() {
            self.macros.undefine(name);
        }
    }

    fn handle_ifdef(&mut self, rest: &str, negate: bool) {
        let name = rest.split_whitespace().next().unwrap_or("");
        let defined = self.macros.is_defined(name);
        let condition = if negate { !defined } else { defined };
        self.conditionals.push_if(condition);
    }

    fn handle_if(&mut self, expr: &str, diag_line: usize) {
        // First resolve `defined(X)` and `__has_*()` before macro expansion
        let resolved = self.resolve_defined_in_expr(expr, diag_line);
        // Expand macros in the resolved expression (reuse directive_expanding set)
        let expanded = self
            .macros
            .expand_line_reuse(&resolved, &mut self.directive_expanding);
        // Resolve again after macro expansion, in case macros expanded to
        // __has_attribute(), __has_builtin(), __has_include(), etc.
        let expanded = self.resolve_defined_in_expr(&expanded, diag_line);
        // Replace any remaining identifiers with 0 (standard C behavior for #if)
        let final_expr = self.replace_remaining_idents_with_zero(&expanded);
        let condition = evaluate_condition(&final_expr, &self.macros);
        self.conditionals.push_if(condition);
    }

    /// `#elifdef NAME` / `#elifndef NAME`: an `#elif` whose condition is
    /// whether NAME is a macro. The name is only consulted when no earlier
    /// group of the conditional was taken, like `#elif`'s expression.
    fn handle_elifdef(&mut self, rest: &str, negate: bool) {
        let name = rest.split_whitespace().next().unwrap_or("");
        let defined = self.macros.is_defined(name);
        self.conditionals.handle_elif(defined != negate);
    }

    fn handle_elif(&mut self, expr: &str, diag_line: usize) {
        let resolved = self.resolve_defined_in_expr(expr, diag_line);
        let expanded = self
            .macros
            .expand_line_reuse(&resolved, &mut self.directive_expanding);
        // Resolve again after macro expansion (same reason as handle_if)
        let expanded = self.resolve_defined_in_expr(&expanded, diag_line);
        let final_expr = self.replace_remaining_idents_with_zero(&expanded);
        let condition = evaluate_condition(&final_expr, &self.macros);
        self.conditionals.handle_elif(condition);
    }

    fn handle_line_directive(&mut self, rest: &str, source_line_num: usize) {
        // #line digit-sequence ["filename"]
        // The argument undergoes macro expansion first
        let expanded = self
            .macros
            .expand_line_reuse(rest, &mut self.directive_expanding);
        let expanded = expanded.trim();

        // Parse the line number (first token)
        let mut parts = expanded.split_whitespace();
        if let Some(line_str) = parts.next() {
            if let Ok(target_line) = line_str.parse::<usize>() {
                // source_line_num is 1-based (the line where #line appears)
                // The line AFTER #line should be target_line, so we record:
                // (target_line, source_line_num_0based_of_directive)
                // source_line_num is already 1-based from process_directive caller
                self.line_override = Some((target_line, source_line_num));

                // If there's a filename argument, update __FILE__
                let mut flags = String::new();
                if let Some(filename_str) = parts.next() {
                    let filename = filename_str.trim_matches('"');
                    if !filename.is_empty() {
                        self.macros.set_file(format!("\"{}\"", filename));
                    }
                    // GNU linemarker flags (1 enter, 2 return, 3 system
                    // header, 4 extern "C") carry over to the marker.
                    for flag in parts.filter(|f| matches!(*f, "1" | "2" | "3" | "4")) {
                        flags.push(' ');
                        flags.push_str(flag);
                    }
                }
                self.line_marker_due = Some(flags);
            }
        }
    }
}

impl Default for Preprocessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod line_resolver_share_tests {
    use super::*;
    use std::time::Instant;

    /// One splice so `join_map` is `Some` (mid-token: a splice at white
    /// space on an ordinary line is kept as a line break, without a map);
    /// one multi-line comment so `comment_map` is `Some` (in a directive:
    /// on an ordinary line it is split, without a map); then `n` ordinary
    /// lines. The old per-line
    /// `Rc::from(join_map)` is Θ(n²) and hung sqlite3.c (~270k lines) in
    /// `build_line_resolver` for minutes. After the shared-Rc fix this is
    /// linear and must finish well under a second on the research VM.
    fn spliced_commented_source(n: usize) -> String {
        let mut src = String::with_capacity(n * 12 + 64);
        src.push_str("int a = 1\\\n2;\n");
        src.push_str("#define M 1 /* multi\n   line comment */\n");
        for i in 0..n {
            src.push_str("int x");
            src.push_str(&i.to_string());
            src.push_str(";\n");
        }
        src.push_str("int line = __LINE__;\n");
        src
    }

    #[test]
    fn build_line_resolver_is_linear_on_large_spliced_commented_file() {
        let n = 12_000usize;
        let src = spliced_commented_source(n);
        let mut pp = Preprocessor::new();
        pp.set_filename("quadratic.c");
        let t0 = Instant::now();
        let out = pp.preprocess(&src);
        let dt = t0.elapsed();
        assert!(
            dt.as_millis() < 4_000,
            "preprocess of {n} spliced+commented lines took {dt:?} \
             (quadratic join-map clone in build_line_resolver?)"
        );
        assert!(out.contains("int a = 12;"), "lost the spliced decl");
        assert!(out.contains("int x0;"), "lost first generated decl");
        assert!(
            out.contains(&format!("int x{};", n - 1)),
            "lost last generated decl"
        );
        assert!(
            pp.errors().is_empty(),
            "unexpected preprocess errors: {:?}",
            pp.errors()
        );
    }

    #[test]
    fn spliced_commented_line_macro_matches_physical_line() {
        // Physical lines:
        //  1: int a = 1 + \
        //  2:  2;
        //  3: /* multi
        //  4:    line comment */
        //  5: int line = __LINE__;
        // `__LINE__` on line 5 must expand to 5, not a joined/stripped index.
        let src = "int a = 1 + \\\n 2;\n/* multi\n   line comment */\nint line = __LINE__;\n";
        let mut pp = Preprocessor::new();
        pp.set_filename("line.c");
        let out = pp.preprocess(src);
        assert!(
            out.contains("int line = 5;"),
            "__LINE__ after splice+comment must be physical 5, got:\n{out}"
        );
    }
}

#[cfg(test)]
mod define_macro_tests {
    use super::*;

    /// `-D'NAME(args)=body'` is a FUNCTION-LIKE definition (GCC: "processed
    /// as if it appeared in a `#define` directive").  The driver splits the
    /// flag text at the first `=`, so `define_macro` receives the parameter
    /// list inside `name`.  Before the fix it defined an object-like macro
    /// literally named `P(x)`, which nothing can invoke, and every `P(...)`
    /// in the source reached the parser unexpanded.
    #[test]
    fn dash_d_function_like_macro_expands_with_token_pasting() {
        let mut pp = Preprocessor::new();
        pp.set_filename("d.c");
        pp.define_macro("P(x)", "L_##x");
        pp.define_macro("ADD(a,b)", "((a)+(b))");
        let out = pp.preprocess("int P(foo) = ADD(1, 2);\n");
        assert!(
            out.contains("int L_foo = ((1)+(2));"),
            "function-like -D macro must expand and paste, got:\n{out}"
        );
    }

    #[test]
    fn dash_d_variadic_function_like_macro() {
        let mut pp = Preprocessor::new();
        pp.set_filename("d.c");
        pp.define_macro("LOG(fmt,...)", "printf(fmt, __VA_ARGS__)");
        let out = pp.preprocess("LOG(\"%d\", 1);\n");
        assert!(out.contains("printf(\"%d\", 1);"), "got:\n{out}");
    }

    /// Object-like `-D` definitions are unchanged, including the GCC
    /// `-DNAME` (no `=`) spelling the driver turns into value `1`, and a
    /// value that itself contains parentheses.
    #[test]
    fn dash_d_object_like_macro_unchanged() {
        let mut pp = Preprocessor::new();
        pp.set_filename("d.c");
        pp.define_macro("FLAG", "1");
        pp.define_macro("EXPR", "(2*3)");
        let out = pp.preprocess("int a = FLAG, b = EXPR;\n");
        assert!(out.contains("int a = 1, b = (2*3);"), "got:\n{out}");
    }

    #[test]
    fn error_directive_does_not_macro_expand_tokens() {
        let mut pp = Preprocessor::new();
        pp.set_filename("err.c");
        let _ = pp.preprocess("#define FOO expanded_wrongly\n#error FOO must stay literal\n");
        assert_eq!(pp.errors().len(), 1);
        assert_eq!(pp.errors()[0].message, "#error FOO must stay literal");
    }
}
