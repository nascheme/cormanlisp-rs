//! Kernel reader: port of upstream `Read.cpp`.
//!
//! Like upstream, this reader is deliberately minimal: every symbol is
//! interned in COMMON-LISP (no package prefixes or keywords), integers are
//! decimal, floats are doubles. `read.lisp` replaces it during bootstrap.
#![allow(dead_code)]

use crate::error::lisp_error;
use crate::heap::{cons, list, string_from_units, vector_node, vector_node_from_list};
use crate::object::*;
use crate::prims::{check_args, funcall};
use crate::qv::{nil, qv, set_num_return_values, t};
use crate::qv_index::*;
use crate::streams::{get_character, putback_character};
use crate::symbols::{find_symbol, find_symbol_str, symbol_function, symbol_value};

fn readtable() -> LispObj {
    symbol_value(qv(READTABLE_Index))
}

fn rt_table(rt: LispObj) -> LispObj {
    rt.uref(READTABLE_TABLE)
}

/// Readtable entries cover chars 0..255; others are constituents.
/// PORT: upstream indexes out of bounds for chars >= 256.
fn char_type(rt: LispObj, c: u32) -> LispObj {
    if c < 256 {
        rt_table(rt).svref(c as usize * 2)
    } else {
        qv(CONSTITUENT_CHAR_TYPE_Index)
    }
}

fn char_func(rt: LispObj, c: u32) -> LispObj {
    rt_table(rt).svref(c as usize * 2 + 1)
}

fn set_entry(rt: LispObj, c: u32, ty: usize, f: LispObj) {
    rt_table(rt).svset(c as usize * 2, qv(ty));
    rt_table(rt).svset(c as usize * 2 + 1, f);
}

fn is_type(ty: LispObj, idx: usize) -> bool {
    ty == qv(idx)
}

pub fn terminating_macro(rt: LispObj, c: u32, f: LispObj) {
    set_entry(rt, c, TERMINATING_MACRO_CHAR_TYPE_Index, f);
}

pub fn non_terminating_macro(rt: LispObj, c: u32, f: LispObj) {
    set_entry(rt, c, NON_TERMINATING_MACRO_CHAR_TYPE_Index, f);
}

pub fn add_dispatching_non_terminating_macro(rt: LispObj, c: u32) {
    set_entry(rt, c, DISPATCHING_NON_TERMINATING_MACRO_CHAR_TYPE_Index, vector_node(256));
}

fn is_dispatching(ty: LispObj) -> bool {
    is_type(ty, DISPATCHING_NON_TERMINATING_MACRO_CHAR_TYPE_Index)
        || is_type(ty, DISPATCHING_TERMINATING_MACRO_CHAR_TYPE_Index)
}

pub fn set_dispatch_function(rt: LispObj, c: u32, sub: u32, f: LispObj) {
    if is_dispatching(char_type(rt, c)) && sub < 256 {
        char_func(rt, c).svset(sub as usize, f);
    }
}

pub fn dispatch_function(rt: LispObj, c: u32, sub: u32) -> LispObj {
    if !is_dispatching(char_type(rt, c)) || sub >= 256 {
        return nil();
    }
    char_func(rt, c).svref(sub as usize)
}

/// Upstream `readtableNode`: the standard kernel readtable.
pub fn readtable_node() -> LispObj {
    let table = vector_node(512);
    for i in (0..512).step_by(2) {
        table.svset(i, qv(CONSTITUENT_CHAR_TYPE_Index));
        table.svset(i + 1, nil());
    }
    for c in [b'\t', b' ', 12, b'\r', b'\n', 0] {
        table.svset(c as usize * 2, qv(WHITESPACE_CHAR_TYPE_Index));
    }
    table.svset(b'\\' as usize * 2, qv(SINGLE_ESCAPE_CHAR_TYPE_Index));
    table.svset(b'|' as usize * 2, qv(MULTIPLE_ESCAPE_CHAR_TYPE_Index));

    let rt = crate::heap::alloc_uvector(READTABLE_SIZE, READ_TABLE_TYPE);
    rt.uset(READTABLE_BACKQUOTE_PROCESSING, nil());
    rt.uset(READTABLE_READ_LEVEL, LispObj(0));
    rt.uset(READTABLE_TABLE, table);
    rt.uset(READTABLE_CASE, nil());

    let f = |name: &str| symbol_function(find_symbol(name));
    terminating_macro(rt, '"' as u32, f("%DOUBLEQUOTEMACRO"));
    terminating_macro(rt, '\'' as u32, f("%QUOTEMACRO"));
    terminating_macro(rt, '(' as u32, f("%LEFTPARENMACRO"));
    terminating_macro(rt, ')' as u32, f("%RIGHTPARENMACRO"));
    terminating_macro(rt, ',' as u32, f("%COMMAMACRO"));
    terminating_macro(rt, ';' as u32, f("%SEMICOLONMACRO"));
    terminating_macro(rt, '`' as u32, f("%BACKQUOTEMACRO"));
    add_dispatching_non_terminating_macro(rt, '#' as u32);
    set_dispatch_function(rt, '#' as u32, '\'' as u32, f("%POUNDQUOTEMACRO"));
    set_dispatch_function(rt, '#' as u32, '(' as u32, f("%POUNDLEFTPARENMACRO"));
    set_dispatch_function(rt, '#' as u32, '\\' as u32, f("%POUNDBACKSLASHMACRO"));
    set_dispatch_function(rt, '#' as u32, '|' as u32, f("%BRACKETEDCOMMENTMACRO"));
    rt
}

fn eof() -> LispObj {
    qv(CEof_Index)
}

fn upper(c: LispObj) -> LispObj {
    let code = c.char_code();
    if (b'a' as u32..=b'z' as u32).contains(&code) {
        LispObj::character(code - 32)
    } else {
        c
    }
}

fn get_char_or_error(stream: LispObj) -> LispObj {
    let c = get_character(stream);
    if c == eof() {
        lisp_error("Unexpected end of file");
    }
    c
}

/// Result of reading one expression.
pub enum ReadResult {
    Eof,
    /// A macro returned no values (e.g. a comment).
    NoValue,
    Value(LispObj),
}

/// Upstream `readExpression`: the reader state machine (CLtL2 pp. 335-338).
pub fn read_expression(stream: LispObj) -> ReadResult {
    let rt = readtable();
    let mut token: Vec<LispChar> = Vec::new();
    let mut state = 1;
    loop {
        match state {
            1 => {
                let x = get_character(stream);
                if x == eof() {
                    return ReadResult::Eof;
                }
                let ty = char_type(rt, x.char_code());
                if is_type(ty, ILLEGAL_CHAR_TYPE_Index) {
                    lisp_error("Illegal character found in input");
                } else if is_type(ty, WHITESPACE_CHAR_TYPE_Index) {
                } else if is_type(ty, TERMINATING_MACRO_CHAR_TYPE_Index)
                    || is_type(ty, NON_TERMINATING_MACRO_CHAR_TYPE_Index)
                {
                    let r = funcall(char_func(rt, x.char_code()), &[stream, x]);
                    return values_result(r);
                } else if is_dispatching(ty) {
                    let mut y = upper(get_char_or_error(stream));
                    let mut arg = nil();
                    if is_digit(y) {
                        let mut n: i64 = 0;
                        while is_digit(y) {
                            n = n * 10 + (y.char_code() - '0' as u32) as i64;
                            y = upper(get_char_or_error(stream));
                        }
                        arg = LispObj::fixnum(n);
                    }
                    let f = dispatch_function(rt, x.char_code(), y.char_code());
                    if f == UNINITIALIZED || f == nil() || f.raw() == 0 {
                        lisp_error("Invalid input form");
                    }
                    let r = funcall(f, &[stream, y, arg]);
                    return values_result(r);
                } else if is_type(ty, SINGLE_ESCAPE_CHAR_TYPE_Index) {
                    let y = get_char_or_error(stream);
                    token.push(y.char_code() as LispChar);
                    state = 8;
                } else if is_type(ty, MULTIPLE_ESCAPE_CHAR_TYPE_Index) {
                    state = 9;
                } else if is_type(ty, CONSTITUENT_CHAR_TYPE_Index) {
                    token.push(upper(x).char_code() as LispChar);
                    state = 8;
                } else {
                    lisp_error("Invalid character type");
                }
            }
            8 => {
                let y = get_character(stream);
                if y == eof() {
                    state = 10;
                    continue;
                }
                let ty = char_type(rt, y.char_code());
                if is_type(ty, CONSTITUENT_CHAR_TYPE_Index)
                    || is_type(ty, NON_TERMINATING_MACRO_CHAR_TYPE_Index)
                    || is_type(ty, DISPATCHING_NON_TERMINATING_MACRO_CHAR_TYPE_Index)
                {
                    token.push(upper(y).char_code() as LispChar);
                } else if is_type(ty, SINGLE_ESCAPE_CHAR_TYPE_Index) {
                    let z = get_char_or_error(stream);
                    token.push(z.char_code() as LispChar);
                } else if is_type(ty, MULTIPLE_ESCAPE_CHAR_TYPE_Index) {
                    state = 9;
                } else if is_type(ty, ILLEGAL_CHAR_TYPE_Index) {
                    lisp_error("Illegal character found in input");
                } else if is_type(ty, TERMINATING_MACRO_CHAR_TYPE_Index)
                    || is_type(ty, DISPATCHING_TERMINATING_MACRO_CHAR_TYPE_Index)
                {
                    putback_character(stream, y);
                    state = 10;
                } else if is_type(ty, WHITESPACE_CHAR_TYPE_Index) {
                    state = 10;
                }
            }
            9 => {
                let y = get_char_or_error(stream);
                let ty = char_type(rt, y.char_code());
                if is_type(ty, SINGLE_ESCAPE_CHAR_TYPE_Index) {
                    let z = get_char_or_error(stream);
                    token.push(z.char_code() as LispChar);
                } else if is_type(ty, MULTIPLE_ESCAPE_CHAR_TYPE_Index) {
                    state = 8;
                } else if is_type(ty, ILLEGAL_CHAR_TYPE_Index) {
                    lisp_error("Illegal character found in input");
                } else {
                    token.push(y.char_code() as LispChar);
                }
            }
            _ => {
                // state 10: token complete
                if let Some(n) = create_number(&token) {
                    return ReadResult::Value(n);
                }
                return ReadResult::Value(find_symbol_str(string_from_units(&token)));
            }
        }
    }
}

fn values_result(r: LispObj) -> ReadResult {
    if crate::qv::num_return_values() == 0 {
        ReadResult::NoValue
    } else {
        ReadResult::Value(r)
    }
}

fn is_digit(c: LispObj) -> bool {
    (b'0' as u32..=b'9' as u32).contains(&c.char_code())
}

/// Upstream `createNumber`: integer or double float, else `None`.
pub fn create_number(token: &[LispChar]) -> Option<LispObj> {
    let s: String = token.iter().map(|&u| char::from_u32(u as u32).unwrap_or('\u{fffd}')).collect();
    valid_integer(&s).or_else(|| valid_float(&s))
}

/// Upstream `validInteger`: optional sign, decimal digits, optional
/// trailing '.'.
fn valid_integer(s: &str) -> Option<LispObj> {
    let b = s.as_bytes();
    let (neg, start) = match b.first() {
        Some(b'+') => (false, 1),
        Some(b'-') => (true, 1),
        _ => (false, 0),
    };
    let mut i = start;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let digits = &s[start..i];
    if i < b.len() && b[i] == b'.' {
        i += 1;
    }
    if i != b.len() || digits.is_empty() {
        return None;
    }
    let v: i64 = match digits.parse::<i64>() {
        Ok(v) => if neg { -v } else { v },
        Err(_) => lisp_error(format!("Integer too large: {s}")), // PORT: no bignums yet
    };
    match LispObj::fixnum_checked(v) {
        Some(o) => Some(o),
        None => lisp_error(format!("Integer too large: {s}")),
    }
}

/// Upstream `validFloat`. PORT: malformed tokens (two decimal points,
/// empty exponent) become symbols instead of upstream's fixnum 0.
fn valid_float(s: &str) -> Option<LispObj> {
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
                    return None;
                }
            }
            _ => break,
        }
        i += 1;
    }
    if digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'E' || b[i] == b'e') {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let e0 = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == e0 {
            return None;
        }
    }
    if i != b.len() {
        return None;
    }
    let d: f64 = s.parse().ok()?;
    Some(crate::heap::double_float_node(d))
}

/// Upstream kernel `READ`: (read stream &optional eof-error-p eof-value
/// recursive-p).
pub fn read_prim(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 4);
    let s = args[0];
    let eof_error_p = args.get(1).copied().unwrap_or(t());
    let eof_value = args.get(2).copied().unwrap_or(eof());
    if !s.is_stream() {
        lisp_error("Not an input stream");
    }
    loop {
        match read_expression(s) {
            ReadResult::Eof => {
                if eof_error_p != nil() {
                    lisp_error("End of file encountered");
                }
                set_num_return_values(1);
                return eof_value;
            }
            ReadResult::Value(v) => {
                set_num_return_values(1);
                return v;
            }
            ReadResult::NoValue => {}
        }
    }
}

/// Convenience: read one object, or `None` at end of file.
pub fn read(stream: LispObj) -> Option<LispObj> {
    let marker = UNINITIALIZED;
    let r = read_prim(&[stream, nil(), marker]);
    (r != marker).then_some(r)
}

fn peek_char_skipping_ws(rt: LispObj, s: LispObj) -> LispObj {
    loop {
        let c = get_char_or_error(s);
        if !is_type(char_type(rt, c.char_code()), WHITESPACE_CHAR_TYPE_Index) {
            putback_character(s, c);
            return c;
        }
    }
}

/// Upstream `Cread_delimited_list`: elements are read with the current
/// READ function (so `read.lisp` takes over reading list elements).
///
/// PORT: if READ returns the `)` token (a comment just before the
/// terminator made READ skip ahead), the list ends there.
pub fn read_delimited_list(ch: LispObj, stream: LispObj) -> LispObj {
    let rt = readtable();
    let mut head = nil();
    let mut tail = nil();
    let mut found_dot = 0;
    let mut count = 0;
    let dot = qv(DOT_Index);
    let rparen = qv(RIGHT_PAREN_Index);
    loop {
        let c = peek_char_skipping_ws(rt, stream);
        if c == ch {
            get_character(stream);
            if found_dot == 1 {
                lisp_error("List ends with dot");
            }
            set_num_return_values(1);
            return head;
        }
        let read = symbol_function(qv(READ_Index));
        let n = funcall(read, &[stream, t(), nil(), t()]);
        if crate::qv::num_return_values() == 0 {
            continue;
        }
        if n == rparen && ch == LispObj::character(')' as u32) {
            if found_dot == 1 {
                lisp_error("List ends with dot");
            }
            set_num_return_values(1);
            return head;
        }
        if c == LispObj::character('.' as u32) && n == dot {
            if found_dot != 0 {
                lisp_error("Multiple dots in list");
            }
            if count == 0 {
                lisp_error("Invalid position for dot");
            }
            found_dot = 1;
            continue;
        }
        if found_dot == 1 {
            if tail == nil() {
                head = n;
                tail = n;
            } else {
                tail.set_cdr(n);
            }
            found_dot = 2;
        } else {
            if found_dot >= 2 {
                lisp_error("Error in dotted expression");
            }
            let cell = cons(n, nil());
            if tail == nil() {
                head = cell;
            } else {
                tail.set_cdr(cell);
            }
            tail = cell;
            count += 1;
        }
    }
}

pub fn read_delimited_list_prim(args: &[LispObj]) -> LispObj {
    check_args(args, 2, 2);
    if !args[0].is_character() {
        lisp_error("Not a character");
    }
    read_delimited_list(args[0], args[1])
}

fn read_recursive(stream: LispObj) -> LispObj {
    funcall(symbol_function(qv(READ_Index)), &[stream])
}

// ---------------------------------------------------------------------------
// Standard reader macro functions

pub fn doublequote_macro(args: &[LispObj]) -> LispObj {
    check_args(args, 2, 2);
    let (stream, delim) = (args[0], args[1]);
    let mut lit: Vec<LispChar> = Vec::new();
    loop {
        let mut c = get_char_or_error(stream);
        if c == delim {
            break;
        }
        if c == LispObj::character('\\' as u32) {
            c = get_char_or_error(stream);
        }
        lit.push(c.char_code() as LispChar);
    }
    string_from_units(&lit)
}

pub fn quote_macro(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 2);
    let x = read_recursive(args[0]);
    set_num_return_values(1);
    list(&[qv(QUOTE_Index), x])
}

pub fn leftparen_macro(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 2);
    read_delimited_list(LispObj::character(')' as u32), args[0])
}

pub fn rightparen_macro(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 2);
    qv(RIGHT_PAREN_Index)
}

pub fn semicolon_macro(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 2);
    let stream = args[0];
    loop {
        let c = get_character(stream);
        if c == LispObj::character('\r' as u32) || c == LispObj::character('\n' as u32) {
            putback_character(stream, c);
            break;
        }
        if c == eof() {
            break;
        }
    }
    set_num_return_values(0);
    nil()
}

pub fn bracketed_comment_macro(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 3);
    let stream = args[0];
    // PORT: upstream does not nest #| |# comments; nesting is supported here
    // as required by ANSI.
    let mut depth = 1;
    let mut prev = nil();
    loop {
        let c = get_character(stream);
        if c == eof() {
            break;
        }
        if prev == LispObj::character('|' as u32) && c == LispObj::character('#' as u32) {
            depth -= 1;
            if depth == 0 {
                break;
            }
            prev = nil();
            continue;
        }
        if prev == LispObj::character('#' as u32) && c == LispObj::character('|' as u32) {
            depth += 1;
            prev = nil();
            continue;
        }
        prev = c;
    }
    set_num_return_values(0);
    nil()
}

pub fn backquote_macro(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 3);
    let rt = readtable();
    let save = rt.uref(READTABLE_BACKQUOTE_PROCESSING);
    rt.uset(READTABLE_BACKQUOTE_PROCESSING, t());
    let s = read_recursive(args[0]);
    rt.uset(READTABLE_BACKQUOTE_PROCESSING, save);
    set_num_return_values(1);
    if !s.is_cons() {
        return list(&[qv(QUOTE_Index), s]);
    }
    backquote_process(s)
}

fn is_comma_token(x: LispObj) -> bool {
    x == qv(COMMA_TOKEN_Index) || x == qv(COMMA_DOT_TOKEN_Index) || x == qv(COMMA_ATSIGN_TOKEN_Index)
}

fn contains_comma(x: LispObj) -> bool {
    let mut x = x;
    while x.is_cons() {
        if is_comma_token(x.car()) || contains_comma(x.car()) {
            return true;
        }
        x = x.cdr();
    }
    is_comma_token(x)
}

/// Upstream computes `eval(backquoteProcess(x))` for a cons following a
/// comma. For a form without comma tokens that is just `x` (backquote
/// processing then evaluation rebuilds it), so no evaluator is needed.
fn unprocess(x: LispObj) -> LispObj {
    if contains_comma(x) {
        lisp_error("Nested comma inside comma form is not supported by the kernel reader");
    }
    x
}

/// Upstream `backquoteProcess`.
fn backquote_process(s0: LispObj) -> LispObj {
    let mut s = s0;
    if !s.is_cons() {
        return s;
    }
    let (q, l) = (qv(QUOTE_Index), qv(LIST_Index));
    let (comma, comma_dot, comma_at) =
        (qv(COMMA_TOKEN_Index), qv(COMMA_DOT_TOKEN_Index), qv(COMMA_ATSIGN_TOKEN_Index));
    let newlis = cons(qv(APPEND_Index), nil());
    let mut np = newlis;
    let push = |np: &mut LispObj, x: LispObj| {
        np.set_cdr(cons(x, nil()));
        *np = np.cdr();
    };
    while s.is_cons() {
        let n = s.car();
        if !n.is_cons() {
            // comma after a dot: `(a . ,b)
            if is_comma_token(n) && s.cdr().is_cons() {
                let arg = s.cdr().car();
                let v = if arg.is_cons() { unprocess(arg) } else { arg };
                if n == comma {
                    np.set_cdr(cons(v, nil()));
                } else {
                    np.set_cdr(v);
                }
                return newlis;
            }
            push(&mut np, list(&[l, list(&[q, n])]));
        } else if n.car() == comma && n.cdr().is_cons() {
            let arg = n.cdr().car();
            let v = if arg.is_cons() { unprocess(arg) } else { arg };
            push(&mut np, list(&[l, v]));
        } else if (n.car() == comma_dot || n.car() == comma_at) && n.cdr().is_cons() {
            let arg = n.cdr().car();
            let v = if arg.is_cons() { unprocess(arg) } else { arg };
            push(&mut np, v);
        } else {
            push(&mut np, list(&[l, backquote_process(n)]));
        }
        s = s.cdr();
    }
    if s != nil() {
        push(&mut np, list(&[q, s]));
    }
    newlis
}

pub fn comma_macro(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 3);
    let rt = readtable();
    let stream = args[0];
    if rt.uref(READTABLE_BACKQUOTE_PROCESSING) == nil() {
        lisp_error("Invalid form: comma found outside backquote form");
    }
    let c = get_char_or_error(stream);
    let tok = if c == LispObj::character('@' as u32) {
        COMMA_ATSIGN_TOKEN_Index
    } else if c == LispObj::character('.' as u32) {
        COMMA_DOT_TOKEN_Index
    } else {
        putback_character(stream, c);
        COMMA_TOKEN_Index
    };
    let x = read_recursive(stream);
    set_num_return_values(1);
    list(&[qv(tok), x])
}

pub fn pound_leftparen_macro(args: &[LispObj]) -> LispObj {
    check_args(args, 2, 3);
    let (stream, ch) = (args[0], args[1]);
    putback_character(stream, ch);
    let n = read_recursive(stream);
    set_num_return_values(1);
    if !(n.is_cons() || n == nil()) {
        lisp_error("Expected a list while reading");
    }
    let len = match args.get(2) {
        Some(a) if a.is_fixnum() => a.integer() as usize,
        _ => list_length(n),
    };
    vector_node_from_list(len, n)
}

fn list_length(mut x: LispObj) -> usize {
    let mut n = 0;
    while x.is_cons() {
        n += 1;
        x = x.cdr();
    }
    n
}

pub fn pound_quote_macro(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 3);
    let x = read_recursive(args[0]);
    set_num_return_values(1);
    list(&[qv(FUNCTION_Index), x])
}

pub fn pound_backslash_macro(args: &[LispObj]) -> LispObj {
    check_args(args, 3, 3);
    let c = get_character(args[0]);
    if c == eof() {
        lisp_error("End of file encountered");
    }
    c
}
