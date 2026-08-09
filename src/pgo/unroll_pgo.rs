use crate::ir::reexports::IrFunction;
use crate::pgo::ProfileData;
fn size_opt() -> bool {
    std::env::var("CFLAGS")
        .map(|x| x.contains("-Os") || x.contains("-Oz"))
        .unwrap_or(false)
}
pub fn should_unroll_loop(
    f: &IrFunction,
    idx: usize,
    size: usize,
    p: Option<&ProfileData>,
) -> Option<bool> {
    let _ = p?;
    let fp = crate::pgo::active_profile_for_function(f)?;
    let l = f.blocks.get(idx)?.label;
    let n = fp.block_count(l);
    if size_opt() {
        Some(false)
    } else if n > 1000 && size <= 24 {
        Some(true)
    } else if n < 50 {
        Some(false)
    } else {
        None
    }
}
pub fn vectorize_gate(_: &IrFunction, _: Option<&ProfileData>) -> bool {
    true
}
