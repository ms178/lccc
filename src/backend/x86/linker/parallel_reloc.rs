//! Parallel relocation application infrastructure.
#![allow(dead_code)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

#[derive(Clone, Copy, Debug)]
pub struct RelocWrite { pub offset: usize, pub width: u8, pub value: u64, }

#[derive(Clone, Debug)]
pub struct InsnRewrite { pub offset: usize, pub bytes: Vec<u8>, }

pub fn apply_writes_parallel(out: &mut [u8], writes: &[RelocWrite], n_threads: usize) {
    if writes.is_empty() || n_threads <= 1 {
        apply_writes_serial(out, writes);
        return;
    }
    let n_threads = n_threads.min(writes.len()).max(1);
    let chunk = (writes.len() + n_threads - 1) / n_threads;
    thread::scope(|scope| {
        let out_addr = out.as_mut_ptr() as usize;
        let out_len = out.len();
        let mut handles = Vec::new();
        for t in 0..n_threads {
            let start = t * chunk;
            if start >= writes.len() { break; }
            let end = (start + chunk).min(writes.len());
            let slice = &writes[start..end];
            if slice.is_empty() { continue; }
            let min_off = slice.iter().map(|w| w.offset).min().unwrap();
            let max_off = slice.iter().map(|w| w.offset + w.width as usize).max().unwrap();
            assert!(max_off <= out_len);
            let thread_slice = unsafe {
                std::slice::from_raw_parts_mut((out_addr as *mut u8).add(min_off), max_off - min_off)
            };
            handles.push(scope.spawn(move || {
                for w in slice {
                    let local = w.offset - min_off;
                    match w.width {
                        1 => thread_slice[local] = w.value as u8,
                        2 => thread_slice[local..local+2].copy_from_slice(&(w.value as u16).to_le_bytes()),
                        4 => thread_slice[local..local+4].copy_from_slice(&(w.value as u32).to_le_bytes()),
                        8 => thread_slice[local..local+8].copy_from_slice(&w.value.to_le_bytes()),
                        _ => {}
                    }
                }
            }));
        }
        for h in handles { let _ = h.join(); }
    });
}

pub fn apply_writes_serial(out: &mut [u8], writes: &[RelocWrite]) {
    for w in writes {
        if w.offset + w.width as usize > out.len() { continue; }
        match w.width {
            1 => out[w.offset] = w.value as u8,
            2 => out[w.offset..w.offset+2].copy_from_slice(&(w.value as u16).to_le_bytes()),
            4 => out[w.offset..w.offset+4].copy_from_slice(&(w.value as u32).to_le_bytes()),
            8 => out[w.offset..w.offset+8].copy_from_slice(&w.value.to_le_bytes()),
            _ => {}
        }
    }
}

pub fn apply_rewrites(out: &mut [u8], rewrites: &[InsnRewrite]) {
    for r in rewrites {
        if r.offset + r.bytes.len() <= out.len() {
            out[r.offset..r.offset + r.bytes.len()].copy_from_slice(&r.bytes);
        }
    }
}

pub fn desired_threads() -> usize {
    match std::env::var("LCCC_LD_PARALLEL") {
        Ok(s) if s == "0" || s.eq_ignore_ascii_case("false") => 1,
        Ok(s) if s == "1" || s.eq_ignore_ascii_case("true") => {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).min(8)
        }
        Ok(s) => s.parse().unwrap_or(1).max(1).min(16),
        Err(_) => 1,
    }
}

pub static PARALLEL_WRITES: AtomicUsize = AtomicUsize::new(0);
pub static SERIAL_WRITES: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serial_and_parallel_agree() {
        let mut out1 = vec![0u8; 256];
        let mut out2 = vec![0u8; 256];
        let writes: Vec<RelocWrite> = (0..32).map(|i| RelocWrite { offset: i*8, width: 8, value: 0x1122334455667788 + i as u64 }).collect();
        apply_writes_serial(&mut out1, &writes);
        apply_writes_parallel(&mut out2, &writes, 4);
        assert_eq!(out1, out2);
    }
}
