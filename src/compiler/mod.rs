//! Kernel compiler (port of `Compx86.cpp`, restructured) and the kernel
//! `eval` / `load` built on it.

pub mod codegen;
pub mod frontend;

use crate::error::lisp_error;
use crate::heap;
use crate::object::*;
use crate::qv::{nil, qv};
use crate::qv_index::*;
use crate::runtime::{lisp_call, make_function, without_gc};
use crate::streams;
use crate::symbols::find_symbol;

/// Compile `(lambda ...)` into a function object with a null environment.
/// The compiler keeps Lisp objects in Rust data structures, which are not
/// GC roots, so collection is inhibited while it runs.
pub fn compile_lambda_expr(lambda: LispObj, name: LispObj) -> LispObj {
    without_gc(|| {
        let mut fe = frontend::Frontend::new();
        let id = fe.convert_toplevel_lambda(lambda, name);
        let code = codegen::compile_lambda(&fe, id);
        make_function(code, nil())
    })
}

/// Compile `form` as the body of a function of no arguments
/// (upstream `compileExpression`).
pub fn compile_form(form: LispObj) -> LispObj {
    let lambda = heap::list(&[qv(LAMBDA_Index), nil(), form]);
    compile_lambda_expr(lambda, nil())
}

/// A function `(f form lexical-macros lexical-symbol-macros)` that calls
/// macro expander `f` with `*LEXICAL-MACROS*` and `*LEXICAL-SYMBOL-MACROS*`
/// bound. Compiled on first use and kept in QV.
pub(crate) fn macro_caller() -> LispObj {
    let f = qv(PORT_MACRO_CALLER_Index);
    if f.is_function() {
        return f;
    }
    let sym = find_symbol;
    let l = heap::list;
    let (f, form, lm, lsm) = (sym("F"), sym("FORM"), sym("LM"), sym("LSM"));
    let lambda = l(&[
        qv(LAMBDA_Index),
        l(&[f, form, lm, lsm]),
        l(&[
            qv(LET_Index),
            l(&[
                l(&[qv(LEXICAL_MACROS_Index), lm]),
                l(&[qv(LEXICAL_SYMBOL_MACROS_Index), lsm]),
            ]),
            l(&[qv(FUNCALL_Index), f, form, nil()]),
        ]),
    ]);
    let f = compile_lambda_expr(lambda, nil());
    crate::qv::set_qv(PORT_MACRO_CALLER_Index, f);
    f
}

fn macroexpand_toplevel(mut x: LispObj) -> LispObj {
    while x.is_cons() {
        match frontend::macro_function(x.car()) {
            Some(m) => x = lisp_call(m, &[x, nil()]),
            None => break,
        }
    }
    x
}

/// Kernel `eval`: top-level `progn` subforms are processed one at a time
/// so that macros defined by earlier subforms apply to later ones.
pub fn eval(form: LispObj) -> LispObj {
    let x = macroexpand_toplevel(form);
    if x.is_cons() && (x.car() == find_symbol("PROGN") || x.car() == find_symbol("EVAL-WHEN")) {
        let mut body = x.cdr();
        if x.car() != find_symbol("PROGN") {
            body = if body.is_cons() { body.cdr() } else { nil() };
        }
        let mut r = nil();
        crate::qv::set_num_return_values(1);
        while body.is_cons() {
            r = eval(body.car());
            body = body.cdr();
        }
        return r;
    }
    if !x.is_cons() && !x.is_symbol() {
        crate::qv::set_num_return_values(1);
        return x;
    }
    let f = compile_form(x);
    lisp_call(f, &[])
}

/// Kernel `load`: read and evaluate every form in the file.
pub fn load(path: LispObj) -> LispObj {
    if !path.is_string() {
        lisp_error("LOAD: not a string");
    }
    let p = path.string_to_rust();
    if std::fs::metadata(&p).is_err() {
        lisp_error(format!("LOAD: cannot open file {p}"));
    }
    let s = streams::make_file_input_stream(&p);
    loop {
        let line = s.uref(STREAM_LINE_NUMBER).integer() + 1;
        let r = crate::error::catch_lisp_error(|| {
            // Upstream `loadFile`: read with whatever READ is defined now.
            let eof = qv(CEof_Index);
            let read = crate::symbols::symbol_function(qv(READ_Index));
            let x = crate::prims::funcall(read, &[s, nil(), eof, nil()]);
            if x == eof {
                return false;
            }
            eval(x);
            true
        });
        match r {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => {
                streams::close_stream(s);
                if e.0.is_empty() {
                    // A Lisp error, already reported: pass it on.
                    eprintln!(";;; while loading {p} (form at line {line})");
                    crate::runtime::throw_to(qv(ERROR_SIGNAL_Index), nil());
                }
                lisp_error(format!("{e}\n  while loading {p} (form at line {line})"))
            }
        }
    }
    streams::close_stream(s);
    crate::qv::set_num_return_values(1);
    crate::qv::t()
}

/// Read and evaluate all forms in `text`; returns the values of the last.
pub fn eval_string(text: &str) -> Vec<LispObj> {
    let s = streams::make_string_input_stream(heap::string_node(text));
    let mut r = heap::list(&[nil()]);
    while let Some(x) = crate::reader::read(s) {
        let v = eval(x);
        r = last_values(v);
    }
    heap::list_to_vec(r)
}

/// Values of the last evaluation as a list (`r` = primary value).
pub fn last_values(r: LispObj) -> LispObj {
    crate::runtime::values_to_list(r, crate::qv::num_return_values())
}

