#![recursion_limit = "512"]
#![allow(
    dead_code,
    unused_variables,
    unused_mut,
    unused_assignments,
    unused_imports,
    unreachable_code
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
