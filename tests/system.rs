//! M2 tests: the full load-sys.lisp system (lisp/boot.lisp), evaluated
//! with the Lisp READ, EVAL and WRITE.

use std::sync::Once;

use cormanlisp::error::catch_lisp_error;
use cormanlisp::prims::funcall;
use cormanlisp::qv::{nil, qv};
use cormanlisp::qv_index::*;
use cormanlisp::runtime::dyn_depth;
use cormanlisp::{compiler, heap, streams, with_lisp};

static BOOT: Once = Once::new();

fn boot() {
    BOOT.call_once(|| {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/lisp");
        std::env::set_current_dir(dir).unwrap();
        with_lisp(|| {
            let r = catch_lisp_error(|| compiler::load(heap::string_node("boot.lisp")));
            assert!(r.is_ok(), "boot.lisp failed: {r:?}");
        });
    });
}

/// Read and evaluate `src` with the Lisp reader; return the values of the
/// last form printed with PRIN1-style WRITE, space separated.
fn ev(src: &str) -> String {
    boot();
    with_lisp(|| {
        let r = catch_lisp_error(|| {
            let s = streams::make_string_input_stream(heap::string_node(src));
            let eof = qv(CEof_Index);
            let mut vals = heap::list(&[nil()]);
            loop {
                let x = funcall(qv(READ_Index), &[s, nil(), eof, nil()]);
                if x == eof {
                    break;
                }
                let v = funcall(qv(EVAL_Index), &[x]);
                vals = compiler::last_values(v);
            }
            let out = streams::make_string_output_stream();
            let kw = cormanlisp::symbols::find_keyword("STREAM");
            let mut first = true;
            while vals.is_cons() {
                if !first {
                    streams::output_char(out, b' ' as u16);
                }
                first = false;
                funcall(qv(WRITE_Index), &[vals.car(), kw, out]);
                vals = vals.cdr();
            }
            streams::get_output_stream_string(out).string_to_rust()
        });
        assert_eq!(dyn_depth(), 0, "dynamic frames leaked by {src:?}");
        match r {
            Ok(s) => s,
            Err(e) => format!("ERROR: {e}"),
        }
    })
}

#[test]
fn reader_and_printer() {
    assert_eq!(ev("'(a :b \"c\" 1.5 1.5d0 -7 1/2)"), "(A :B \"c\" 1.5 1.5d0 -7 1/2)");
    assert_eq!(ev("(symbol-package :foo)"), "#<PACKAGE \"KEYWORD\">");
    assert_eq!(ev("#(1 2 3)"), "#(1 2 3)");
    assert_eq!(ev("`(1 ,@(list 2 3) ,(+ 2 2))"), "(1 2 3 4)");
}

#[test]
fn format_works() {
    assert_eq!(ev("(format nil \"~a+~s=~d\" \"x\" \"y\" 42)"), "\"x+\\\"y\\\"=42\"");
    assert_eq!(ev("(format nil \"~{~a~^, ~}\" '(1 2 3))"), "\"1, 2, 3\"");
    assert_eq!(ev("(format nil \"~x ~b ~o\" 255 5 8)"), "\"FF 101 10\"");
}

#[test]
fn defstruct_and_setf() {
    assert_eq!(
        ev("(defstruct pt x (y 2)) (let ((p (make-pt :x 1))) (setf (pt-x p) 10) (incf (pt-y p)) (list (pt-x p) (pt-y p) (pt-p p)))"),
        "(10 3 T)"
    );
    assert_eq!(ev("(let ((l (list 1 2 3))) (setf (cadr l) 'b) (push 0 l) l)"), "(0 1 B 3)");
}

#[test]
fn hash_tables() {
    assert_eq!(
        ev("(let ((h (make-hash-table :test 'equal))) (setf (gethash \"a\" h) 1 (gethash '(1 2) h) 2) (list (gethash \"a\" h) (gethash (list 1 2) h) (hash-table-count h)))"),
        "(1 2 2)"
    );
    assert_eq!(
        ev("(let ((h (make-hash-table))) (dotimes (i 1000) (setf (gethash i h) (* i i))) (list (gethash 999 h) (hash-table-count h)))"),
        "(998001 1000)"
    );
}

#[test]
fn packages() {
    assert_eq!(
        ev("(progn (make-package \"P1\" :use nil) (intern \"S1\" \"P1\") (export (find-symbol \"S1\" \"P1\") \"P1\") (multiple-value-list (find-symbol \"S1\" \"P1\")))"),
        "(P1:S1 :EXTERNAL)"
    );
}

#[test]
fn macrolet_and_symbol_macrolet() {
    assert_eq!(ev("(macrolet ((twice (x) `(* 2 ,x))) (twice 21))"), "42");
    assert_eq!(ev("(flet ((foo () 1)) (macrolet ((foo () 2)) (foo)))"), "2");
    assert_eq!(ev("(macrolet ((foo () 2)) (flet ((foo () 1)) (foo)))"), "1");
    assert_eq!(
        ev("(let ((x (list 1 2))) (symbol-macrolet ((a (car x))) (setf a 10) (incf a) x))"),
        "(11 2)"
    );
    assert_eq!(
        ev("(defmacro my-exp (form &environment env) `',(macroexpand form env)) (macrolet ((m () 'expanded)) (my-exp (m)))"),
        "EXPANDED"
    );
}

#[test]
fn errors_are_reported() {
    assert_eq!(ev("(car 5)"), "ERROR: ");
    // Upstream conditions.lisp throws the condition to %ERROR.
    assert_eq!(ev("(list (typep (catch 'common-lisp::%error (car 5)) 'error) 1)"), "(T 1)");
    assert_eq!(ev("(handler-case (car 5) (error (c) (princ-to-string c)))"), "\"Not a list: 5\"");
}

#[test]
fn declarations() {
    assert_eq!(ev("(funcall (lambda (x) (+ x 1)) 2)"), "3");
    assert_eq!(ev("(mapcar (lambda (x) (* x x)) '(1 2 3))"), "(1 4 9)");
    assert_eq!(ev("(intersection '(1 2 3) '(2 3 4))"), "(2 3)");
    assert_eq!(ev("(declaim (special *decl-test*)) (setq *decl-test* 5) (let ((*decl-test* 6)) (symbol-value '*decl-test*))"), "6");
}

// M4: load-sys2 subset.

#[test]
fn lists_and_trees() {
    assert_eq!(ev("(subst 'x 'b '(a b (b c)))"), "(A X (X C))");
    assert_eq!(ev("(union '(1 2) '(2 3))"), "(1 2 3)");
    assert_eq!(ev("(set-difference '(1 2 3) '(2))"), "(1 3)");
    assert_eq!(ev("(last '(1 2 3) 2)"), "(2 3)");
    assert_eq!(ev("(copy-list '(1 2 . 3))"), "(1 2 . 3)");
    assert_eq!(ev("(let ((s (gensym))) (setf (get s 'a) 1 (get s 'b) 2) (remprop s 'a) (symbol-plist s))"), "(B 2)");
}

#[test]
fn sequences_strings_characters() {
    assert_eq!(ev("(sort (list 3 1 2) #'<)"), "(1 2 3)");
    assert_eq!(ev("(nreverse (list 1 2 3))"), "(3 2 1)");
    assert_eq!(ev("(remove-duplicates '(1 2 1 3))"), "(2 1 3)");
    assert_eq!(ev("(search \"lo\" \"hello\")"), "3");
    assert_eq!(ev("(string-trim \" \" \"  hi  \")"), "\"hi\"");
    assert_eq!(ev("(string-upcase \"hello\")"), "\"HELLO\"");
    assert_eq!(ev("(digit-char-p #\\7)"), "7");
    assert_eq!(ev("(equalp #(1 #\\a) (vector 1 #\\A))"), "T");
}

#[test]
fn math() {
    assert_eq!(ev("(list (gcd 12 18) (lcm 4 6) (isqrt 17) (integer-length 255))"), "(6 12 4 8)");
    assert_eq!(ev("(list (ldb (byte 4 4) #xff) (logcount 255) (logbitp 3 8))"), "(15 8 T)");
    assert_eq!(ev("(integer-decode-float 1.5d0)"), "6755399441055744 -52 1");
    assert_eq!(ev("(decode-float -3.0d0)"), "0.75d0 2 -1.0d0");
    assert_eq!(ev("(scale-float 1.0d0 3)"), "8.0d0");
    assert_eq!(ev("(rational 0.5d0)"), "1/2");
    assert_eq!(ev("(< most-positive-double-float (* 2 most-positive-double-float))"), "T");
    assert_eq!(ev("(< (random 10) 10)"), "T");
    assert_eq!(ev("(let ((r (random 2d0))) (and (typep r 'double-float) (<= 0 r) (< r 2)))"), "T");
}

#[test]
fn types_and_packages() {
    assert_eq!(ev("(list (type-of 3) (type-of 'a) (subtypep 'fixnum 'integer))"), "(FIXNUM SYMBOL T)");
    assert_eq!(ev("(coerce '(1 2) 'vector)"), "#(1 2)");
    assert_eq!(ev("(defpackage \"M4-PKG\" (:use \"COMMON-LISP\") (:export \"BAR\")) (find-symbol \"BAR\" \"M4-PKG\")"), "M4-PKG:BAR :EXTERNAL");
}

#[test]
fn conditions() {
    assert_eq!(ev("(handler-case (error \"boom ~a\" 1) (error (c) (format nil \"~a\" c)))"), "\"boom 1\"");
    assert_eq!(ev("(handler-case (car 5) (error () 'caught))"), "CAUGHT");
    assert_eq!(ev("(handler-case (no-such-function-m4) (undefined-function (c) (cell-error-name c)))"), "NO-SUCH-FUNCTION-M4");
    assert_eq!(ev("(handler-case 42 (:no-error (x) (list 'ok x)))"), "(OK 42)");
    assert_eq!(ev("(car (multiple-value-list (ignore-errors (error \"x\"))))"), "NIL");
    assert_eq!(
        ev("(let (log) (handler-bind ((warning (lambda (c) (push (format nil \"~a\" c) log) (muffle-warning c)))) (warn \"w ~a\" 1)) log)"),
        "(\"w 1\")"
    );
    assert_eq!(
        ev("(define-condition m4-err (error) ((code :initarg :code :reader m4-code))) (handler-case (error 'm4-err :code 7) (m4-err (c) (m4-code c)))"),
        "7"
    );
    assert_eq!(ev("(block b (handler-bind ((error (lambda (c) (return-from b (type-of c))))) (error 'type-error :datum 1 :expected-type 'string)))"), "TYPE-ERROR");
    assert_eq!(ev("(error \"unhandled\")"), "ERROR: ");
}

#[test]
fn loop_macro() {
    assert_eq!(ev("(loop for i from 1 to 5 collect (* i i))"), "(1 4 9 16 25)");
    assert_eq!(
        ev("(loop for i below 10 when (evenp i) collect i into e else collect i into o finally (return (list e o)))"),
        "((0 2 4 6 8) (1 3 5 7 9))"
    );
    assert_eq!(
        ev("(loop with (a b) = '(1 2) and (c d) = '(3 4) return (list a b c d))"),
        "(1 2 3 4)"
    );
    assert_eq!(ev("(loop for i from 1 when (> i 3) do (loop-finish) collect i)"), "(1 2 3)");
    assert_eq!(ev("(with-output-to-string (s) (fresh-line s) (princ 1 s) (fresh-line s))"), "\"1\n\"");
}

#[test]
fn string_streams() {
    assert_eq!(ev("(multiple-value-list (read-from-string \"(a b) c\"))"), "((A B) 5)");
    assert_eq!(ev("(read-from-string \"\" nil :eof)"), ":EOF 0");
    assert_eq!(
        ev("(with-input-from-string (s \"hello world\") (list (read s) (read-char s) (read-line s)))"),
        "(HELLO #\\w \"orld\")"
    );
    assert_eq!(ev("(let ((i 0)) (with-input-from-string (s \"abc def\" :index i) (read s)) i)"), "4");
    assert_eq!(ev("(prin1-to-string \"a\")"), "\"\\\"a\\\"\"");
    assert_eq!(ev("(read-from-string \"#+win32 win:x 5\")"), "5 15");
}

#[test]
fn progv() {
    assert_eq!(
        ev("(defvar *pv* 1) (defun pv () *pv*) (list (progv '(*pv*) '(2) (pv)) *pv*)"),
        "(2 1)"
    );
    assert_eq!(ev("(list (catch 'tag (progv '(*pv*) '(7) (throw 'tag (pv)))) *pv*)"), "(7 1)");
    assert_eq!(ev("(progv '(*pv*) '() (boundp '*pv*))"), "NIL");
    assert_eq!(ev("(multiple-value-list (progv '(*pv*) '(5) (values *pv* 2)))"), "(5 2)");
}

#[test]
fn subtypep_structures() {
    assert_eq!(
        ev("(defstruct st-a x) (defstruct (st-b (:include st-a)) y) (list (subtypep 'st-b 'st-a) (subtypep 'st-a 'st-b) (subtypep 'st-a 'structure-object))"),
        "(T NIL T)"
    );
    assert_eq!(ev("(subtypep 'simple-error 'error)"), "T T");
}
