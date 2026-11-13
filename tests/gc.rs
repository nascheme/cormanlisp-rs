//! M3 tests: the garbage collector. Boots load-sys.lisp with a small
//! allocation threshold (many collections) and heap verification on.

use std::sync::Once;

use cormanlisp::error::catch_lisp_error;
use cormanlisp::prims::funcall;
use cormanlisp::qv::{nil, qv};
use cormanlisp::qv_index::*;
use cormanlisp::{compiler, gc, heap, streams, with_lisp};

static BOOT: Once = Once::new();

fn boot() {
    BOOT.call_once(|| {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/lisp");
        std::env::set_current_dir(dir).unwrap();
        with_lisp(|| {
            heap::set_gc_params(256 << 10, true);
            let r = catch_lisp_error(|| compiler::load(heap::string_node("boot.lisp")));
            assert!(r.is_ok(), "boot.lisp failed: {r:?}");
        });
    });
}

/// Evaluate `src` with the Lisp reader and EVAL; the last value printed.
fn ev(src: &str) -> String {
    boot();
    with_lisp(|| {
        let r = catch_lisp_error(|| {
            let s = streams::make_string_input_stream(heap::string_node(src));
            let eof = qv(CEof_Index);
            let mut v = nil();
            loop {
                let x = funcall(qv(READ_Index), &[s, nil(), eof, nil()]);
                if x == eof {
                    break;
                }
                v = funcall(qv(EVAL_Index), &[x]);
            }
            let out = streams::make_string_output_stream();
            let kw = cormanlisp::symbols::find_keyword("STREAM");
            funcall(qv(WRITE_Index), &[v, kw, out]);
            streams::get_output_stream_string(out).string_to_rust()
        });
        match r {
            Ok(s) => s,
            Err(e) => format!("ERROR: {e}"),
        }
    })
}

#[test]
fn boot_with_many_collections() {
    boot();
    let n = with_lisp(gc::gc_count);
    assert!(n > 20, "only {n} collections during boot");
    with_lisp(gc::verify_heap);
}

#[test]
fn collection_frees_garbage() {
    ev("(gc)");
    let before = with_lisp(heap::bytes_used);
    ev("(dotimes (i 20000) (make-list 10))");
    ev("(gc)");
    let after = with_lisp(heap::bytes_used);
    assert!(after < before + (256 << 10), "before {before}, after {after}");
}

#[test]
fn data_survives_collections() {
    assert_eq!(
        ev("(defvar *l* (let (r) (dotimes (i 5000) (push (format nil \"~d\" i) r)) r))
            (gc)
            (list (length *l*) (car *l*) (car (last *l*)))"),
        "(5000 \"4999\" \"0\")"
    );
    assert_eq!(
        ev("(let ((f (let ((n 0)) #'(lambda () (incf n)))))
              (dotimes (i 1000) (funcall f) (make-list 100))
              (funcall f))"),
        "1001"
    );
    assert_eq!(
        ev("(length (apply #'append (make-list 300 :initial-element (make-list 50))))"),
        "15000"
    );
    assert_eq!(
        ev("(length (apply #'list (make-list 20000 :initial-element 1)))"),
        "20000"
    );
}

#[test]
fn eq_hash_tables_rehash() {
    assert_eq!(
        ev("(defvar *h* (make-hash-table :test #'eq))
            (defvar *keys* nil)
            (dotimes (i 2000) (let ((k (list i))) (push k *keys*) (setf (gethash k *h*) i)))
            (gc)
            (let ((n 0))
              (dolist (k *keys*) (when (eql (gethash k *h*) (car k)) (incf n)))
              n)"),
        "2000"
    );
}

#[test]
fn errors_during_compilation_reenable_gc() {
    // A macro expander signals an error while the compiler runs (GC
    // inhibited); the throw out of the compiler must re-enable GC.
    ev("(defmacro bad () (error \"bad macro\"))");
    assert!(ev("(defun uses-bad () (bad))").starts_with("ERROR"));
    let n = with_lisp(gc::gc_count);
    ev("(gc)");
    assert!(with_lisp(gc::gc_count) > n);
}
