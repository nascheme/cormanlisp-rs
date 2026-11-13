//! CormanLisp kernel in Rust (AMD64 Linux). See `port-plan.md`.

pub mod compiler;
pub mod error;
pub mod gc;
pub mod heap;
pub mod image;
pub mod numbers;
pub mod object;
pub mod prims;
pub mod printer;
pub mod qv;
pub mod qv_index;
pub mod reader;
pub mod runtime;
pub mod streams;
pub mod symbols;

use std::sync::{Mutex, MutexGuard, Once};

use object::LispObj;
use qv_index::*;

static INIT: Once = Once::new();
static LISP_LOCK: Mutex<()> = Mutex::new(());

/// Port of upstream `initLisp`: map the heap, create packages, symbols,
/// kernel functions, the readtable and the console stream. Idempotent.
pub fn init_lisp() {
    INIT.call_once(|| {
        let base = heap::map_region();
        qv::set_qv_base(base);
        heap::init_alloc();
        // The world is not consistent until initialization is done.
        heap::inhibit_gc(1);
        symbols::init_packages();
        let tag = symbols::symbol_node(heap::string_node("KERNEL-ERROR"));
        qv::set_qv(PORT_ERROR_TAG_Index, tag);
        qv::set_qv(PORT_STD_GF_SYM_Index, symbols::find_symbol("THE-CLASS-STANDARD-GF"));
        runtime::init_runtime();
        prims::init_kernel_functions();
        let rt = reader::readtable_node();
        symbols::set_symbol_value(qv::qv(READTABLE_Index), rt);
        symbols::set_symbol_value(qv::qv(COMMON_LISP_READTABLE_Index), rt);
        let console = streams::make_console_stream();
        for idx in [
            CONSOLE_INPUT_STREAM_Index,
            CONSOLE_OUTPUT_STREAM_Index,
        ] {
            symbols::set_symbol_value(qv::qv(idx), console);
        }
        for idx in [
            STANDARD_INPUT_Index,
            STANDARD_OUTPUT_Index,
            TERMINAL_IO_Index,
            ERROR_OUTPUT_Index,
        ] {
            let sym = qv::qv(idx);
            symbols::set_symbol_value(sym, console);
            symbols::set_special_symbol(sym);
        }
        heap::inhibit_gc(-1);
    });
}

/// Initialize the Lisp world from an image file instead of building it.
/// Must be called before anything else touches the heap.
pub fn init_lisp_from_image(path: &str) -> Result<(), String> {
    let mut r = Err("Lisp already initialized".to_string());
    INIT.call_once(|| {
        let base = heap::map_region();
        qv::set_qv_base(base);
        r = image::load_image_into_heap(path);
        if r.is_ok() {
            image::restore_after_load();
        }
    });
    r
}

/// Run `f` with exclusive access to the (single-threaded) Lisp world,
/// initializing it first if needed. Used by tests and embedders.
pub fn with_lisp<R>(f: impl FnOnce() -> R) -> R {
    let _guard: MutexGuard<'_, ()> = LISP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    init_lisp();
    f()
}

/// Read all forms from `text` (kernel reader).
pub fn read_all_from_str(text: &str) -> Vec<LispObj> {
    let s = streams::make_string_input_stream(heap::string_node(text));
    let mut out = qv::nil();
    while let Some(x) = reader::read(s) {
        out = heap::cons(x, out);
    }
    let mut v = heap::list_to_vec(out);
    v.reverse();
    v
}
