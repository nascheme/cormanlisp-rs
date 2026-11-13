//! Generic arithmetic on fixnums, ratios (of fixnums), single and double
//! floats (subset of upstream `LispMath.cpp`). Bignums are not supported
//! yet: integer overflow is an error.

use std::cmp::Ordering;

use crate::error::lisp_error;
use crate::heap::{alloc_uvector, double_float_node, single_float_node};
use crate::object::*;
use crate::printer::write_to_string;

#[derive(Clone, Copy)]
pub enum Num {
    Int(i64),
    Ratio(i64, i64),
    Single(f32),
    Double(f64),
}

pub const RATIO_NUMERATOR: usize = 1;
pub const RATIO_DENOMINATOR: usize = 2;

pub fn num(x: LispObj) -> Num {
    if x.is_fixnum() {
        Num::Int(x.integer())
    } else if x.is_uvector_of(RATIO_TYPE) {
        Num::Ratio(x.uref(RATIO_NUMERATOR).integer(), x.uref(RATIO_DENOMINATOR).integer())
    } else if x.is_double_float() {
        Num::Double(x.double_float())
    } else if x.is_single_float() {
        Num::Single(x.single_float())
    } else {
        lisp_error(format!("Not a number: {}", write_to_string(x)))
    }
}

pub fn is_number(x: LispObj) -> bool {
    x.is_fixnum() || x.is_float() || x.is_uvector_of(RATIO_TYPE)
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn fix128(n: i128) -> Option<i64> {
    i64::try_from(n).ok().filter(|v| (MOST_NEGATIVE_FIXNUM..=MOST_POSITIVE_FIXNUM).contains(v))
}

/// Normalized rational n/d: a fixnum if d divides n, else a ratio.
pub fn make_rational(n: i128, d: i128) -> LispObj {
    if d == 0 {
        lisp_error("Division by zero");
    }
    let g = gcd(n, d);
    let (mut n, mut d) = (n / g, d / g);
    if d < 0 {
        n = -n;
        d = -d;
    }
    let (Some(n), Some(d)) = (fix128(n), fix128(d)) else {
        lisp_error("Integer overflow (bignums are not supported yet)")
    };
    if d == 1 {
        return LispObj::fixnum(n);
    }
    let r = alloc_uvector(RATIO_DENOMINATOR, RATIO_TYPE);
    r.uset(RATIO_NUMERATOR, LispObj::fixnum(n));
    r.uset(RATIO_DENOMINATOR, LispObj::fixnum(d));
    r
}

fn rat(n: Num) -> Option<(i128, i128)> {
    match n {
        Num::Int(i) => Some((i as i128, 1)),
        Num::Ratio(a, b) => Some((a as i128, b as i128)),
        _ => None,
    }
}

pub fn check_number(x: LispObj) {
    num(x);
}

pub fn integer_value(x: LispObj) -> i64 {
    if !x.is_fixnum() {
        lisp_error(format!("Not an integer: {}", write_to_string(x)));
    }
    x.integer()
}

pub fn int_result(n: Option<i64>) -> LispObj {
    match n.and_then(LispObj::fixnum_checked) {
        Some(v) => v,
        None => lisp_error("Integer overflow (bignums are not supported yet)"),
    }
}

pub fn as_f64(n: Num) -> f64 {
    match n {
        Num::Int(i) => i as f64,
        Num::Ratio(a, b) => a as f64 / b as f64,
        Num::Single(f) => f as f64,
        Num::Double(f) => f,
    }
}

/// Float result with the contagion of the two operands.
fn float_result(a: Num, b: Num, v: f64) -> LispObj {
    match (a, b) {
        (Num::Double(_), _) | (_, Num::Double(_)) => double_float_node(v),
        _ => single_float_node(v as f32),
    }
}

type RatOp = fn((i128, i128), (i128, i128)) -> (i128, i128);

fn arith(a: LispObj, b: LispObj, fr: RatOp, ff: fn(f64, f64) -> f64) -> LispObj {
    let (x, y) = (num(a), num(b));
    match (rat(x), rat(y)) {
        (Some(p), Some(q)) => {
            let (n, d) = fr(p, q);
            make_rational(n, d)
        }
        _ => float_result(x, y, ff(as_f64(x), as_f64(y))),
    }
}

pub fn add(a: LispObj, b: LispObj) -> LispObj {
    arith(a, b, |(a, b), (c, d)| (a * d + c * b, b * d), |x, y| x + y)
}

pub fn sub(a: LispObj, b: LispObj) -> LispObj {
    arith(a, b, |(a, b), (c, d)| (a * d - c * b, b * d), |x, y| x - y)
}

pub fn mul(a: LispObj, b: LispObj) -> LispObj {
    arith(a, b, |(a, b), (c, d)| (a * c, b * d), |x, y| x * y)
}

pub fn div(a: LispObj, b: LispObj) -> LispObj {
    let (x, y) = (num(a), num(b));
    match (rat(x), rat(y)) {
        (_, Some((0, _))) => lisp_error("Division by zero"),
        (Some((a, b)), Some((c, d))) => make_rational(a * d, b * c),
        _ => {
            if as_f64(y) == 0.0 {
                lisp_error("Division by zero");
            }
            float_result(x, y, as_f64(x) / as_f64(y))
        }
    }
}

pub fn compare(a: LispObj, b: LispObj) -> Ordering {
    let (x, y) = (num(a), num(b));
    match (rat(x), rat(y)) {
        (Some((a, b)), Some((c, d))) => (a * d).cmp(&(c * b)),
        _ => as_f64(x).partial_cmp(&as_f64(y)).unwrap_or(Ordering::Less),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    Trunc,
    Floor,
    Ceiling,
    Round,
}

fn round_f(q: f64, mode: Rounding) -> f64 {
    match mode {
        Rounding::Trunc => q.trunc(),
        Rounding::Floor => q.floor(),
        Rounding::Ceiling => q.ceil(),
        Rounding::Round => q.round_ties_even(),
    }
}

/// Integer division with the given rounding. Returns (quotient,
/// remainder); the remainder has the type of the float contagion.
pub fn int_divide(a: LispObj, b: LispObj, mode: Rounding) -> (LispObj, LispObj) {
    match (num(a), num(b)) {
        (_, Num::Int(0)) => lisp_error("Division by zero"),
        (x @ Num::Ratio(..), y) | (x, y @ Num::Ratio(..))
            if rat(x).is_some() && rat(y).is_some() =>
        {
            // q = round(x/y); r = x - q*y
            let ((p, q), (r, s)) = (rat(x).unwrap(), rat(y).unwrap());
            let (n, d) = (p * s, q * r);
            let (n, d) = if d < 0 { (-n, -d) } else { (n, d) };
            let fl = n.div_euclid(d);
            let rem = n - fl * d;
            let quot = match mode {
                Rounding::Floor => fl,
                Rounding::Ceiling => fl + i128::from(rem != 0),
                Rounding::Trunc => fl + i128::from(rem != 0 && n < 0),
                Rounding::Round => {
                    if 2 * rem > d || (2 * rem == d && fl % 2 != 0) { fl + 1 } else { fl }
                }
            };
            let Some(qf) = fix128(quot) else {
                lisp_error("Integer overflow (bignums are not supported yet)")
            };
            let qo = LispObj::fixnum(qf);
            (qo, sub(a, mul(qo, b)))
        }
        (Num::Int(x), Num::Int(y)) => {
            let (mut q, mut r) = (x.wrapping_div(y), x.wrapping_rem(y));
            if r != 0 {
                let adjust = match mode {
                    Rounding::Trunc => 0,
                    Rounding::Floor => -i64::from((r < 0) != (y < 0)),
                    Rounding::Ceiling => i64::from((r < 0) == (y < 0)),
                    Rounding::Round => {
                        let twice = (r.unsigned_abs() as u128) * 2;
                        let ay = y.unsigned_abs() as u128;
                        let sign = if (x < 0) != (y < 0) { -1 } else { 1 };
                        if twice > ay || (twice == ay && q % 2 != 0) { sign } else { 0 }
                    }
                };
                q += adjust;
                r = x - q * y;
            }
            (int_result(Some(q)), LispObj::fixnum(r))
        }
        (x, y) => {
            let (xf, yf) = (as_f64(x), as_f64(y));
            if yf == 0.0 {
                lisp_error("Division by zero");
            }
            let q = round_f(xf / yf, mode);
            if !q.is_finite() || q.abs() >= 2f64.powi(60) {
                lisp_error("Integer overflow (bignums are not supported yet)");
            }
            (LispObj::fixnum(q as i64), float_result(x, y, xf - q * yf))
        }
    }
}

/// Convert to a float of the same format as `proto` (single if NIL).
pub fn to_float(x: LispObj, double: bool) -> LispObj {
    let n = num(x);
    match (n, double) {
        (Num::Double(_), true) | (Num::Single(_), false) => x,
        (_, true) => double_float_node(as_f64(n)),
        (_, false) => single_float_node(as_f64(n) as f32),
    }
}

/// Apply a float function, keeping single floats single.
pub fn float_fn(x: LispObj, f: fn(f64) -> f64) -> LispObj {
    let n = num(x);
    let v = f(as_f64(n));
    match n {
        Num::Double(_) => double_float_node(v),
        _ => single_float_node(v as f32),
    }
}
