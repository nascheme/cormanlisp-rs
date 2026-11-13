//! Core kernel primitives needed by the bootstrap (subset of upstream
//! `Lispfunc.cpp`; most are redefined later in Lisp).

use super::{check_args, funcall};
use crate::error::lisp_error;
use crate::heap::{self, cons, string_node};
use crate::numbers;
use crate::object::*;
use crate::printer::{self, write_to_string};
use crate::qv::{lbool, nil, qv, t};
use crate::qv_index::*;
use crate::runtime::set_values;
use crate::streams;
use crate::symbols::{self, *};

fn check_symbol(x: LispObj) {
    if !x.is_symbol() {
        lisp_error(format!("Not a symbol: {}", write_to_string(x)));
    }
}

fn check_list(x: LispObj) {
    if !(x.is_cons() || x == nil()) {
        lisp_error(format!("Not a list: {}", write_to_string(x)));
    }
}

// ---------------------------------------------------------------------------
// Symbols and functions

pub fn symbol_value_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_symbol(a[0]);
    let v = symbol_value(a[0]);
    if v == SYMBOL_UNBOUND {
        lisp_error(format!("The variable {} is unbound", write_to_string(a[0])));
    }
    v
}

/// Upstream `SymbolFunction`: the function cell (also for macros and
/// special operators); an error only if it is empty.
pub fn symbol_function_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_symbol(a[0]);
    let f = symbol_function(a[0]);
    if f == UNINITIALIZED {
        lisp_error(format!(
            "Symbol does not have a function binding: {}",
            write_to_string(a[0])
        ));
    }
    f
}

pub fn macro_function_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 2);
    check_symbol(a[0]);
    crate::compiler::frontend::macro_function(a[0]).unwrap_or_else(nil)
}

pub fn set_symbol_function_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    check_symbol(a[1]);
    set_symbol_function(a[1], a[0], qv(FUNCTION_Index));
    a[0]
}

pub fn set_symbol_macro_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    check_symbol(a[1]);
    set_symbol_function(a[1], a[0], qv(MACRO_Index));
    a[0]
}

pub fn symbol_get_flags(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_symbol(a[0]);
    a[0].uref(SYMBOL_FLAGS)
}

pub fn symbol_set_flags(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    check_symbol(a[1]);
    if !a[0].is_fixnum() {
        lisp_error("%SYMBOL-SET-FLAGS: flags must be a fixnum");
    }
    a[1].uset(SYMBOL_FLAGS, a[0]);
    a[0]
}

pub fn constantp(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 2);
    let x = a[0];
    let r = if x.is_symbol() {
        x == nil() || is_constant_symbol(x)
    } else if x.is_cons() {
        x.car() == qv(QUOTE_Index)
    } else {
        true
    };
    lbool(r)
}

static GENSYM_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

pub fn gensym(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 1);
    let prefix = match a.first() {
        Some(p) if p.is_string() => p.string_to_rust(),
        _ => "G".to_string(),
    };
    let n = GENSYM_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    symbol_node(string_node(&format!("{prefix}{n}")))
}

pub fn funcall_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, usize::MAX);
    funcall(a[0], &a[1..])
}

pub fn apply_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 2, usize::MAX);
    let mut args: Vec<LispObj> = a[1..a.len() - 1].to_vec();
    let mut l = a[a.len() - 1];
    check_list(l);
    while l.is_cons() {
        args.push(l.car());
        l = l.cdr();
    }
    funcall(a[0], &args)
}

pub fn values_prim(a: &[LispObj]) -> LispObj {
    set_values(a)
}

pub fn eval_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    crate::compiler::eval(a[0])
}

pub fn load_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, usize::MAX);
    crate::compiler::load(a[0])
}

pub fn compile_form_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    crate::compiler::compile_form(a[0])
}

pub fn compile_lambda_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 2);
    crate::compiler::compile_lambda_expr(a[0], a.get(1).copied().unwrap_or_else(nil))
}

/// (ERROR format-string &rest args). Kernel version: `~A`/`~S` directives
/// only; the message is thrown to the kernel error catcher.
pub fn error_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, usize::MAX);
    let msg = if a[0].is_string() {
        simple_format(&a[0].string_to_rust(), &a[1..])
    } else {
        write_to_string(a[0])
    };
    lisp_error(msg)
}

pub fn simple_format(fmt: &str, args: &[LispObj]) -> String {
    let mut out = String::new();
    let mut it = fmt.chars().peekable();
    let mut ai = 0;
    while let Some(c) = it.next() {
        if c != '~' {
            out.push(c);
            continue;
        }
        match it.next().map(|c| c.to_ascii_uppercase()) {
            Some('A') | Some('S') => {
                if let Some(&x) = args.get(ai) {
                    out.push_str(&write_to_string(x));
                }
                ai += 1;
            }
            Some('%') => out.push('\n'),
            Some('~') => out.push('~'),
            Some(o) => {
                out.push('~');
                out.push(o);
            }
            None => out.push('~'),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Lists

pub fn car_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_list(a[0]);
    if a[0].is_cons() { a[0].car() } else { nil() }
}

pub fn cdr_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    check_list(a[0]);
    if a[0].is_cons() { a[0].cdr() } else { nil() }
}

pub fn cons_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    cons(a[0], a[1])
}

pub fn rplaca(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    if !a[0].is_cons() {
        lisp_error("RPLACA: not a cons");
    }
    a[0].set_car(a[1]);
    a[0]
}

pub fn rplacd(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    if !a[0].is_cons() {
        lisp_error("RPLACD: not a cons");
    }
    a[0].set_cdr(a[1]);
    a[0]
}

pub fn list_prim(a: &[LispObj]) -> LispObj {
    heap::list(a)
}

pub fn list_star(a: &[LispObj]) -> LispObj {
    check_args(a, 1, usize::MAX);
    let mut r = a[a.len() - 1];
    for &x in a[..a.len() - 1].iter().rev() {
        r = cons(x, r);
    }
    r
}

pub fn append_prim(a: &[LispObj]) -> LispObj {
    if a.is_empty() {
        return nil();
    }
    // Copy the lists front to back (no Rust Vec: it would not be a GC root).
    let mut head = nil();
    let mut tail = nil();
    for &l in &a[..a.len() - 1] {
        check_list(l);
        let mut p = l;
        while p.is_cons() {
            let c = cons(p.car(), nil());
            if tail.is_cons() {
                tail.set_cdr(c);
            } else {
                head = c;
            }
            tail = c;
            p = p.cdr();
        }
    }
    let last = a[a.len() - 1];
    if tail.is_cons() {
        tail.set_cdr(last);
        head
    } else {
        last
    }
}

// ---------------------------------------------------------------------------
// Predicates

macro_rules! pred {
    ($name:ident, $test:expr) => {
        pub fn $name(a: &[LispObj]) -> LispObj {
            check_args(a, 1, 1);
            let f: fn(LispObj) -> bool = $test;
            lbool(f(a[0]))
        }
    };
}

pred!(null_prim, |x| x == nil());
pred!(consp, |x| x.is_cons());
pred!(listp, |x| x.is_cons() || x == nil());
pred!(symbolp, |x| x.is_symbol());
pred!(stringp, |x| x.is_string());
pred!(fixnump, |x| x.is_fixnum());
pred!(integerp, |x| x.is_fixnum());
pred!(floatp, |x| x.is_float());
pred!(characterp, |x| x.is_character());
pred!(vectorp, |x| x.is_vector());
pred!(arrayp, |x| x.is_array());
pred!(functionp, crate::runtime::is_funcallable);
pred!(streamp, |x| x.is_stream());
pred!(packagep, |x| x.is_package());

pub fn eq_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    lbool(a[0] == a[1])
}

// ---------------------------------------------------------------------------
// Numbers

pub fn plus(a: &[LispObj]) -> LispObj {
    let mut r = LispObj::fixnum(0);
    for &x in a {
        r = numbers::add(r, x);
    }
    r
}

pub fn times(a: &[LispObj]) -> LispObj {
    let mut r = LispObj::fixnum(1);
    for &x in a {
        r = numbers::mul(r, x);
    }
    r
}

pub fn minus(a: &[LispObj]) -> LispObj {
    check_args(a, 1, usize::MAX);
    if a.len() == 1 {
        return numbers::sub(LispObj::fixnum(0), a[0]);
    }
    a[1..].iter().fold(a[0], |r, &x| numbers::sub(r, x))
}

pub fn divide(a: &[LispObj]) -> LispObj {
    check_args(a, 1, usize::MAX);
    if a.len() == 1 {
        return numbers::div(LispObj::fixnum(1), a[0]);
    }
    a[1..].iter().fold(a[0], |r, &x| numbers::div(r, x))
}

fn compare_chain(a: &[LispObj], ok: fn(std::cmp::Ordering) -> bool) -> LispObj {
    check_args(a, 1, usize::MAX);
    for &x in a {
        numbers::check_number(x);
    }
    lbool(a.windows(2).all(|w| ok(numbers::compare(w[0], w[1]))))
}

pub fn num_eq(a: &[LispObj]) -> LispObj {
    compare_chain(a, |o| o.is_eq())
}
pub fn num_lt(a: &[LispObj]) -> LispObj {
    compare_chain(a, |o| o.is_lt())
}
pub fn num_le(a: &[LispObj]) -> LispObj {
    compare_chain(a, |o| o.is_le())
}
pub fn num_gt(a: &[LispObj]) -> LispObj {
    compare_chain(a, |o| o.is_gt())
}
pub fn num_ge(a: &[LispObj]) -> LispObj {
    compare_chain(a, |o| o.is_ge())
}

pub fn num_ne(a: &[LispObj]) -> LispObj {
    check_args(a, 1, usize::MAX);
    for i in 0..a.len() {
        for j in i + 1..a.len() {
            if numbers::compare(a[i], a[j]).is_eq() {
                return nil();
            }
        }
    }
    t()
}

fn divide_values(a: &[LispObj], mode: numbers::Rounding) -> LispObj {
    check_args(a, 1, 2);
    let d = a.get(1).copied().unwrap_or(LispObj::fixnum(1));
    let (q, r) = numbers::int_divide(a[0], d, mode);
    set_values(&[q, r])
}

pub fn truncate(a: &[LispObj]) -> LispObj {
    divide_values(a, numbers::Rounding::Trunc)
}

pub fn floor(a: &[LispObj]) -> LispObj {
    divide_values(a, numbers::Rounding::Floor)
}

pub fn ceiling(a: &[LispObj]) -> LispObj {
    divide_values(a, numbers::Rounding::Ceiling)
}

pub fn round(a: &[LispObj]) -> LispObj {
    divide_values(a, numbers::Rounding::Round)
}

pub fn mod_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    numbers::int_divide(a[0], a[1], numbers::Rounding::Floor).1
}

pub fn bit_or(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    if !a[0].is_fixnum() || !a[1].is_fixnum() {
        lisp_error("BIT-OR: not a fixnum");
    }
    LispObj(a[0].raw() | a[1].raw())
}

// ---------------------------------------------------------------------------
// Arrays

pub fn array_dimension(a: &[LispObj]) -> LispObj {
    check_args(a, 2, 2);
    let (arr, n) = (a[0], a[1]);
    if !arr.is_array() || !n.is_fixnum() {
        lisp_error("ARRAY-DIMENSION: invalid arguments");
    }
    if arr.is_adjustable_array() {
        let rank = arr.array_dimensions();
        let i = n.integer() as usize;
        if i >= rank {
            lisp_error("ARRAY-DIMENSION: axis out of range");
        }
        arr.uref(ARRAY_DIM1_LENGTH + i)
    } else {
        if n.integer() != 0 {
            lisp_error("ARRAY-DIMENSION: axis out of range");
        }
        arr.vector_length()
    }
}

// ---------------------------------------------------------------------------
// Output

fn out_stream(a: &[LispObj], i: usize) -> LispObj {
    match a.get(i) {
        Some(&s) if s.is_stream() => s,
        Some(&s) if s == t() => symbol_value(qv(TERMINAL_IO_Index)),
        _ => symbol_value(qv(STANDARD_OUTPUT_Index)),
    }
}

pub fn write_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, usize::MAX);
    // Accepts (write obj stream) and (write obj :stream stream ...).
    let s = if a.len() >= 3 && a[1] == symbols::find_keyword("STREAM") {
        out_stream(a, 2)
    } else {
        out_stream(a, 1)
    };
    printer::write(a[0], s);
    a[0]
}

pub fn terpri(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 1);
    let s = out_stream(a, 0);
    streams::output_char(s, b'\n' as u16);
    nil()
}

pub fn force_output(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 1);
    streams::flush_stream(out_stream(a, 0));
    nil()
}

// ---------------------------------------------------------------------------
// Misc

pub fn get_internal_run_time(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 0);
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    LispObj::fixnum(ts.tv_sec * 1000 + ts.tv_nsec / 1_000_000)
}

pub fn get_gc_time(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 0);
    LispObj::fixnum(0)
}

pub fn version_string(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 0);
    string_node(concat!("3.1 (Rust kernel ", env!("CARGO_PKG_VERSION"), ")"))
}

pub fn ignore(_a: &[LispObj]) -> LispObj {
    nil()
}

/// 1 = console client (upstream: 0 = none, 1 = console, 2 = IDE).
pub fn client_type(_a: &[LispObj]) -> LispObj {
    LispObj::fixnum(1)
}

pub fn find_keyword_prim(a: &[LispObj]) -> LispObj {
    check_args(a, 1, 1);
    if !a[0].is_string() {
        lisp_error("FIND-KEYWORD: not a string");
    }
    symbols::find_keyword(&a[0].string_to_rust())
}

/// Internal time units are milliseconds.
pub fn get_internal_time_units_per_second(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 0);
    LispObj::fixnum(1000)
}

pub fn get_internal_real_time(a: &[LispObj]) -> LispObj {
    check_args(a, 0, 0);
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    LispObj::fixnum(ts.tv_sec * 1000 + ts.tv_nsec / 1_000_000)
}
