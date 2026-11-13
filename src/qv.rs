//! The QV globals vector. Upstream keeps it per thread and passes it in ESI;
//! the port keeps it at the start of the heap region and will pass it in R15.
//! Only one Lisp thread is supported for now.
#![allow(dead_code)]

use crate::object::LispObj;
use crate::qv_index::*;
use std::sync::atomic::{AtomicPtr, Ordering};

static QV: AtomicPtr<LispObj> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) fn set_qv_base(p: *mut LispObj) {
    QV.store(p, Ordering::Relaxed);
}

#[inline]
pub fn qv_base() -> *mut LispObj {
    QV.load(Ordering::Relaxed)
}

#[inline]
pub fn qv(i: usize) -> LispObj {
    debug_assert!(i < QV_MAX);
    unsafe { *qv_base().add(i) }
}

#[inline]
pub fn set_qv(i: usize, v: LispObj) {
    debug_assert!(i < QV_MAX);
    unsafe { *qv_base().add(i) = v }
}

#[inline]
pub fn nil() -> LispObj {
    qv(Nil_Index)
}

#[inline]
pub fn t() -> LispObj {
    qv(T_Index)
}

/// Lisp boolean from a Rust bool.
#[inline]
pub fn lbool(b: bool) -> LispObj {
    if b { t() } else { nil() }
}

/// Number of values returned by the last kernel primitive.
pub fn num_return_values() -> usize {
    qv(PORT_NUM_RETURN_VALUES_Index).integer() as usize
}

pub fn set_num_return_values(n: usize) {
    set_qv(PORT_NUM_RETURN_VALUES_Index, LispObj::fixnum(n as i64));
}
