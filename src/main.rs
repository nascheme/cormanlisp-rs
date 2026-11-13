//! Command line entry point.
//!
//! `cormanlisp [--image IMG] [--disasm] [--boot FILE] [-e FORMS]... [--repl]`
//! starts from heap image IMG (default: none, an empty kernel world), loads
//! FILE, evaluates the FORMS, and starts the REPL if neither was given (or
//! with `--repl`). `--save IMG` after the other options writes an image. Reading, evaluation and printing go through the functions
//! currently named READ, EVAL and WRITE, so once the Lisp system is loaded
//! they are the Lisp versions. The REPL is the function in `*TOP-LEVEL*`
//! if there is one (upstream `lispmain`), else a kernel REPL.

use cormanlisp::compiler::{self, codegen::DISASSEMBLE};
use cormanlisp::error::catch_lisp_error;
use cormanlisp::object::{LispObj, STREAM_INPUT_BUFFER_NUM, STREAM_INPUT_BUFFER_POS};
use cormanlisp::prims::funcall;
use cormanlisp::qv::{nil, qv};
use cormanlisp::qv_index::*;
use cormanlisp::symbols::{find_symbol, symbol_value};
use cormanlisp::{heap, image, init_lisp, init_lisp_from_image, streams};

fn usage() -> ! {
    eprintln!(
        "usage: cormanlisp [--image IMG] [--disasm] [--boot FILE] [-e FORMS]... [--repl] [--save IMG]"
    );
    std::process::exit(2);
}

fn report(e: cormanlisp::error::LispError) {
    flush_stdout();
    if !e.0.is_empty() {
        eprintln!("Error: {e}");
    }
}

/// Read one form with the current READ function; `None` at end of file.
fn lisp_read(stream: LispObj) -> Option<LispObj> {
    let eof = qv(CEof_Index);
    let x = funcall(qv(READ_Index), &[stream, nil(), eof, nil()]);
    (x != eof).then_some(x)
}

/// Values of evaluating `x`, as a list.
fn lisp_eval(x: LispObj) -> LispObj {
    let r = funcall(qv(EVAL_Index), &[x]);
    compiler::last_values(r)
}

fn print_values(mut vals: LispObj) {
    while vals.is_cons() {
        funcall(qv(WRITE_Index), &[vals.car()]);
        funcall(find_symbol("TERPRI"), &[]);
        vals = vals.cdr();
    }
    funcall(find_symbol("FORCE-OUTPUT"), &[]);
}

/// Evaluate every form in `text`, printing the values of the last one.
fn eval_string(text: &str) -> bool {
    let r = catch_lisp_error(|| {
        let s = streams::make_string_input_stream(heap::string_node(text));
        let mut vals = heap::list(&[nil()]);
        while let Some(x) = lisp_read(s) {
            vals = lisp_eval(x);
        }
        print_values(vals);
    });
    match r {
        Ok(()) => true,
        Err(e) => {
            report(e);
            false
        }
    }
}

/// Load a boot file with its directory as the current directory (as
/// upstream's compile-sys.lisp changes to the Corman Lisp directory), so
/// it can refer to `sys/...` files. The previous directory is restored.
fn load_file(path: &str) -> bool {
    let p = std::path::Path::new(path);
    let saved = std::env::current_dir().ok();
    let dir = p.parent().filter(|d| !d.as_os_str().is_empty());
    let name = p.file_name().map_or(path.to_string(), |n| n.to_string_lossy().into_owned());
    if let Some(d) = dir
        && let Err(e) = std::env::set_current_dir(d)
    {
        eprintln!("Cannot change directory to {}: {e}", d.display());
        return false;
    }
    let r = catch_lisp_error(|| compiler::load(heap::string_node(&name)));
    if let (Some(_), Some(s)) = (dir, saved) {
        let _ = std::env::set_current_dir(s);
    }
    flush_stdout();
    if let Err(e) = r {
        if e.0.is_empty() {
            eprintln!("Error while loading {path}");
        } else {
            eprintln!("Error while loading {path}: {e}");
        }
        return false;
    }
    true
}

fn flush_stdout() {
    let out = symbol_value(qv(STANDARD_OUTPUT_Index));
    if out.is_stream() {
        streams::flush_stream(out);
    }
}

/// Kernel REPL, used when no `*TOP-LEVEL*` function is defined.
fn kernel_repl() {
    let input = symbol_value(qv(STANDARD_INPUT_Index));
    let out = symbol_value(qv(STANDARD_OUTPUT_Index));
    loop {
        streams::output_str(out, "? ");
        streams::flush_stream(out);
        let r = catch_lisp_error(|| match lisp_read(input) {
            None => false,
            Some(x) => {
                print_values(lisp_eval(x));
                true
            }
        });
        match r {
            Ok(true) => {}
            Ok(false) => {
                streams::output_char(out, b'\n' as u16);
                streams::flush_stream(out);
                break;
            }
            Err(e) => {
                report(e);
                // Discard the rest of the offending input line.
                let pos = input.uref(STREAM_INPUT_BUFFER_NUM);
                input.uset(STREAM_INPUT_BUFFER_POS, pos);
            }
        }
    }
}

fn repl() {
    let top = symbol_value(qv(TOP_LEVEL_Index));
    if top.is_function() {
        // The Lisp top level catches errors itself; an error escaping it
        // ends the session.
        if let Err(e) = catch_lisp_error(|| funcall(top, &[])) {
            report(e);
        }
        flush_stdout();
    } else {
        kernel_repl();
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--image") {
        if args.len() < 2 {
            usage();
        }
        let path = args[1].clone();
        args.drain(..2);
        if let Err(e) = init_lisp_from_image(&path) {
            eprintln!("Cannot load image: {e}");
            std::process::exit(1);
        }
        if let Err(e) = catch_lisp_error(image::run_restore_funcs) {
            report(e);
        }
    } else {
        init_lisp();
    }
    let mut ok = true;
    let mut did_something = false;
    let mut want_repl = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-e" => {
                let Some(text) = it.next() else { usage() };
                ok &= eval_string(text);
                did_something = true;
            }
            "--boot" => {
                let Some(path) = it.next() else { usage() };
                ok &= load_file(path);
                did_something = true;
            }
            "--disasm" => DISASSEMBLE.store(true, std::sync::atomic::Ordering::Relaxed),
            "--repl" => want_repl = true,
            "--save" => {
                let Some(path) = it.next() else { usage() };
                let r = catch_lisp_error(|| image::save_image(path));
                if let Err(e) = r {
                    report(e);
                    ok = false;
                }
                did_something = true;
            }
            _ => usage(),
        }
    }
    if !did_something || want_repl {
        repl();
    }
    flush_stdout();
    std::process::exit(if ok { 0 } else { 1 });
}
