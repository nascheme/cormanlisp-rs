//! Kernel error signalling.
//!
//! Upstream `Error()` calls the function named ERROR. The kernel ERROR
//! primitive (and the fallback below) throws the message string to a
//! kernel error tag caught by the loader / REPL (`catch_lisp_error`). Once
//! format.lisp defines ERROR in Lisp, kernel errors print a message and
//! throw to `%ERROR` like any Lisp error. Like any throw, an error discards
//! intervening Rust frames without unwinding.

use crate::heap;
use crate::object::LispObj;
use crate::qv::{nil, qv};
use crate::qv_index::{ERROR_SIGNAL_Index, LISPERROR_Index, PORT_ERROR_TAG_Index};
use crate::runtime;
use crate::symbols::{symbol_function, symbol_function_type};

#[derive(Debug, Clone)]
pub struct LispError(pub String);

impl std::fmt::Display for LispError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn error_tag() -> LispObj {
    qv(PORT_ERROR_TAG_Index)
}

/// Signal an error with message `msg` (not a format control string).
pub fn lisp_error(msg: impl Into<String>) -> ! {
    let msg = msg.into();
    let err = qv(LISPERROR_Index);
    let f = symbol_function(err);
    let lisp_defined = symbol_function_type(err) == qv(crate::qv_index::FUNCTION_Index)
        && !f.is_kfunction();
    if lisp_defined && runtime::find_catcher(qv(ERROR_SIGNAL_Index)) != 0 {
        let s = heap::string_node(&msg.replace('~', "~~"));
        runtime::lisp_call(f, &[s]);
        // ERROR returned (it should not): fall through to the kernel tag.
    }
    kernel_error(msg)
}

/// Throw `msg` to the kernel error catcher.
pub fn kernel_error(msg: String) -> ! {
    let tag = error_tag();
    if runtime::find_catcher(tag) == 0 {
        eprintln!("Unhandled error: {msg}");
        std::process::exit(1);
    }
    let v = heap::cons(heap::string_node(&msg), nil());
    runtime::throw_to(tag, v)
}

/// Run `f`, converting a kernel error into `Err`. A throw to `%ERROR` (a
/// Lisp-level error, already reported by ERROR) gives `Err` with an empty
/// message.
pub fn catch_lisp_error<R>(f: impl FnOnce() -> R) -> Result<R, LispError> {
    let r = runtime::rust_catch(error_tag(), || runtime::rust_catch(qv(ERROR_SIGNAL_Index), f));
    match r {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(_)) => Err(LispError(String::new())),
        Err(_) => {
            let m = qv(crate::qv_index::RETURN_VALUES_Index);
            let s = if m.is_string() {
                m.string_to_rust()
            } else {
                crate::printer::write_to_string(m)
            };
            Err(LispError(s))
        }
    }
}

/// Kept for API compatibility; errors no longer use panics.
pub fn install_panic_hook() {}
