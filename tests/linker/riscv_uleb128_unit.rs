// Compile independently with rustc --test; the tested module is production code.
#[path = "../../src/backend/riscv/linker/uleb128.rs"]
mod uleb128;
#[path = "../../src/backend/riscv/linker/displacement.rs"]
mod displacement;
