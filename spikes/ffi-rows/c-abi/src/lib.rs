use std::ffi::{c_char, CStr};
use std::fs;

pub struct Rom { bytes: Vec<u8> }

/// Fixed-width row record: address (4) + 16 bytes + 16 ascii = 36 bytes.
#[repr(C)]
pub struct RowRec { pub file_offset: u32, pub bytes: [u8; 16], pub ascii: [u8; 16] }

#[repr(C)]
pub struct RowBatch { pub ptr: *mut RowRec, pub len: usize, cap: usize }

#[no_mangle]
pub extern "C" fn spike_open(path: *const c_char) -> *mut Rom {
    let p = unsafe { CStr::from_ptr(path) }.to_str().unwrap();
    match fs::read(p) { Ok(b) => Box::into_raw(Box::new(Rom { bytes: b })), Err(_) => std::ptr::null_mut() }
}
#[no_mangle]
pub extern "C" fn spike_close(r: *mut Rom) { if !r.is_null() { unsafe { drop(Box::from_raw(r)); } } }
#[no_mangle]
pub extern "C" fn spike_size(r: *const Rom) -> usize { unsafe { (*r).bytes.len() } }

/// Chatty: one call per byte.
#[no_mangle]
pub extern "C" fn spike_byte(r: *const Rom, off: usize) -> u8 { let rom = unsafe { &*r }; *rom.bytes.get(off).unwrap_or(&0) }

/// Chunky, caller-owned buffer: fills up to `count` RowRecs starting at `start_row`. Returns rows written.
#[no_mangle]
pub extern "C" fn spike_rows_into(r: *const Rom, start_row: usize, count: usize, out: *mut RowRec) -> usize {
    let rom = unsafe { &*r };
    let total_rows = (rom.bytes.len() + 15) / 16;
    let n = count.min(total_rows.saturating_sub(start_row));
    let out = unsafe { std::slice::from_raw_parts_mut(out, n) };
    for (i, rec) in out.iter_mut().enumerate() {
        let off = (start_row + i) * 16;
        rec.file_offset = off as u32;
        let src = &rom.bytes[off..(off + 16).min(rom.bytes.len())];
        rec.bytes = [0; 16]; rec.bytes[..src.len()].copy_from_slice(src);
        for (k, b) in src.iter().enumerate() { rec.ascii[k] = if (32..127).contains(b) { *b } else { b'.' }; }
    }
    n
}

/// Chunky, Rust-owned buffer (what a binding generator does): allocate, return, free later.
#[no_mangle]
pub extern "C" fn spike_rows_alloc(r: *const Rom, start_row: usize, count: usize) -> RowBatch {
    let mut v: Vec<RowRec> = Vec::with_capacity(count);
    unsafe { v.set_len(count); }
    let n = spike_rows_into(r, start_row, count, v.as_mut_ptr());
    v.truncate(n);
    let mut v = std::mem::ManuallyDrop::new(v);
    RowBatch { ptr: v.as_mut_ptr(), len: v.len(), cap: v.capacity() }
}
#[no_mangle]
pub extern "C" fn spike_rows_free(b: RowBatch) { unsafe { drop(Vec::from_raw_parts(b.ptr, b.len, b.cap)); } }

/// Pre-formatted text rows (UTF-8, newline separated) to measure the "core formats strings" option.
#[no_mangle]
pub extern "C" fn spike_rows_text(r: *const Rom, start_row: usize, count: usize, out_len: *mut usize) -> *mut u8 {
    use std::fmt::Write;
    let rom = unsafe { &*r };
    let mut s = String::with_capacity(count * 80);
    let total_rows = (rom.bytes.len() + 15) / 16;
    let n = count.min(total_rows.saturating_sub(start_row));
    for i in 0..n {
        let off = (start_row + i) * 16;
        let src = &rom.bytes[off..(off + 16).min(rom.bytes.len())];
        let _ = write!(s, "{:06X}  ", off);
        for b in src { let _ = write!(s, "{:02X} ", b); }
        s.push(' ');
        for b in src { s.push(if (32..127).contains(b) { *b as char } else { '.' }); }
        s.push('\n');
    }
    let mut v = std::mem::ManuallyDrop::new(s.into_bytes());
    unsafe { *out_len = v.len(); }
    v.shrink_to_fit();
    v.as_mut_ptr()
}
#[no_mangle]
pub extern "C" fn spike_text_free(p: *mut u8, len: usize) { unsafe { drop(Vec::from_raw_parts(p, len, len)); } }
