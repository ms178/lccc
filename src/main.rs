fn main() {
    lccc::compiler_main();
    // Coverage census for the MachInst instruction-selection layer; a no-op
    // unless CCC_ISEL_STATS=1. Printed here so it covers the whole run.
    lccc::isel_stats_report();
}
