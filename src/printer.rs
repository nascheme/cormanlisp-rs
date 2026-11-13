//! Kernel printer: port of upstream `Write.cpp`.
//!
//! Like upstream this is a `princ`-style printer (no escaping of strings,
//! characters or symbols); `write.lisp` replaces it during bootstrap.
#![allow(dead_code)]

use crate::error::lisp_error;
use crate::heap::string_node;
use crate::object::*;
use crate::qv::{nil, qv};
use crate::qv_index::*;
use crate::streams::{self, output_char, output_str, output_units_to};
use crate::symbols::symbol_name;

/// Upstream `_Write`: write `s` to output stream `os`. Returns `s`.
pub fn write(s: LispObj, os: LispObj) -> LispObj {
    if !os.is_stream() {
        lisp_error("Not a stream");
    }
    match s.tag() {
        FIXNUM_TAG => output_integer(s.integer(), os, 10),
        CONS_TAG => output_list(s, os),
        UVECTOR_TAG => output_uvector(s, os),
        FORWARD_TAG => output_str(os, &format!("#< Forward pointer: {:#x} >", s.strip_tag())),
        IMMEDIATE_TAG => output_immediate(s, os),
        UVECTOR_LENGTH_TAG => {
            let ty = (s.raw() >> 3) & 0x1f;
            let size = (s.raw() >> 8) * 16;
            output_str(os, &format!("#< Uvector header: Type = {ty}, Size = {size} bytes >"));
        }
        _ => output_str(os, &format!("#< Unknown object: {:#x} >", s.raw())),
    }
    if s == UNINITIALIZED { nil() } else { s }
}

/// Print to a fresh Rust string.
pub fn write_to_string(s: LispObj) -> String {
    let os = streams::make_string_output_stream();
    write(s, os);
    streams::get_output_stream_string(os).string_to_rust()
}

fn output_uvector(n: LispObj, os: LispObj) {
    match n.uvector_type() {
        FUNCTION_TYPE | KFUNCTION_TYPE => unreadable(n, "FUNCTION", os),
        STRUCTURE_TYPE => output_structure(n, os),
        ARRAY_TYPE => {
            if n.is_string() {
                output_text(n, os)
            } else {
                unreadable(n, "ARRAY", os)
            }
        }
        SYMBOL_TYPE => output_text(symbol_name(n), os),
        STREAM_TYPE => unreadable(n, "STREAM", os),
        DOUBLE_FLOAT_TYPE => output_float(n.double_float(), os),
        SINGLE_FLOAT_TYPE => {
            let f = unsafe { *(n.uvector_ptr().add(1) as *const f32) };
            output_float(f as f64, os)
        }
        RATIO_TYPE => {
            output_integer(n.uref(1).integer(), os, 10);
            output_char(os, b'/' as u16);
            output_integer(n.uref(2).integer(), os, 10);
        }
        PACKAGE_TYPE => {
            output_str(os, "#< PACKAGE: ");
            write(n.uref(PACKAGE_NAME), os);
            output_str(os, " >");
        }
        HASHTABLE_TYPE => unreadable(n, "HASHTABLE", os),
        READ_TABLE_TYPE => unreadable(n, "READTABLE", os),
        SIMPLE_VECTOR_TYPE => unreadable(n, "SIMPLE-VECTOR", os),
        SIMPLE_CHAR_VECTOR_TYPE => output_text(n, os),
        SIMPLE_BYTE_VECTOR_TYPE => unreadable(n, "SIMPLE-BYTE-VECTOR", os),
        SIMPLE_SHORT_VECTOR_TYPE => unreadable(n, "SIMPLE-SHORT-VECTOR", os),
        SIMPLE_DOUBLE_FLOAT_VECTOR_TYPE => unreadable(n, "SIMPLE-DOUBLE-FLOAT-VECTOR", os),
        SIMPLE_BIT_VECTOR_TYPE => unreadable(n, "SIMPLE-BIT-VECTOR", os),
        SIMPLE_SINGLE_FLOAT_VECTOR_TYPE => unreadable(n, "SIMPLE-SINGLE-FLOAT-VECTOR", os),
        // PORT: upstream errors on other types.
        ty => unreadable(n, &format!("UVECTOR-TYPE-{ty}"), os),
    }
}

/// Upstream `outputInteger`.
pub fn output_integer(n: i64, os: LispObj, base: u32) {
    let mut digits = Vec::new();
    let mut m = n.unsigned_abs();
    if m == 0 {
        digits.push(b'0');
    }
    while m > 0 {
        let d = (m % base as u64) as u8;
        m /= base as u64;
        digits.push(if d < 10 { b'0' + d } else { b'A' + d - 10 });
    }
    if n < 0 {
        digits.push(b'-');
    }
    digits.reverse();
    output_str(os, std::str::from_utf8(&digits).unwrap());
}

/// Upstream `outputFloat` uses C `"%f"`.
fn output_float(d: f64, os: LispObj) {
    output_str(os, &format!("{d:.6}"));
}

fn output_text(n: LispObj, os: LispObj) {
    let len = n.vector_length().integer() as usize;
    let units = unsafe { std::slice::from_raw_parts(n.char_array_start(), len) };
    output_units_to(os, units);
}

fn output_immediate(n: LispObj, os: LispObj) {
    if n.is_character() {
        output_char(os, n.char_code() as LispChar);
    } else {
        output_str(os, &format!("#< Immediate object: {:#x} >", n.raw() >> 8));
    }
}

/// Upstream `outputList`.
fn output_list(s0: LispObj, os: LispObj) {
    if s0 == UNINITIALIZED {
        output_str(os, "|--Uninitialized--|");
        return;
    }
    if s0.car() == qv(QUOTE_Index) && s0.cdr().is_cons() {
        output_char(os, b'\'' as u16);
        write(s0.cdr().car(), os);
        return;
    }
    output_char(os, b'(' as u16);
    let mut s = s0;
    let mut count = 0;
    while s.is_cons() {
        if count > 0 {
            output_char(os, b' ' as u16);
        }
        write(s.car(), os);
        s = s.cdr();
        count += 1;
    }
    if s != nil() {
        output_str(os, " . ");
        write(s, os);
    }
    output_char(os, b')' as u16);
}

/// Upstream `outputStructure`.
fn output_structure(n: LispObj, os: LispObj) {
    output_str(os, "#< STRUCTURE: ");
    let len = (n.uvector_size() - 1) * 2;
    for i in 0..len {
        write(n.uref(i + 2), os);
        if i != len - 1 {
            output_char(os, b' ' as u16);
        }
    }
    output_str(os, ">");
}

/// Upstream `outputUnreadableObject`.
fn unreadable(obj: LispObj, name: &str, os: LispObj) {
    output_str(os, "#< ");
    output_str(os, name);
    output_str(os, ": #x");
    output_integer(obj.strip_tag() as i64, os, 16);
    output_str(os, " >");
}

/// Kept for parity with upstream helpers that take a Lisp string.
pub fn write_string(text: &str, os: LispObj) {
    write(string_node(text), os);
}
