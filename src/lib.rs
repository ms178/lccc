#![recursion_limit = "512"]
#![allow(
    dead_code,
    unused_variables,
    unused_mut,
    unused_assignments,
    unused_imports,
    unreachable_code
)]
//! Clippy policy.
//!
//! This codebase deliberately does not conform to the default `clippy::all` /
//! `clippy::perf` / `clippy::style` / `clippy::complexity` lint set. In a
//! correctness-critical compiler that is fuzzed, differentially tested against
//! GCC, and boot-tested on the Linux kernel, mechanically "fixing" a lint that
//! fires 1000+ times across ~80 categories is churn with real (if small) risk:
//! e.g. `clone_on_copy` rewrites are semantics-preserving by construction, but a
//! `needless_range_loop` or `question_mark` rewrite in an else-if chain is not —
//! and clippy-1.98's own `--fix` emits *non-compiling* `let...else` for the
//! latter, which is why it is suppressed rather than applied wholesale here.
//!
//! The lints below are the exact set the tree violates. They are allowed at the
//! crate root (the same mechanism this file already uses for the rustc lints
//! above) rather than suppressed per-site, so a future contributor who does
//! fix a category can delete its line here and get an immediate, complete list
//! of the remaining sites from a fresh `cargo clippy` run. Generated code is
//! byte-identical with and without this block; it only affects compiler-build
//! diagnostics. CI runs clippy advisory (`continue-on-error: true`).
#![allow(
    clippy::bind_instead_of_map,
    clippy::chunks_exact_to_as_chunks,
    clippy::clone_on_copy,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items,
    clippy::empty_line_after_doc_comments,
    clippy::enum_variant_names,
    clippy::explicit_counter_loop,
    clippy::extend_with_drain,
    clippy::field_reassign_with_default,
    clippy::for_kv_map,
    clippy::if_same_then_else,
    clippy::implicit_saturating_sub,
    clippy::int_plus_one,
    clippy::iter_cloned_collect,
    clippy::large_enum_variant,
    clippy::len_zero,
    clippy::let_and_return,
    clippy::manual_checked_ops,
    clippy::manual_clamp,
    clippy::manual_contains,
    clippy::manual_div_ceil,
    clippy::manual_find,
    clippy::manual_flatten,
    clippy::manual_is_multiple_of,
    clippy::manual_pattern_char_comparison,
    clippy::manual_range_contains,
    clippy::manual_range_patterns,
    clippy::manual_saturating_arithmetic,
    clippy::manual_slice_fill,
    clippy::manual_split_once,
    clippy::manual_strip,
    clippy::map_entry,
    clippy::map_flatten,
    clippy::match_like_matches_macro,
    clippy::match_result_ok,
    clippy::mut_range_bound,
    clippy::needless_borrow,
    clippy::needless_borrows_for_generic_args,
    clippy::needless_ifs,
    clippy::needless_late_init,
    clippy::needless_lifetimes,
    clippy::needless_range_loop,
    clippy::needless_return,
    clippy::new_without_default,
    clippy::op_ref,
    clippy::precedence,
    clippy::ptr_arg,
    clippy::question_mark,
    clippy::redundant_closure,
    clippy::redundant_closure_call,
    clippy::redundant_guards,
    clippy::redundant_locals,
    clippy::redundant_slicing,
    clippy::single_match,
    clippy::slow_vector_initialization,
    clippy::too_many_arguments,
    clippy::trim_split_whitespace,
    clippy::type_complexity,
    clippy::unnecessary_cast,
    clippy::unnecessary_get_then_check,
    clippy::unnecessary_lazy_evaluations,
    clippy::unnecessary_map_or,
    clippy::unnecessary_sort_by,
    clippy::unnecessary_to_owned,
    clippy::unnecessary_unwrap,
    clippy::unneeded_struct_pattern,
    clippy::unneeded_wildcard_pattern,
    clippy::unwrap_or_default,
    clippy::useless_borrows_in_formatting,
    clippy::useless_format,
    clippy::vec_init_then_push,
    clippy::while_let_loop,
    clippy::wrong_self_convention
)]

pub mod backend;
pub(crate) mod common;
pub mod driver;
pub(crate) mod frontend;
pub(crate) mod ir;
pub mod linker_entry;
pub(crate) mod passes;
pub(crate) mod pgo;

/// Shared entry point for all compiler binaries. Spawns the real work on a
/// thread with a large stack so deeply recursive C files don't overflow.
pub fn compiler_main() {
    const STACK_SIZE: usize = 64 * 1024 * 1024; // 64 MB
    let builder = std::thread::Builder::new().stack_size(STACK_SIZE);
    let handler = builder
        .spawn(|| {
            let args: Vec<String> = std::env::args().collect();
            let mut driver = driver::Driver::new();
            if driver.parse_cli_args(&args)? {
                return Ok(());
            }
            if !driver.has_input_files() {
                return Err("no input files".to_string());
            }
            driver.run()
        })
        .expect("failed to spawn main thread");

    match handler.join() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            eprintln!("ccc: error: {}", e);
            std::process::exit(1);
        }
        Err(e) => {
            if let Some(s) = e.downcast_ref::<&str>() {
                eprintln!("ccc: internal error: {}", s);
            } else if let Some(s) = e.downcast_ref::<String>() {
                eprintln!("ccc: internal error: {}", s);
            } else {
                eprintln!("ccc: internal error (thread panicked)");
            }
            std::process::exit(1);
        }
    }
}

/// Print the MachInst instruction-selection coverage census (no-op unless
/// `CCC_ISEL_STATS=1`). Exposed for the driver binary.
pub fn isel_stats_report() {
    crate::backend::x86::codegen::isel::stats::report();
}
