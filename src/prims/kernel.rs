//! Kernel primitives used by the `load-sys.lisp` files (M2). Ports of the
//! corresponding `Lispfunc.cpp` functions; most are low level uvector,
//! character, stream and number operations that the Lisp code builds on.

use super::{check_args, funcall};
use crate::error::lisp_error;
use crate::heap::{self, cons, string_from_units};
use crate::numbers::{self, Num};
use crate::object::*;
use crate::printer::write_to_string;
use crate::qv::{lbool, nil, qv, t};
use crate::qv_index::*;
use crate::runtime::set_values;
use crate::streams;
use crate::symbols::{self, symbol_value};

fn err_obj(msg: &str, x: LispObj) -> ! {
    lisp_error(format!("{msg}: {}", write_to_string(x)))
}

pub fn check_fixnum(x: LispObj) -> i64 {
    if !x.is_fixnum() {
        err_obj("Not an integer", x);
    }
    x.integer()
}

pub fn check_index(x: LispObj) -> usize {
    let n = check_fixnum(x);
    if n < 0 {
        err_obj("Not a valid index", x);
    }
    n as usize
}

pub fn check_character(x: LispObj) -> u32 {
    if !x.is_character() {
        err_obj("Not a character", x);
    }
    x.char_code()
}

pub fn check_uvector(x: LispObj) {
    if !x.is_uvector() {
        err_obj("Not a uvector", x);
    }
}

pub fn check_string(x: LispObj) {
    if !x.is_string() {
        err_obj("Not a string", x);
    }
}

pub fn check_stream(x: LispObj) {
    if !x.is_stream() {
        err_obj("Not a stream", x);
    }
}

pub fn check_function(x: LispObj) {
    if !x.is_function() {
        err_obj("Not a function", x);
    }
}

fn check_symbol(x: LispObj) {
    if !x.is_symbol() {
        err_obj("Not a symbol", x);
    }
}

/// Contents of a string (simple or adjustable) as UTF-16 units.
pub fn string_units(s: LispObj) -> &'static [LispChar] {
    let len = s.vector_length().integer() as usize;
    let len = if s.is_adjustable_array() && has_fill_pointer(s) {
        s.uref(ARRAY_FILL_POINTER).integer() as usize
    } else {
        len
    };
    unsafe { std::slice::from_raw_parts(s.char_array_start(), len) }
}

pub fn has_fill_pointer(a: LispObj) -> bool {
    a.is_adjustable_array() && a.uref(ARRAY_FILL_POINTER) != nil()
}

/// The simple vector holding the data of `a`.
fn data_vector(a: LispObj) -> LispObj {
    if a.is_adjustable_array() { a.uref(ARRAY_VECTOR) } else { a }
}

fn byte_start(a: LispObj) -> *mut u8 {
    a.array_start() as *mut u8
}

// ---------------------------------------------------------------------------
// Characters

pub fn char_int(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    LispObj::fixnum(check_character(a[0]) as i64)
}

pub fn int_char(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    // PORT: upstream truncates to 8 bits; characters are UTF-16 units here.
    LispObj::character((check_fixnum(a[0]) as u32) & 0xffff)
}

fn map_char(c: u32, upper: bool) -> u32 {
    let Some(ch) = char::from_u32(c) else { return c };
    let mut it: Box<dyn Iterator<Item = char>> = if upper {
        Box::new(ch.to_uppercase())
    } else {
        Box::new(ch.to_lowercase())
    };
    match (it.next(), it.next()) {
        (Some(m), None) if (m as u32) < 0x10000 => m as u32,
        _ => c,
    }
}

pub fn char_upcase(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    LispObj::character(map_char(check_character(a[0]), true))
}

pub fn char_downcase(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    LispObj::character(map_char(check_character(a[0]), false))
}

// ---------------------------------------------------------------------------
// Uvectors

pub fn uref(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    check_uvector(a[0]);
    a[0].uref(check_index(a[1]))
}

pub fn uref_set(a: &[LispObj]) -> LispObj {
    check_args(a, 3, 3);
    check_uvector(a[1]);
    a[1].uset(check_index(a[2]), a[0]);
    a[0]
}

pub fn alloc_uvector(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    let size = check_index(a[0]);
    let tag = check_index(a[1]);
    heap::alloc_uvector(size, tag)
}

pub fn alloc_char_vector(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    heap::char_vector(check_index(a[0]))
}

pub fn uvectorp(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    lbool(a[0].is_uvector())
}

pub fn uvector_type_bits(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_uvector(a[0]);
    LispObj::fixnum(a[0].uvector_type() as i64)
}

pub fn uvector_num_slots(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_uvector(a[0]);
    LispObj::fixnum((a[0].uvector_size() * 2 - 1) as i64)
}

pub fn tag_bits(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    LispObj::fixnum(a[0].tag() as i64)
}

/// Address of a heap object, as a fixnum. PORT: upstream checks for a
/// uvector, but hash-table.lisp also applies it to conses (and the later
/// assembler version does not check), so any object is accepted.
pub fn uvector_address(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    LispObj::fixnum(a[0].strip_tag() as i64)
}

pub fn uninitialized_object_p(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    lbool(a[0] == UNINITIALIZED)
}

// ---------------------------------------------------------------------------
// Symbols and functions

pub fn make_symbol(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_string(a[0]);
    symbols::symbol_node(a[0])
}

pub fn fmakunbound(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_symbol(a[0]);
    a[0].uref(SYMBOL_FUNCTION).set_car(UNINITIALIZED);
    a[0].uset(SYMBOL_FUNCTION_TYPE, nil());
    a[0]
}

pub fn function_environment(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_function(a[0]);
    a[0].uref(FUNCTION_ENVIRONMENT)
}

pub fn function_info_list(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_function(a[0]);
    if a[0].is_kfunction() {
        return nil();
    }
    a[0].uref(FUNCTION_ADDRESS).uref(crate::runtime::CODE_INFO)
}

pub fn execution_address(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_function(a[0]);
    if a[0].is_kfunction() {
        return a[0].uref(FUNCTION_ADDRESS);
    }
    let code = a[0].uref(FUNCTION_ADDRESS);
    let off = code.uref(crate::runtime::CODE_ENTRY_OFFSET).integer() as usize;
    LispObj::fixnum((code.uvector_ptr() as usize + off) as i64)
}

// ---------------------------------------------------------------------------
// Type predicates

macro_rules! pred {
    ($name:ident, $test:expr) => {
        pub fn $name(a: &[LispObj]) -> LispObj {
            check_args(a, 1, 1);
            let f: fn(LispObj) -> bool = $test;
            lbool(f(a[0]))
        }
    };
}

pred!(structurep, |x| x.is_structure());
pred!(sequencep, |x| x.is_list() || x.is_vector());
pred!(hash_table_p, |x| x.is_uvector_of(HASHTABLE_TYPE));
pred!(readtablep, |x| x.is_readtable());
pred!(bignump, |x| x.is_uvector_of(BIGNUM_TYPE));
pred!(ratiop, |x| x.is_uvector_of(RATIO_TYPE));
pred!(complexp, |x| x.is_uvector_of(COMPLEX_TYPE));
pred!(double_float_p, |x| x.is_double_float());
pred!(single_float_p, |x| x.is_single_float());
pred!(short_float_p, |_x| false);
pred!(foreignp, |x| x.is_uvector_of(FOREIGN_TYPE));
pred!(foreign_heap_p, |x| x.is_uvector_of(FOREIGN_HEAP_TYPE));
pred!(numberp, numbers::is_number);

// ---------------------------------------------------------------------------
// Sequences and arrays

/// Upstream `ELT` for vectors: bounds check against the fill pointer.
fn vector_index(seq: LispObj, index: LispObj) -> usize {
    let n = check_fixnum(index);
    let limit = if has_fill_pointer(seq) {
        seq.uref(ARRAY_FILL_POINTER).integer()
    } else {
        seq.vector_length().integer()
    };
    if n < 0 || n >= limit {
        err_obj("Index out of range", index);
    }
    n as usize
}

fn list_cell(seq: LispObj, index: LispObj) -> LispObj {
    let n = check_fixnum(index);
    let mut s = seq;
    if n < 0 || !s.is_cons() {
        err_obj("Index out of range", index);
    }
    for _ in 0..n {
        s = s.cdr();
        if !s.is_cons() {
            err_obj("Index out of range", index);
        }
    }
    s
}

/// Element `i` of the data vector `v` (any simple vector type).
pub fn vector_ref(v: LispObj, i: usize) -> LispObj {
    match v.uvector_type() {
        SIMPLE_VECTOR_TYPE => v.svref(i),
        SIMPLE_CHAR_VECTOR_TYPE => LispObj::character(unsafe { *v.char_array_start().add(i) } as u32),
        SIMPLE_BYTE_VECTOR_TYPE => LispObj::fixnum(unsafe { *byte_start(v).add(i) } as i64),
        SIMPLE_BIT_VECTOR_TYPE => {
            let b = unsafe { *byte_start(v).add(i / 8) };
            LispObj::fixnum(((b >> (i % 8)) & 1) as i64)
        }
        SIMPLE_SHORT_VECTOR_TYPE => {
            LispObj::fixnum(unsafe { *(v.array_start() as *const i16).add(i) } as i64)
        }
        SIMPLE_DOUBLE_FLOAT_VECTOR_TYPE => {
            heap::double_float_node(unsafe { *(v.array_start() as *const f64).add(i) })
        }
        SIMPLE_SINGLE_FLOAT_VECTOR_TYPE => {
            heap::single_float_node(unsafe { *(v.array_start() as *const f32).add(i) })
        }
        _ => err_obj("Not a vector", v),
    }
}

/// Store `x` as element `i` of the data vector `v`.
pub fn vector_set(v: LispObj, i: usize, x: LispObj) {
    match v.uvector_type() {
        SIMPLE_VECTOR_TYPE => v.svset(i, x),
        SIMPLE_CHAR_VECTOR_TYPE => {
            let c = check_character(x);
            unsafe { *v.char_array_start().add(i) = c as LispChar };
        }
        SIMPLE_BYTE_VECTOR_TYPE => {
            let n = check_fixnum(x);
            if !(0..=255).contains(&n) {
                err_obj("Invalid value stored in byte array--out of range", x);
            }
            unsafe { *byte_start(v).add(i) = n as u8 };
        }
        SIMPLE_BIT_VECTOR_TYPE => {
            let n = check_fixnum(x);
            if !(0..=1).contains(&n) {
                err_obj("Invalid value stored in bit array--out of range", x);
            }
            unsafe {
                let p = byte_start(v).add(i / 8);
                *p = (*p & !(1 << (i % 8))) | ((n as u8) << (i % 8));
            }
        }
        SIMPLE_SHORT_VECTOR_TYPE => {
            let n = check_fixnum(x);
            unsafe { *(v.array_start() as *mut i16).add(i) = n as i16 };
        }
        SIMPLE_DOUBLE_FLOAT_VECTOR_TYPE => {
            let d = numbers::as_f64(numbers::num(x));
            unsafe { *(v.array_start() as *mut f64).add(i) = d };
        }
        SIMPLE_SINGLE_FLOAT_VECTOR_TYPE => {
            let d = numbers::as_f64(numbers::num(x));
            unsafe { *(v.array_start() as *mut f32).add(i) = d as f32 };
        }
        _ => err_obj("Not a vector", v),
    }
}

/// Element index of the start of an adjustable array's data (offset for
/// displaced arrays).
fn array_offset(a: LispObj) -> usize {
    if a.is_adjustable_array() {
        let o = a.uref(ARRAY_OFFSET);
        if o.is_fixnum() { o.integer() as usize } else { 0 }
    } else {
        0
    }
}

pub fn elt(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    let (seq, index) = (a[0], a[1]);
    if seq.is_vector() {
        let i = vector_index(seq, index);
        vector_ref(data_vector(seq), i + array_offset(seq))
    } else if seq.is_list() {
        list_cell(seq, index).car()
    } else {
        err_obj("Not a sequence", seq)
    }
}

pub fn setelt(a: &[LispObj]) -> LispObj {
    check_args(a, 3, 3);
    let (val, seq, index) = (a[0], a[1], a[2]);
    if seq.is_vector() {
        let i = vector_index(seq, index);
        vector_set(data_vector(seq), i + array_offset(seq), val);
    } else if seq.is_list() {
        list_cell(seq, index).set_car(val);
    } else {
        err_obj("Not a sequence", seq);
    }
    val
}

pub fn row_major_aref(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    if !a[0].is_array() {
        err_obj("Not an array", a[0]);
    }
    let i = check_index(a[1]) + array_offset(a[0]);
    let r = vector_ref(data_vector(a[0]), i);
    if r == UNINITIALIZED { nil() } else { r }
}

pub fn set_row_major_aref(a: &[LispObj]) -> LispObj {
    check_args(a, 3, 3);
    if !a[1].is_array() {
        err_obj("Not an array", a[1]);
    }
    let i = check_index(a[2]) + array_offset(a[1]);
    vector_set(data_vector(a[1]), i, a[0]);
    a[0]
}

pub fn array_type(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    if !a[0].is_array() {
        err_obj("Not an array", a[0]);
    }
    match data_vector(a[0]).uvector_type() {
        SIMPLE_VECTOR_TYPE => t(),
        SIMPLE_CHAR_VECTOR_TYPE => qv(CHARACTER_Index),
        SIMPLE_BYTE_VECTOR_TYPE => qv(LBYTE_Index),
        SIMPLE_BIT_VECTOR_TYPE => qv(BIT_Index),
        SIMPLE_SHORT_VECTOR_TYPE => qv(SHORT_SYM_Index),
        SIMPLE_DOUBLE_FLOAT_VECTOR_TYPE => qv(DOUBLE_FLOAT_Index),
        SIMPLE_SINGLE_FLOAT_VECTOR_TYPE => qv(SINGLE_FLOAT_Index),
        _ => nil(),
    }
}

pub fn array_rank(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    if !a[0].is_array() {
        err_obj("Not an array", a[0]);
    }
    LispObj::fixnum(a[0].array_dimensions() as i64)
}

/// Kernel COERCE: list of characters to string only.
pub fn coerce(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    let (seq, ty) = (a[0], a[1]);
    if seq.is_list() && ty == qv(STRING_Index) {
        let mut units = Vec::new();
        let mut p = seq;
        while p.is_cons() {
            if !p.car().is_character() {
                err_obj("Cannot coerce to string", seq);
            }
            units.push(p.car().char_code() as LispChar);
            p = p.cdr();
        }
        return string_from_units(&units);
    }
    err_obj("Cannot coerce", seq)
}

/// Upstream kernel `STRING=` (two strings, case sensitive).
pub fn string_equal(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    check_string(a[0]);
    check_string(a[1]);
    lbool(string_units(a[0]) == string_units(a[1]))
}

// ---------------------------------------------------------------------------
// Packages

pub fn package_hash_index(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    if !a[0].is_package() {
        err_obj("Not a package", a[0]);
    }
    check_string(a[1]);
    LispObj::fixnum(symbols::package_hash_index(a[0], a[1]) as i64)
}

// ---------------------------------------------------------------------------
// Streams

fn input_stream_arg(a: &[LispObj], i: usize) -> LispObj {
    match a.get(i) {
        None => symbol_value(qv(STANDARD_INPUT_Index)),
        Some(&s) if s == nil() || s == t() => symbol_value(qv(STANDARD_INPUT_Index)),
        Some(&s) => {
            check_stream(s);
            s
        }
    }
}

pub fn output_char_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    let c = check_character(a[0]);
    check_stream(a[1]);
    streams::output_char(a[1], c as LispChar);
    a[0]
}

pub fn output_chars(a: &[LispObj]) -> LispObj {
    check_args(a, 4, 4);
    let (s, stream) = (a[0], a[1]);
    check_string(s);
    check_stream(stream);
    let (start, end) = (check_fixnum(a[2]), check_fixnum(a[3]));
    let len = s.vector_length().integer();
    if start < 0 || end > len || end < start {
        lisp_error(format!("Index out of range: start = {start}, end = {end}"));
    }
    let units = unsafe { std::slice::from_raw_parts(s.char_array_start(), len as usize) };
    streams::output_units_to(stream, &units[start as usize..end as usize]);
    s
}

pub fn read_char_internal(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_stream(a[0]);
    let c = streams::get_character(a[0]);
    if c == qv(CEof_Index) { nil() } else { c }
}

pub fn read_char_with_error(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_stream(a[0]);
    let c = streams::get_character(a[0]);
    if c == qv(CEof_Index) {
        err_obj("Unexpected end of file in stream", a[0]);
    }
    c
}

pub fn read_char(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 4);
    let s = input_stream_arg(a, 0);
    let c = streams::get_character(s);
    if c == qv(CEof_Index) {
        if a.get(1).copied().unwrap_or(t()) != nil() {
            err_obj("End of file encountered in stream", s);
        }
        return a.get(2).copied().unwrap_or(nil());
    }
    c
}

pub fn unread_char(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 2);
    let s = input_stream_arg(a, 1);
    streams::putback_character(s, a[0]);
    nil()
}

pub fn close(a: &[LispObj]) -> LispObj {
    check_args(a, 1, usize::MAX);
    check_stream(a[0]);
    let s = a[0];
    if s.uref(STREAM_SUBCLASS) == qv(FILE_STREAM_Index) {
        let d = s.uref(STREAM_DIRECTION);
        if d == qv(OUTPUT_KEY_Index) || d == qv(BIDIRECTIONAL_KEY_Index) {
            streams::flush_stream(s);
        }
        streams::close_stream(s);
    }
    nil()
}

pub fn open_input_file(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_string(a[0]);
    streams::make_file_input_stream(&a[0].string_to_rust())
}

pub fn probe_file(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_string(a[0]);
    lbool(std::fs::File::open(a[0].string_to_rust()).is_ok())
}

// ---------------------------------------------------------------------------
// Numbers

/// Upstream `%CHARS-TO-FLOAT`: parse a list of characters as a float, or
/// return NIL.
pub fn chars_to_float(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    let mut s = String::new();
    let mut p = a[0];
    while p.is_cons() && s.len() < 128 {
        s.push(char::from_u32(check_character(p.car())).unwrap_or('?'));
        p = p.cdr();
    }
    let dflt = symbol_value(qv(READ_DEFAULT_FLOAT_FORMAT_Index));
    let mut double = dflt == qv(DOUBLE_FLOAT_Index) || dflt == qv(LONG_FLOAT_Index);
    let b = s.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let (mut digits, mut dots) = (0, 0);
    while i < b.len() {
        match b[i] {
            b'0'..=b'9' => digits += 1,
            b'.' => {
                dots += 1;
                if dots > 1 {
                    return nil();
                }
            }
            _ => break,
        }
        i += 1;
    }
    if digits == 0 {
        return nil();
    }
    let mut text = s[..i].to_string();
    if i < b.len() {
        match b[i].to_ascii_uppercase() {
            b'E' => {}
            b'F' | b'S' => double = false,
            b'D' | b'L' => double = true,
            _ => return nil(),
        }
        text.push('e');
        i += 1;
        let e0 = i;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let d0 = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == d0 || i != b.len() {
            return nil();
        }
        text.push_str(&s[e0..]);
    }
    let Ok(v) = text.parse::<f64>() else { return nil() };
    if double {
        heap::double_float_node(v)
    } else {
        heap::single_float_node(v as f32)
    }
}

pub fn float(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 2);
    let double = matches!(a.get(1), Some(p) if p.is_double_float());
    numbers::to_float(a[0], double)
}

fn logop(a: &[LispObj], init: i64, f: fn(i64, i64) -> i64) -> LispObj {
    let mut r = init;
    for &x in a {
        r = f(r, numbers::integer_value(x));
    }
    LispObj::fixnum(r)
}

pub fn logand(a: &[LispObj]) -> LispObj {
    logop(a, -1, |x, y| x & y)
}
pub fn logior(a: &[LispObj]) -> LispObj {
    logop(a, 0, |x, y| x | y)
}
pub fn logxor(a: &[LispObj]) -> LispObj {
    logop(a, 0, |x, y| x ^ y)
}
pub fn logeqv(a: &[LispObj]) -> LispObj {
    logop(a, -1, |x, y| !(x ^ y))
}
pub fn lognot(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    LispObj::fixnum(!numbers::integer_value(a[0]))
}

fn logop2(a: &[LispObj], f: fn(i64, i64) -> i64) -> LispObj {
    check_args(a, 2, 2);
    LispObj::fixnum(f(numbers::integer_value(a[0]), numbers::integer_value(a[1])))
}
pub fn logandc1(a: &[LispObj]) -> LispObj {
    logop2(a, |x, y| !x & y)
}
pub fn logandc2(a: &[LispObj]) -> LispObj {
    logop2(a, |x, y| x & !y)
}
pub fn lognand(a: &[LispObj]) -> LispObj {
    logop2(a, |x, y| !(x & y))
}
pub fn lognor(a: &[LispObj]) -> LispObj {
    logop2(a, |x, y| !(x | y))
}
pub fn logorc1(a: &[LispObj]) -> LispObj {
    logop2(a, |x, y| !x | y)
}
pub fn logorc2(a: &[LispObj]) -> LispObj {
    logop2(a, |x, y| x | !y)
}

/// (%ASH n count): arithmetic shift on fixnums.
pub fn ash(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    let n = numbers::integer_value(a[0]);
    let c = numbers::integer_value(a[1]);
    if c <= 0 {
        LispObj::fixnum(n >> (-c).min(63))
    } else {
        let r = (n as i128) << c.min(100);
        numbers::int_result(i64::try_from(r).ok())
    }
}

pub fn sqrt(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    if numbers::as_f64(numbers::num(a[0])) < 0.0 {
        lisp_error("SQRT of a negative number: complex numbers are not supported yet");
    }
    numbers::float_fn(a[0], f64::sqrt)
}

pub fn isqrt(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    let n = numbers::integer_value(a[0]);
    if n < 0 {
        err_obj("ISQRT: not a non-negative integer", a[0]);
    }
    let mut r = (n as f64).sqrt() as i64;
    while r * r > n {
        r -= 1;
    }
    while (r + 1) * (r + 1) <= n {
        r += 1;
    }
    LispObj::fixnum(r)
}

pub fn exp(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    numbers::float_fn(a[0], f64::exp)
}

pub fn sin(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    numbers::float_fn(a[0], f64::sin)
}

pub fn cos(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    numbers::float_fn(a[0], f64::cos)
}

pub fn log(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 2);
    let x = numbers::num(a[0]);
    if numbers::as_f64(x) <= 0.0 {
        lisp_error("LOG of a non-positive number is not supported yet");
    }
    match a.get(1) {
        None => numbers::float_fn(a[0], f64::ln),
        Some(&b) => {
            let l = numbers::float_fn(a[0], f64::ln);
            let lb = numbers::float_fn(b, f64::ln);
            numbers::div(l, lb)
        }
    }
}

pub fn expt(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    match (numbers::num(a[0]), numbers::num(a[1])) {
        (Num::Int(b), Num::Int(e)) if e >= 0 => {
            let r = u32::try_from(e).ok().and_then(|e| b.checked_pow(e));
            numbers::int_result(r)
        }
        (Num::Int(_), Num::Int(_)) => {
            let p = expt(&[a[0], LispObj::fixnum(-a[1].integer())]);
            numbers::div(LispObj::fixnum(1), p)
        }
        (b, Num::Int(e)) => {
            let v = numbers::as_f64(b).powi(e.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
            if matches!(b, Num::Double(_)) {
                heap::double_float_node(v)
            } else {
                heap::single_float_node(v as f32)
            }
        }
        (b, e) => {
            let v = numbers::as_f64(b).powf(numbers::as_f64(e));
            if matches!(b, Num::Double(_)) || matches!(e, Num::Double(_)) {
                heap::double_float_node(v)
            } else {
                heap::single_float_node(v as f32)
            }
        }
    }
}

pub fn complex(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 2);
    match a.get(1) {
        None => a[0],
        Some(&i) if i == LispObj::fixnum(0) => a[0],
        _ => lisp_error("Complex numbers are not supported yet"),
    }
}

/// PRINT-FLOAT: upstream ignores the format arguments and calls WRITE.
pub fn print_float(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 8);
    let kw = symbols::find_keyword("STREAM");
    funcall(qv(WRITE_Index), &[a[0], kw, a[1]]);
    set_values(&[a[0]])
}

// ---------------------------------------------------------------------------
// Hashing (upstream `Hash_eq_function` etc.). Results are non-negative
// fixnums.

fn hash_ulong(n: u64) -> u64 {
    n ^ (n << 5) ^ (n << 10) ^ (n << 15) ^ (n << 20) ^ (n << 25)
}

fn hash_result(n: u64) -> LispObj {
    LispObj((n & 0x0fff_ffff_ffff_fff8) as usize)
}

fn hash_eq(x: LispObj) -> u64 {
    hash_ulong(x.raw() as u64)
}

fn hash_eql(x: LispObj) -> u64 {
    if x.is_double_float() {
        hash_ulong(x.double_float().to_bits())
    } else if x.is_single_float() {
        hash_ulong(x.single_float().to_bits() as u64)
    } else {
        hash_eq(x)
    }
}

fn hash_equal(x: LispObj, depth: usize) -> u64 {
    if x.is_cons() {
        if depth > 8 {
            return 17;
        }
        hash_equal(x.car(), depth + 1).rotate_left(7) ^ hash_equal(x.cdr(), depth + 1)
    } else if x.is_string() {
        // PORT: hash the characters (upstream hashes the raw uvector,
        // which differs between equal simple and adjustable strings).
        let mut h: u64 = 0;
        for &u in string_units(x) {
            h = h.wrapping_mul(31).wrapping_add(u as u64);
        }
        hash_ulong(h)
    } else {
        hash_eql(x)
    }
}

pub fn hash_eq_function(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    hash_result(hash_eq(a[0]))
}

pub fn hash_eql_function(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    hash_result(hash_eql(a[0]))
}

pub fn hash_equal_function(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    hash_result(hash_equal(a[0], 0))
}

// ---------------------------------------------------------------------------
// Misc (no-ops or constant results in this kernel)

pub fn get_gc_id(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 0);
    LispObj::fixnum(crate::gc::gc_count() as i64)
}

/// (GC &optional level): collect garbage now. There are no generations,
/// so the level is ignored.
pub fn gc(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 1);
    crate::gc::collect();
    nil()
}

pub fn return_nil(_a: &[LispObj]) -> LispObj {
    nil()
}

pub fn return_t(_a: &[LispObj]) -> LispObj {
    t()
}

pub fn identity_first(a: &[LispObj]) -> LispObj {
    check_args(a, 1, usize::MAX);
    a[0]
}

pub fn stack_trace(_a: &[LispObj]) -> LispObj {
    nil()
}

pub fn lisp_shutdown(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 1);
    let code = a.first().filter(|x| x.is_fixnum()).map_or(0, |x| x.integer());
    for idx in [STANDARD_OUTPUT_Index, ERROR_OUTPUT_Index] {
        let s = symbol_value(qv(idx));
        if s.is_stream() {
            streams::flush_stream(s);
        }
    }
    std::process::exit(code as i32)
}

pub fn get_universal_time(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 0);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Universal time counts from 1900-01-01.
    LispObj::fixnum(secs as i64 + 2_208_988_800)
}

pub fn alloc_cons_cell(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 0);
    cons(nil(), nil())
}

// ---------------------------------------------------------------------------
// Array initialization (redefined in arrays.lisp)

fn array_dim(a: LispObj, i: usize) -> usize {
    if a.is_adjustable_array() {
        a.uref(ARRAY_DIM1_LENGTH + i).integer() as usize
    } else {
        a.vector_length().integer() as usize
    }
}

fn array_total_size(a: LispObj) -> usize {
    (0..a.array_dimensions()).map(|i| array_dim(a, i)).product()
}

fn init_contents(a: LispObj, contents: LispObj, axis: usize, index: &mut usize) {
    let rank = a.array_dimensions();
    let items: Vec<LispObj> = if contents.is_list() {
        let mut v = Vec::new();
        let mut p = contents;
        while p.is_cons() {
            v.push(p.car());
            p = p.cdr();
        }
        v
    } else if contents.is_vector() {
        let n = contents.vector_length().integer() as usize;
        (0..n).map(|i| vector_ref(data_vector(contents), i + array_offset(contents))).collect()
    } else {
        err_obj("Invalid initializer form", contents)
    };
    if items.len() != array_dim(a, axis) {
        err_obj("Invalid initializer form, incorrect number of items in dimension", contents);
    }
    for x in items {
        if axis + 1 == rank {
            vector_set(data_vector(a), *index + array_offset(a), x);
            *index += 1;
        } else {
            init_contents(a, x, axis + 1, index);
        }
    }
}

pub fn array_initialize_contents(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    if !a[0].is_array() {
        err_obj("Not an array", a[0]);
    }
    if a[0].array_dimensions() == 0 {
        vector_set(data_vector(a[0]), array_offset(a[0]), a[1]);
    } else {
        init_contents(a[0], a[1], 0, &mut 0);
    }
    a[0]
}

pub fn array_initialize_element(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    if !a[0].is_array() {
        err_obj("Not an array", a[0]);
    }
    let v = data_vector(a[0]);
    let off = array_offset(a[0]);
    for i in 0..array_total_size(a[0]) {
        vector_set(v, i + off, a[1]);
    }
    a[0]
}

pub fn vector_slot_initialized(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    let v = a[0];
    if !v.is_vector() || data_vector(v).uvector_type() != SIMPLE_VECTOR_TYPE {
        err_obj("Invalid vector type passed to VECTOR-SLOT-INITIALIZED", v);
    }
    let i = vector_index(v, a[1]);
    lbool(data_vector(v).svref(i + array_offset(v)) != UNINITIALIZED)
}

/// Port of the `WHICH-HEAP` defasm in hash-table.lisp: the heap
/// generation (0-2) holding `obj`, or NIL for immediates. There is a
/// single heap without generations, reported as generation 0 (whose GC id
/// changes on every collection, so EQ hash tables rehash).
pub fn which_heap(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    let x = a[0];
    if (x.is_cons() || x.is_uvector()) && heap::in_allocated_block(x.raw()) {
        LispObj::fixnum(0)
    } else {
        nil()
    }
}

/// Port of the `HASH-OBJ-ID` defasm in hash-table.lisp (64-bit rotates):
/// a non-negative fixnum derived from the raw bits of `x`.
fn hash_obj_id_raw(x: u64) -> LispObj {
    let mut e = x;
    let mut d = x;
    for _ in 0..3 {
        e = e.rotate_left(9);
        d ^= e;
    }
    LispObj((d & 0x7fff_ffff_ffff_fff8) as usize)
}

pub fn hash_obj_id(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    hash_obj_id_raw(a[0].raw() as u64)
}

/// Port of the `HASH-CELL` defasm: hash of the raw contents of cell
/// `index` of a uvector (no type checking upstream).
pub fn hash_cell(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    check_uvector(a[0]);
    hash_obj_id_raw(a[0].uref(check_index(a[1])).raw() as u64)
}

/// Port of the `FIXNUM-MOD` defasm (truncating remainder).
pub fn fixnum_mod(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    let (x, y) = (check_fixnum(a[0]), check_fixnum(a[1]));
    if y == 0 {
        lisp_error("Division by zero");
    }
    LispObj::fixnum(x % y)
}

/// REGISTRATION-INFO: values registered, version, name, organization,
/// days remaining.
pub fn registration_info(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 0);
    let name = heap::string_node("Unknown");
    set_values(&[t(), LispObj::fixnum(3), name, nil(), LispObj::fixnum(0)])
}

pub fn heap_currently_used(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 0);
    LispObj::fixnum(heap::bytes_used() as i64)
}

// ---------------------------------------------------------------------------
// M4: replacements for kernel-asm.lisp DEFASMs used by load-sys2 files

fn simple_vector_index(v: LispObj, index: LispObj) -> usize {
    check_uvector(v);
    let i = check_index(index);
    if i >= v.vector_length().integer() as usize {
        err_obj("Index out of range", index);
    }
    i
}

/// `X86::%SVREF` and friends: element of a simple vector of any type
/// (upstream has one DEFASM per element type).
pub fn sv_ref(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    let i = simple_vector_index(a[0], a[1]);
    vector_ref(a[0], i)
}

/// `(SETF X86::%SVREF)` and friends.
pub fn sv_set(a: &[LispObj]) -> LispObj {
    check_args(a, 3, 3);
    let i = simple_vector_index(a[1], a[2]);
    vector_set(a[1], i, a[0]);
    a[0]
}

pub fn svref(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    if !a[0].is_uvector() || a[0].uvector_type() != SIMPLE_VECTOR_TYPE {
        err_obj("Not a simple vector", a[0]);
    }
    sv_ref(a)
}

pub fn set_svref(a: &[LispObj]) -> LispObj {
    check_args(a, 3, 3);
    if !a[1].is_uvector() || a[1].uvector_type() != SIMPLE_VECTOR_TYPE {
        err_obj("Not a simple vector", a[1]);
    }
    sv_set(a)
}

pub fn set(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    check_symbol(a[0]);
    if crate::symbols::is_constant_symbol(a[0]) {
        err_obj("Cannot assign to a constant", a[0]);
    }
    crate::symbols::set_symbol_value(a[0], a[1]);
    a[1]
}

pub fn boundp(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_symbol(a[0]);
    lbool(crate::symbols::is_bound(a[0]))
}

pub fn makunbound(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_symbol(a[0]);
    if crate::symbols::is_constant_symbol(a[0]) {
        err_obj("Cannot make a constant unbound", a[0]);
    }
    crate::symbols::set_symbol_value(a[0], SYMBOL_UNBOUND);
    a[0]
}

// Float bit access for math.lisp. Upstream returns the 64 bits of a
// double as one integer (a bignum); without bignums the halves are used.

pub fn make_single_float(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    heap::single_float_node(f32::from_bits(check_fixnum(a[0]) as u32))
}

pub fn single_float_bits(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    match numbers::num(a[0]) {
        Num::Single(f) => LispObj::fixnum(f.to_bits() as i64),
        _ => err_obj("Not a single float", a[0]),
    }
}

/// `(%MAKE-DOUBLE-FLOAT high low)`: high and low 32 bits.
pub fn make_double_float(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    let hi = check_fixnum(a[0]) as u64 & 0xffff_ffff;
    let lo = check_fixnum(a[1]) as u64 & 0xffff_ffff;
    heap::double_float_node(f64::from_bits(hi << 32 | lo))
}

fn double_bits(x: LispObj) -> u64 {
    match numbers::num(x) {
        Num::Double(d) => d.to_bits(),
        _ => err_obj("Not a double float", x),
    }
}

pub fn double_float_high_bits(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    LispObj::fixnum((double_bits(a[0]) >> 32) as i64)
}

pub fn double_float_low_bits(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    LispObj::fixnum((double_bits(a[0]) & 0xffff_ffff) as i64)
}

/// Real ASIN (upstream uses complex arithmetic, not supported yet).
pub fn asin(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    let x = numbers::as_f64(numbers::num(a[0]));
    if !(-1.0..=1.0).contains(&x) {
        err_obj("ASIN of a number outside [-1, 1] is not supported yet", a[0]);
    }
    numbers::float_fn(a[0], f64::asin)
}

/// Real ATAN of y, or of y/x with the quadrant from both signs.
pub fn atan(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 2);
    match a.get(1) {
        None => numbers::float_fn(a[0], f64::atan),
        Some(&x) => {
            let (ny, nx) = (numbers::num(a[0]), numbers::num(x));
            let v = numbers::as_f64(ny).atan2(numbers::as_f64(nx));
            if matches!(ny, Num::Double(_)) || matches!(nx, Num::Double(_)) {
                heap::double_float_node(v)
            } else {
                heap::single_float_node(v as f32)
            }
        }
    }
}
