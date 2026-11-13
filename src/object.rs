//! Lisp object representation: a 64-bit widening of upstream `include/Lisp.h`.
//!
//! Low 3 bits are the tag. Fixnums are `n << 3` (61-bit). Characters are
//! `code << 8 | ImmediateTag`. Conses are 16 bytes; a cons pointer is the
//! cell address + 4, CAR at `addr`, CDR at `addr + 8`. Uvectors are 16-byte
//! aligned; a uvector pointer is the address + 5 and cell 0 holds the header
//! `(length_in_16_byte_units << 8) | (type << 3) | UvectorLengthTag`.
#![allow(dead_code)]

use std::fmt;

/// A tagged Lisp value. One machine word.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LispObj(pub usize);

impl fmt::Debug for LispObj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LispObj({:#x})", self.0)
    }
}

pub const CELL: usize = 8;

// Tags
pub const FIXNUM_TAG: usize = 0;
pub const IMMEDIATE_TAG: usize = 1;
pub const FORWARD_TAG: usize = 2;
pub const SHORT_FLOAT_TAG: usize = 3; // reserved, unused
pub const CONS_TAG: usize = 4;
pub const UVECTOR_TAG: usize = 5;
pub const UVECTOR_LENGTH_TAG: usize = 6;
pub const SHORT_FLOAT_TAG2: usize = 7; // reserved, unused

// Immediate types
pub const CHARACTER_TYPE: usize = 0;

// Uvector types (identical numbering to upstream)
pub const FUNCTION_TYPE: usize = 0;
pub const KFUNCTION_TYPE: usize = 1;
pub const STRUCTURE_TYPE: usize = 2;
pub const ARRAY_TYPE: usize = 3;
pub const SYMBOL_TYPE: usize = 4;
pub const STREAM_TYPE: usize = 5;
pub const DOUBLE_FLOAT_TYPE: usize = 6;
pub const PACKAGE_TYPE: usize = 7;
pub const HASHTABLE_TYPE: usize = 8;
pub const FOREIGN_TYPE: usize = 9;
pub const COMPILED_CODE_TYPE: usize = 10;
pub const READ_TABLE_TYPE: usize = 11;
pub const COMPLEX_TYPE: usize = 12;
pub const RATIO_TYPE: usize = 13;
pub const BIGNUM_TYPE: usize = 14;
pub const FOREIGN_HEAP_TYPE: usize = 15;
pub const WEAK_POINTER_TYPE: usize = 16;
pub const SIMPLE_VECTOR_TYPE: usize = 17;
pub const SIMPLE_CHAR_VECTOR_TYPE: usize = 18;
pub const SIMPLE_BYTE_VECTOR_TYPE: usize = 19;
pub const SIMPLE_SHORT_VECTOR_TYPE: usize = 20;
pub const SIMPLE_DOUBLE_FLOAT_VECTOR_TYPE: usize = 21;
pub const SIMPLE_BIT_VECTOR_TYPE: usize = 22;
pub const SIMPLE_SINGLE_FLOAT_VECTOR_TYPE: usize = 23;
pub const SINGLE_FLOAT_TYPE: usize = 24;
pub const CLOS_INSTANCE_TYPE: usize = 25;
pub const FOREIGN_STACK_TYPE: usize = 26;
pub const FOREIGN_STACK_END_TYPE: usize = 27;

// Function / KFunction
pub const FUNCTION_ENVIRONMENT: usize = 1;
pub const FUNCTION_ADDRESS: usize = 2;
pub const FUNCTION_SIZE: usize = 3;
pub const KFUNCTION_SIZE: usize = 3;

// CLOS instance
pub const CLOS_INSTANCE_CLASS: usize = 1;
pub const CLOS_INSTANCE_SLOTS: usize = 2;
/// Slot index of DISCRIMINATING-FUNCTION in a standard generic function
/// (clos.lisp `slot-location-generic-function-discriminating-function`).
pub const SLOT_LOCATION_GF_DISCRIMINATING_FUNCTION: usize = 6;

// Structure
pub const STRUCT_TEMPLATE: usize = 1;

// Arrays
pub const ARRAY_VECTOR: usize = 1;
pub const ARRAY_FILL_POINTER: usize = 2;
pub const ARRAY_OFFSET: usize = 3;
pub const ARRAY_DIMENSIONS: usize = 4;
pub const ARRAY_DIM1_LENGTH: usize = 5;
pub const ARRAY_SIMPLE_VECTOR_LENGTH: usize = 1;
pub const ARRAY_SIMPLE_VECTOR_START: usize = 2;

// Symbol
pub const SYMBOL_NAME: usize = 1;
pub const SYMBOL_VALUE: usize = 2;
pub const SYMBOL_PACKAGE: usize = 3;
pub const SYMBOL_PROPERTY_LIST: usize = 4;
pub const SYMBOL_CONSTANT: usize = 5;
pub const SYMBOL_FLAGS: usize = 5;
pub const SYMBOL_FUNCTION_TYPE: usize = 6;
pub const SYMBOL_FUNCTION: usize = 7;
pub const SYMBOL_JUMP_TABLE: usize = 8;
pub const SYMBOL_VAR_TABLE: usize = 9;
pub const SYMBOL_SIZE: usize = 9;
pub const SYMBOL_CONSTANT_FLAG: usize = 1 << 3;
pub const SYMBOL_SPECIAL_FLAG: usize = 1 << 4;

/// Unbound marker / uninitialized / end of `list()` args: raw value 4.
pub const SYMBOL_UNBOUND: LispObj = LispObj(CONS_TAG);
pub const UNINITIALIZED: LispObj = LispObj(CONS_TAG);
pub const END_LIST: LispObj = LispObj(CONS_TAG);
pub const UNWIND_MARKER: LispObj = LispObj(CONS_TAG + 8);

// Stream
pub const STREAM_NAME: usize = 1;
pub const STREAM_UNDERFLOW_FUNC: usize = 2;
pub const STREAM_POSITION: usize = 3;
pub const STREAM_COL_POSITION: usize = 4;
pub const STREAM_INPUT_BUFFER: usize = 5;
pub const STREAM_INPUT_BUFFER_LENGTH: usize = 6;
pub const STREAM_INPUT_BUFFER_POS: usize = 7;
pub const STREAM_INPUT_BUFFER_NUM: usize = 8;
pub const STREAM_HANDLE: usize = 9;
pub const STREAM_SUBCLASS: usize = 10;
pub const STREAM_BINARY: usize = 11;
pub const STREAM_OPEN: usize = 12;
pub const STREAM_DIRECTION: usize = 13;
pub const STREAM_INTERACTIVE: usize = 14;
pub const STREAM_ELEMENT_TYPE: usize = 15;
pub const STREAM_ASSOCIATED_STREAMS: usize = 16;
pub const STREAM_OVERFLOW_FUNC: usize = 17;
pub const STREAM_OUTPUT_BUFFER: usize = 18;
pub const STREAM_OUTPUT_BUFFER_LENGTH: usize = 19;
pub const STREAM_OUTPUT_BUFFER_POS: usize = 20;
pub const STREAM_LINE_NUMBER: usize = 21;
pub const STREAM_SIZE: usize = 21;

// Double float. PORT: offset kept at 2 (cell 1 unused) to match upstream.
pub const DOUBLE_FLOAT_OFFSET: usize = 2;
pub const DOUBLE_FLOAT_SIZE: usize = 3;
pub const SINGLE_FLOAT_OFFSET: usize = 1;

// Package
pub const PACKAGE_NAME: usize = 1;
pub const PACKAGE_NICKNAMES: usize = 2;
pub const PACKAGE_USE_LIST: usize = 3;
pub const PACKAGE_USED_BY_LIST: usize = 4;
pub const PACKAGE_SHADOWING_SYMBOLS: usize = 5;
pub const PACKAGE_CAPACITY: usize = 6;
pub const PACKAGE_COUNT: usize = 7;
pub const PACKAGE_TABLE: usize = 8;
pub const PACKAGE_SYNC: usize = 9;
pub const PACKAGE_SIZE: usize = 9;
pub const PACKAGE_RESIZE_THRESHOLD: f64 = 0.8;

// Readtable
pub const READTABLE_READ_LEVEL: usize = 1;
pub const READTABLE_BACKQUOTE_PROCESSING: usize = 2;
pub const READTABLE_TABLE: usize = 3;
pub const READTABLE_CASE: usize = 4;
pub const READTABLE_SIZE: usize = 5;

/// Element type of character vectors (UTF-16 code unit, as upstream).
pub type LispChar = u16;

pub const FIXNUM_BITS: u32 = 61;
pub const MOST_POSITIVE_FIXNUM: i64 = (1i64 << (FIXNUM_BITS - 1)) - 1;
pub const MOST_NEGATIVE_FIXNUM: i64 = -MOST_POSITIVE_FIXNUM - 1;

impl LispObj {
    #[inline]
    pub const fn raw(self) -> usize {
        self.0
    }
    #[inline]
    pub const fn tag(self) -> usize {
        self.0 & 7
    }
    #[inline]
    pub const fn strip_tag(self) -> usize {
        self.0 & !7
    }

    // ---- fixnums ----
    #[inline]
    pub const fn fixnum(n: i64) -> LispObj {
        LispObj((n << 3) as usize)
    }
    #[inline]
    pub fn fixnum_checked(n: i64) -> Option<LispObj> {
        (MOST_NEGATIVE_FIXNUM..=MOST_POSITIVE_FIXNUM)
            .contains(&n)
            .then(|| LispObj::fixnum(n))
    }
    #[inline]
    pub const fn is_fixnum(self) -> bool {
        self.0 & 7 == FIXNUM_TAG
    }
    /// Value of a fixnum (arithmetic shift).
    #[inline]
    pub const fn integer(self) -> i64 {
        (self.0 as i64) >> 3
    }

    // ---- characters / immediates ----
    #[inline]
    pub const fn character(code: u32) -> LispObj {
        LispObj(((code as usize) << 8) | IMMEDIATE_TAG)
    }
    #[inline]
    pub const fn is_immediate(self) -> bool {
        self.tag() == IMMEDIATE_TAG
    }
    #[inline]
    pub const fn immediate_type(self) -> usize {
        (self.0 >> 3) & 0x1f
    }
    #[inline]
    pub const fn is_character(self) -> bool {
        self.is_immediate() && self.immediate_type() == CHARACTER_TYPE
    }
    #[inline]
    pub const fn char_code(self) -> u32 {
        (self.0 >> 8) as u32
    }

    // ---- heap objects ----
    #[inline]
    pub const fn is_cons(self) -> bool {
        self.tag() == CONS_TAG
    }
    #[inline]
    pub const fn is_uvector(self) -> bool {
        self.tag() == UVECTOR_TAG
    }
    #[inline]
    pub const fn is_uvector_header(self) -> bool {
        self.tag() == UVECTOR_LENGTH_TAG
    }
    #[inline]
    pub const fn is_forward(self) -> bool {
        self.tag() == FORWARD_TAG
    }

    #[inline]
    pub fn car(self) -> LispObj {
        debug_assert!(self.is_cons());
        unsafe { *((self.0 - CONS_TAG) as *const LispObj) }
    }
    #[inline]
    pub fn cdr(self) -> LispObj {
        debug_assert!(self.is_cons());
        unsafe { *((self.0 - CONS_TAG + CELL) as *const LispObj) }
    }
    #[inline]
    pub fn set_car(self, v: LispObj) {
        debug_assert!(self.is_cons());
        unsafe { *((self.0 - CONS_TAG) as *mut LispObj) = v }
    }
    #[inline]
    pub fn set_cdr(self, v: LispObj) {
        debug_assert!(self.is_cons());
        unsafe { *((self.0 - CONS_TAG + CELL) as *mut LispObj) = v }
    }

    /// Pointer to cell 0 (the header) of a uvector. Upstream `UVECTOR(n)`.
    #[inline]
    pub fn uvector_ptr(self) -> *mut LispObj {
        debug_assert!(self.is_uvector());
        (self.0 - UVECTOR_TAG) as *mut LispObj
    }
    /// Read cell `i` of a uvector (cell 0 = header).
    #[inline]
    pub fn uref(self, i: usize) -> LispObj {
        unsafe { *self.uvector_ptr().add(i) }
    }
    #[inline]
    pub fn uset(self, i: usize, v: LispObj) {
        unsafe { *self.uvector_ptr().add(i) = v }
    }
    #[inline]
    pub fn uvector_type(self) -> usize {
        (self.uref(0).0 >> 3) & 0x1f
    }
    /// Length in 16-byte units (upstream `uvectorSize`).
    #[inline]
    pub fn uvector_size(self) -> usize {
        self.uref(0).0 >> 8
    }
    #[inline]
    pub fn set_uvector_type(self, ty: usize) {
        let h = self.uref(0).0;
        self.uset(0, LispObj((h & !0xf8) | (ty << 3)));
    }
    #[inline]
    pub fn is_uvector_of(self, ty: usize) -> bool {
        self.is_uvector() && self.uvector_type() == ty
    }

    pub fn is_symbol(self) -> bool {
        self.is_uvector_of(SYMBOL_TYPE)
    }
    pub fn is_package(self) -> bool {
        self.is_uvector_of(PACKAGE_TYPE)
    }
    pub fn is_stream(self) -> bool {
        self.is_uvector_of(STREAM_TYPE)
    }
    pub fn is_readtable(self) -> bool {
        self.is_uvector_of(READ_TABLE_TYPE)
    }
    pub fn is_double_float(self) -> bool {
        self.is_uvector_of(DOUBLE_FLOAT_TYPE)
    }
    pub fn is_function(self) -> bool {
        self.is_uvector() && self.uvector_type() <= KFUNCTION_TYPE
    }
    pub fn is_kfunction(self) -> bool {
        self.is_uvector_of(KFUNCTION_TYPE)
    }
    pub fn is_adjustable_array(self) -> bool {
        self.is_uvector_of(ARRAY_TYPE)
    }
    pub fn is_simple_vector(self) -> bool {
        self.is_uvector() && {
            let t = self.uvector_type();
            (SIMPLE_VECTOR_TYPE..=SIMPLE_SINGLE_FLOAT_VECTOR_TYPE).contains(&t)
        }
    }
    pub fn is_simple_char_vector(self) -> bool {
        self.is_uvector_of(SIMPLE_CHAR_VECTOR_TYPE)
    }
    pub fn is_simple_generic_vector(self) -> bool {
        self.is_uvector_of(SIMPLE_VECTOR_TYPE)
    }
    pub fn is_array(self) -> bool {
        self.is_simple_vector() || self.is_adjustable_array()
    }
    pub fn array_dimensions(self) -> usize {
        if self.is_adjustable_array() {
            self.uref(ARRAY_DIMENSIONS).integer() as usize
        } else {
            1
        }
    }
    pub fn is_vector(self) -> bool {
        self.is_array() && self.array_dimensions() == 1
    }
    pub fn is_string(self) -> bool {
        self.is_vector()
            && (self.is_simple_char_vector()
                || (self.is_adjustable_array()
                    && self.uref(ARRAY_VECTOR).is_simple_char_vector()))
    }

    /// Wrapped (fixnum) length of a vector.
    pub fn vector_length(self) -> LispObj {
        if self.is_adjustable_array() {
            self.uref(ARRAY_DIM1_LENGTH)
        } else {
            self.uref(ARRAY_SIMPLE_VECTOR_LENGTH)
        }
    }
    /// Pointer to the first data cell of a simple vector, or of an
    /// adjustable array's backing vector.
    pub fn array_start(self) -> *mut LispObj {
        let v = if self.is_adjustable_array() {
            self.uref(ARRAY_VECTOR)
        } else {
            self
        };
        unsafe { v.uvector_ptr().add(ARRAY_SIMPLE_VECTOR_START) }
    }
    pub fn char_array_start(self) -> *mut LispChar {
        self.array_start() as *mut LispChar
    }
    /// Element `i` of a simple general vector.
    pub fn svref(self, i: usize) -> LispObj {
        unsafe { *self.array_start().add(i) }
    }
    pub fn svset(self, i: usize, v: LispObj) {
        unsafe { *self.array_start().add(i) = v }
    }

    pub fn is_single_float(self) -> bool {
        self.is_uvector_of(SINGLE_FLOAT_TYPE)
    }
    pub fn is_float(self) -> bool {
        self.is_double_float() || self.is_single_float()
    }
    pub fn single_float(self) -> f32 {
        unsafe { *(self.uvector_ptr().add(SINGLE_FLOAT_OFFSET) as *const f32) }
    }
    pub fn is_structure(self) -> bool {
        self.is_uvector_of(STRUCTURE_TYPE)
    }
    pub fn is_list(self) -> bool {
        self.is_cons() || self == crate::qv::nil()
    }
    pub fn double_float(self) -> f64 {
        unsafe { *(self.uvector_ptr().add(DOUBLE_FLOAT_OFFSET) as *const f64) }
    }

    /// Contents of a Lisp string as a Rust `String`.
    pub fn string_to_rust(self) -> String {
        debug_assert!(self.is_string());
        let len = self.vector_length().integer() as usize;
        let s = unsafe { std::slice::from_raw_parts(self.char_array_start(), len) };
        String::from_utf16_lossy(s)
    }
}

/// Uvector header word for `units` 16-byte units of type `ty`.
#[inline]
pub const fn uvector_header(units: usize, ty: usize) -> LispObj {
    LispObj((units << 8) | (ty << 3) | UVECTOR_LENGTH_TAG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixnum_roundtrip() {
        for n in [0i64, 1, -1, 42, -42, MOST_POSITIVE_FIXNUM, MOST_NEGATIVE_FIXNUM] {
            let o = LispObj::fixnum(n);
            assert!(o.is_fixnum());
            assert_eq!(o.integer(), n);
        }
        assert!(LispObj::fixnum_checked(MOST_POSITIVE_FIXNUM + 1).is_none());
        assert_eq!(LispObj::fixnum(5).raw(), 40);
    }

    #[test]
    fn character_tagging() {
        let c = LispObj::character('A' as u32);
        assert!(c.is_character());
        assert!(!c.is_fixnum());
        assert_eq!(c.char_code(), 65);
        assert_eq!(c.raw(), (65 << 8) | 1);
    }

    #[test]
    fn markers() {
        assert_eq!(UNINITIALIZED.raw(), 4);
        assert_eq!(UNWIND_MARKER.raw(), 12);
        assert_eq!(uvector_header(2, SYMBOL_TYPE).raw(), (2 << 8) | (4 << 3) | 6);
    }
}
