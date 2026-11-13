//! Kernel streams. Streams are `StreamType` uvectors with upstream's slot
//! layout and character buffers; filling and draining the buffers is done
//! by the underflow/overflow functions stored in the stream (symbols naming
//! kernel primitives), as upstream.
//!
//! PORT: file and console handles are indices into a Rust-side table
//! instead of Win32 HANDLEs. String input is done in the kernel (upstream
//! implements it in Lisp).
#![allow(dead_code)]

use crate::error::lisp_error;
use crate::heap::{self, char_vector, string_from_units};
use crate::object::*;
use crate::prims::{check_args, funcall};
use crate::qv::{nil, qv, t};
use crate::qv_index::*;
use crate::symbols::symbol_name;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{BufRead, Write};

const BUF_LEN: usize = 0x800;

enum Source {
    Stdin,
    File(std::io::BufReader<std::fs::File>),
}

struct Handle {
    src: Source,
    /// Decoded UTF-16 units not yet moved into the Lisp buffer.
    pending: VecDeque<u16>,
    eof: bool,
}

thread_local! {
    static HANDLES: RefCell<Vec<Option<Handle>>> = const { RefCell::new(Vec::new()) };
}

/// Decode one line of input: UTF-8 if valid, else Latin-1.
fn decode_line(bytes: &[u8]) -> Vec<u16> {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.encode_utf16().collect(),
        Err(_) => bytes.iter().map(|&b| b as u16).collect(),
    }
}

fn add_handle(src: Source) -> usize {
    HANDLES.with(|h| {
        let mut h = h.borrow_mut();
        h.push(Some(Handle { src, pending: VecDeque::new(), eof: false }));
        h.len() - 1
    })
}

fn base_stream(name: LispObj, subclass: usize, direction: usize) -> LispObj {
    let s = heap::alloc_uvector(STREAM_SIZE, STREAM_TYPE);
    let n = nil();
    let zero = LispObj::fixnum(0);
    s.uset(STREAM_NAME, name);
    s.uset(STREAM_UNDERFLOW_FUNC, n);
    s.uset(STREAM_OVERFLOW_FUNC, n);
    s.uset(STREAM_POSITION, zero);
    s.uset(STREAM_COL_POSITION, zero);
    s.uset(STREAM_INPUT_BUFFER, n);
    s.uset(STREAM_INPUT_BUFFER_LENGTH, zero);
    s.uset(STREAM_INPUT_BUFFER_POS, zero);
    s.uset(STREAM_INPUT_BUFFER_NUM, zero);
    s.uset(STREAM_HANDLE, zero);
    s.uset(STREAM_SUBCLASS, qv(subclass));
    s.uset(STREAM_BINARY, n);
    s.uset(STREAM_OPEN, t());
    s.uset(STREAM_DIRECTION, qv(direction));
    s.uset(STREAM_INTERACTIVE, n);
    s.uset(STREAM_ELEMENT_TYPE, qv(CHARACTER_Index));
    s.uset(STREAM_ASSOCIATED_STREAMS, n);
    s.uset(STREAM_OUTPUT_BUFFER, n);
    s.uset(STREAM_OUTPUT_BUFFER_LENGTH, zero);
    s.uset(STREAM_OUTPUT_BUFFER_POS, zero);
    s.uset(STREAM_LINE_NUMBER, zero);
    s
}

fn add_input_buffer(s: LispObj, underflow: usize) {
    s.uset(STREAM_UNDERFLOW_FUNC, qv(underflow));
    s.uset(STREAM_INPUT_BUFFER, char_vector(BUF_LEN));
    s.uset(STREAM_INPUT_BUFFER_LENGTH, LispObj::fixnum(BUF_LEN as i64));
}

fn add_output_buffer(s: LispObj, overflow: usize) {
    s.uset(STREAM_OVERFLOW_FUNC, qv(overflow));
    s.uset(STREAM_OUTPUT_BUFFER, char_vector(BUF_LEN));
    s.uset(STREAM_OUTPUT_BUFFER_LENGTH, LispObj::fixnum(BUF_LEN as i64));
}

/// Upstream `createConsoleStream`: bidirectional, stdin/stdout.
pub fn make_console_stream() -> LispObj {
    let s = base_stream(
        symbol_name(qv(TERMINAL_IO_Index)),
        CONSOLE_STREAM_Index,
        BIDIRECTIONAL_KEY_Index,
    );
    add_input_buffer(s, CONSOLE_UNDERFLOW_FUNCTION_Index);
    add_output_buffer(s, CONSOLE_OVERFLOW_FUNCTION_Index);
    s.uset(STREAM_HANDLE, LispObj::fixnum(add_handle(Source::Stdin) as i64));
    s.uset(STREAM_INTERACTIVE, t());
    s
}

/// After loading an image: forget the handles of the previous process and
/// give the console stream `s` a fresh stdin handle.
pub fn reattach_console(s: LispObj) {
    HANDLES.with(|h| h.borrow_mut().clear());
    if s.is_stream() {
        s.uset(STREAM_HANDLE, LispObj::fixnum(add_handle(Source::Stdin) as i64));
        s.uset(STREAM_INPUT_BUFFER_POS, LispObj::fixnum(0));
        s.uset(STREAM_INPUT_BUFFER_NUM, LispObj::fixnum(0));
        s.uset(STREAM_OUTPUT_BUFFER_POS, LispObj::fixnum(0));
    }
}

/// Upstream `inputFileStreamNode`.
pub fn make_file_input_stream(path: &str) -> LispObj {
    let f = std::fs::File::open(path)
        .unwrap_or_else(|e| lisp_error(format!("Could not open file {path} for reading: {e}")));
    let s = base_stream(heap::string_node(path), FILE_STREAM_Index, INPUT_KEY_Index);
    add_input_buffer(s, FILE_UNDERFLOW_FUNCTION_Index);
    let h = add_handle(Source::File(std::io::BufReader::new(f)));
    s.uset(STREAM_HANDLE, LispObj::fixnum(h as i64));
    s
}

/// String input stream, laid out as upstream's Lisp
/// MAKE-STRING-INPUT-STREAM (streams.lisp): the (simple) string itself is
/// the input buffer and the handle is `(string start end)`.
pub fn make_string_input_stream(str: LispObj) -> LispObj {
    let s = base_stream(nil(), STRING_STREAM_Index, INPUT_KEY_Index);
    let len = str.vector_length();
    s.uset(STREAM_UNDERFLOW_FUNC, qv(STRING_UNDERFLOW_FUNCTION_Index));
    s.uset(STREAM_HANDLE, heap::list(&[str, LispObj::fixnum(0), len]));
    s.uset(STREAM_INPUT_BUFFER, str);
    s.uset(STREAM_INPUT_BUFFER_LENGTH, len);
    s.uset(STREAM_INPUT_BUFFER_NUM, len);
    s
}

/// String output stream; the handle accumulates the output (upstream).
pub fn make_string_output_stream() -> LispObj {
    let s = base_stream(nil(), STRING_STREAM_Index, OUTPUT_KEY_Index);
    add_output_buffer(s, STRING_OVERFLOW_FUNCTION_Index);
    s.uset(STREAM_HANDLE, char_vector(0));
    s
}

/// Flush and return the accumulated string, resetting the stream.
pub fn get_output_stream_string(s: LispObj) -> LispObj {
    flush_stream(s);
    let r = s.uref(STREAM_HANDLE);
    s.uset(STREAM_HANDLE, char_vector(0));
    r
}

pub fn close_stream(s: LispObj) {
    let subclass = s.uref(STREAM_SUBCLASS);
    if subclass == qv(FILE_STREAM_Index) {
        let h = s.uref(STREAM_HANDLE).integer() as usize;
        HANDLES.with(|t| t.borrow_mut()[h] = None);
    }
    s.uset(STREAM_OPEN, nil());
}

fn check_stream(s: LispObj) {
    if !s.is_stream() {
        lisp_error("Not a stream");
    }
}

fn input_buffer_units(s: LispObj) -> *mut LispChar {
    s.uref(STREAM_INPUT_BUFFER).char_array_start()
}

/// Fill the input buffer of `s` from a Rust handle.
fn underflow_from_handle(s: LispObj) -> LispObj {
    check_stream(s);
    let h = s.uref(STREAM_HANDLE).integer() as usize;
    let cap = s.uref(STREAM_INPUT_BUFFER_LENGTH).integer() as usize;
    let buf = input_buffer_units(s);
    let n = HANDLES.with(|t| {
        let mut t = t.borrow_mut();
        let Some(handle) = t.get_mut(h).and_then(|x| x.as_mut()) else {
            lisp_error("Stream is closed");
        };
        if handle.pending.is_empty() && !handle.eof {
            let mut bytes = Vec::new();
            let r = match &mut handle.src {
                Source::Stdin => {
                    let _ = std::io::stdout().flush();
                    std::io::stdin().lock().read_until(b'\n', &mut bytes)
                }
                Source::File(f) => f.read_until(b'\n', &mut bytes),
            };
            match r {
                Ok(0) => handle.eof = true,
                Ok(_) => {
                    if bytes.ends_with(b"\r\n") {
                        bytes.truncate(bytes.len() - 2);
                        bytes.push(b'\n');
                    }
                    handle.pending.extend(decode_line(&bytes));
                }
                Err(e) => lisp_error(format!("Read error: {e}")),
            }
        }
        let n = handle.pending.len().min(cap);
        for i in 0..n {
            unsafe { *buf.add(i) = handle.pending.pop_front().unwrap() };
        }
        n
    });
    s.uset(STREAM_INPUT_BUFFER_POS, LispObj::fixnum(0));
    s.uset(STREAM_INPUT_BUFFER_NUM, LispObj::fixnum(n as i64));
    LispObj::fixnum(n as i64)
}

pub fn console_underflow(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 1);
    underflow_from_handle(args[0])
}

pub fn file_underflow(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 1);
    underflow_from_handle(args[0])
}

/// The whole string is the buffer: nothing more to read.
pub fn string_underflow(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 1);
    check_stream(args[0]);
    LispObj::fixnum(0)
}

fn output_units(s: LispObj) -> &'static [LispChar] {
    let n = s.uref(STREAM_OUTPUT_BUFFER_POS).integer() as usize;
    unsafe { std::slice::from_raw_parts(s.uref(STREAM_OUTPUT_BUFFER).char_array_start(), n) }
}

pub fn console_overflow(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 1);
    let s = args[0];
    let units = output_units(s);
    let n = units.len();
    let text = String::from_utf16_lossy(units);
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
    s.uset(STREAM_OUTPUT_BUFFER_POS, LispObj::fixnum(0));
    LispObj::fixnum(n as i64)
}

/// Upstream `stringOverflow`.
pub fn string_overflow(args: &[LispObj]) -> LispObj {
    check_args(args, 1, 1);
    let s = args[0];
    let old = s.uref(STREAM_HANDLE);
    let old_len = old.vector_length().integer() as usize;
    let add = output_units(s);
    let mut units = Vec::with_capacity(old_len + add.len());
    units.extend_from_slice(unsafe { std::slice::from_raw_parts(old.char_array_start(), old_len) });
    units.extend_from_slice(add);
    s.uset(STREAM_HANDLE, string_from_units(&units));
    s.uset(STREAM_OUTPUT_BUFFER_POS, LispObj::fixnum(0));
    LispObj::fixnum(add.len() as i64)
}

/// Upstream `getCharacter`: next char, or the EOF symbol.
pub fn get_character(s: LispObj) -> LispObj {
    if s.uref(STREAM_INPUT_BUFFER_POS) == s.uref(STREAM_INPUT_BUFFER_NUM) {
        funcall(s.uref(STREAM_UNDERFLOW_FUNC), &[s]);
    }
    let pos = s.uref(STREAM_INPUT_BUFFER_POS);
    if pos == s.uref(STREAM_INPUT_BUFFER_NUM) {
        return qv(CEof_Index);
    }
    let c = unsafe { *input_buffer_units(s).add(pos.integer() as usize) };
    s.uset(STREAM_INPUT_BUFFER_POS, LispObj::fixnum(pos.integer() + 1));
    bump(s, STREAM_POSITION, 1);
    if c == b'\n' as u16 {
        let ln = s.uref(STREAM_LINE_NUMBER).integer();
        s.uset(STREAM_LINE_NUMBER, LispObj::fixnum(ln + 1));
    }
    LispObj::character(c as u32)
}

/// Upstream `putbackCharacter`.
pub fn putback_character(s: LispObj, c: LispObj) {
    let pos = s.uref(STREAM_INPUT_BUFFER_POS).integer();
    if pos == 0 {
        lisp_error("Cannot put back character");
    }
    if c == LispObj::character('\n' as u32) {
        let ln = s.uref(STREAM_LINE_NUMBER).integer();
        s.uset(STREAM_LINE_NUMBER, LispObj::fixnum(ln - 1));
    }
    s.uset(STREAM_INPUT_BUFFER_POS, LispObj::fixnum(pos - 1));
    bump(s, STREAM_POSITION, -1);
}

fn bump(s: LispObj, i: usize, d: i64) {
    s.uset(i, LispObj::fixnum(s.uref(i).integer() + d));
}

/// Upstream `outputChar`.
pub fn output_char(s: LispObj, ch: LispChar) {
    if s.uref(STREAM_OUTPUT_BUFFER_POS) == s.uref(STREAM_OUTPUT_BUFFER_LENGTH) {
        funcall(s.uref(STREAM_OVERFLOW_FUNC), &[s]);
    }
    let pos = s.uref(STREAM_OUTPUT_BUFFER_POS).integer();
    unsafe { *s.uref(STREAM_OUTPUT_BUFFER).char_array_start().add(pos as usize) = ch };
    s.uset(STREAM_OUTPUT_BUFFER_POS, LispObj::fixnum(pos + 1));
    bump(s, STREAM_POSITION, 1);
    let col = if ch == b'\n' as u16 {
        0
    } else {
        s.uref(STREAM_COL_POSITION).integer() + 1
    };
    s.uset(STREAM_COL_POSITION, LispObj::fixnum(col));
}

pub fn output_units_to(s: LispObj, units: &[LispChar]) {
    for &u in units {
        output_char(s, u);
    }
}

pub fn output_str(s: LispObj, text: &str) {
    for u in text.encode_utf16() {
        output_char(s, u);
    }
}

/// Upstream `flushStream`.
pub fn flush_stream(s: LispObj) {
    let f = s.uref(STREAM_OVERFLOW_FUNC);
    if f != nil() {
        funcall(f, &[s]);
    }
}
