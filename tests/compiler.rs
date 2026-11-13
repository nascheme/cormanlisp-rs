//! M1 tests: kernel compiler, runtime and bootstrap.lisp.

use std::sync::Once;

use cormanlisp::compiler::eval_string;
use cormanlisp::error::catch_lisp_error;
use cormanlisp::printer::write_to_string;
use cormanlisp::runtime::dyn_depth;
use cormanlisp::with_lisp;

/// Evaluate `src`; print the values of the last form, space separated.
fn ev(src: &str) -> String {
    with_lisp(|| {
        let r = catch_lisp_error(|| {
            let vals = eval_string(src);
            vals.iter().map(|&v| write_to_string(v)).collect::<Vec<_>>().join(" ")
        });
        assert_eq!(dyn_depth(), 0, "dynamic frames leaked by {src:?}");
        match r {
            Ok(s) => s,
            Err(e) => format!("ERROR: {e}"),
        }
    })
}

static BOOT: Once = Once::new();

/// Like `ev`, after loading bootstrap.lisp once.
fn evb(src: &str) -> String {
    BOOT.call_once(|| {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/lisp/sys/bootstrap.lisp");
        let r = ev(&format!("(load \"{path}\")"));
        assert_eq!(r, "T", "loading bootstrap.lisp");
    });
    ev(src)
}

#[test]
fn constants_and_calls() {
    assert_eq!(ev("42"), "42");
    assert_eq!(ev("'(1 2)"), "(1 2)");
    assert_eq!(ev("(car '(1 2))"), "1");
    assert_eq!(ev("(cdr '(1 2))"), "(2)");
    assert_eq!(ev("(car nil)"), "NIL");
    assert_eq!(ev("(cons 1 2)"), "(1 . 2)");
    assert_eq!(ev("(list 1 (+ 1 1) (* 3 1))"), "(1 2 3)");
    assert_eq!(ev("(if (eq 'a 'a) 'yes 'no)"), "YES");
    assert_eq!(ev("(if nil 1)"), "NIL");
    assert_eq!(ev("(null nil)"), "T");
    assert_eq!(ev("(+ 1.5 2)"), "3.500000"); // kernel printer format
    assert!(ev("(car 5)").starts_with("ERROR: Not a list"));
}

#[test]
fn let_and_setq() {
    assert_eq!(ev("(let ((x 1) (y 2)) (list x y))"), "(1 2)");
    assert_eq!(ev("(let ((x 1)) (let ((x 2) (y x)) (list x y)))"), "(2 1)");
    assert_eq!(ev("(let ((x 1)) (let* ((x 2) (y x)) (list x y)))"), "(2 2)");
    assert_eq!(ev("(let (a b) (setq a 1 b (+ a 1)) (list a b))"), "(1 2)");
}

#[test]
fn lambda_lists() {
    assert_eq!(
        ev("(funcall #'(lambda (a &optional (b 10) (c 3 cp)) (list a b c cp)) 1 2)"),
        "(1 2 3 NIL)"
    );
    assert_eq!(
        ev("(funcall #'(lambda (a &optional (b a)) (list a b)) 7)"),
        "(7 7)"
    );
    assert_eq!(ev("(funcall #'(lambda (&rest r) r) 1 2 3)"), "(1 2 3)");
    assert_eq!(ev("(funcall #'(lambda (a &rest r) r) 1)"), "NIL");
    assert_eq!(
        ev("(funcall #'(lambda (&key (a 1) (b 2 bp)) (list a b bp)) (find-keyword \"B\") 5)"),
        "(1 5 T)"
    );
    assert!(ev("(funcall #'(lambda (&key a) a) (find-keyword \"Z\") 1)")
        .starts_with("ERROR: Invalid keyword"));
    assert_eq!(
        ev("(funcall #'(lambda (&key a &allow-other-keys) a) (find-keyword \"Z\") 1)"),
        "NIL"
    );
    assert_eq!(ev("(funcall #'(lambda (a &aux (b (+ a 1))) b) 1)"), "2");
    assert!(ev("(funcall #'(lambda (a) a))").starts_with("ERROR: Wrong number of arguments"));
    assert!(ev("(funcall #'(lambda (a &optional b) a) 1 2 3)")
        .starts_with("ERROR: Wrong number of arguments"));
}

#[test]
fn closures() {
    assert_eq!(
        ev("(let ((x 1)) (funcall #'(lambda () (setq x (+ x 1)))) x)"),
        "2"
    );
    assert_eq!(
        ev("(funcall (funcall (funcall #'(lambda (x) #'(lambda (y) #'(lambda (z) (list x y z)))) 1) 2) 3)"),
        "(1 2 3)"
    );
    assert_eq!(
        ev("(let ((fs nil)) (let ((n 0)) (setq fs (list #'(lambda () (setq n (+ n 1))) #'(lambda () n)))) (funcall (car fs)) (funcall (car fs)) (funcall (car (cdr fs))))"),
        "2"
    );
    assert_eq!(
        ev("(labels ((ev (n) (if (= n 0) t (od (- n 1)))) (od (n) (if (= n 0) nil (ev (- n 1))))) (list (ev 10) (od 7)))"),
        "(T T)"
    );
    assert_eq!(ev("(flet ((g (x) (* x 2))) (g 21))"), "42");
    assert_eq!(ev("(flet ((car (x) x)) (car 5))"), "5");
}

#[test]
fn blocks_and_tagbody() {
    assert_eq!(ev("(block b 1 (return-from b 2) 3)"), "2");
    assert_eq!(ev("(block b (funcall #'(lambda () (return-from b 7))) 6)"), "7");
    assert_eq!(
        ev("(let ((n 0)) (tagbody a (setq n (+ n 1)) (if (< n 5) (go a))) n)"),
        "5"
    );
    assert_eq!(
        ev("(let ((i 0)) (tagbody top (funcall #'(lambda () (setq i (+ i 1)) (if (< i 3) (go top))))) i)"),
        "3"
    );
    assert_eq!(ev("(block b (block c (return-from b 1)) 2)"), "1");
    // a closure escaping its block cannot return to it
    assert!(ev("(funcall (block b #'(lambda () (return-from b 1))))")
        .starts_with("ERROR: Throw to tag"));
}

#[test]
fn catch_throw_unwind_protect() {
    assert_eq!(ev("(catch 'x (throw 'x 9) 1)"), "9");
    assert_eq!(ev("(catch 'x (catch 'y (throw 'x 1)) 2)"), "1");
    assert_eq!(ev("(catch 'x (throw 'x (values 1 2)))"), "1 2");
    assert_eq!(
        ev("(let ((r nil)) (catch 'x (unwind-protect (throw 'x 1) (setq r 42))) r)"),
        "42"
    );
    assert_eq!(
        ev("(let ((r nil)) (list (catch 'x (unwind-protect (unwind-protect (throw 'x 1) (setq r (cons 1 r))) (setq r (cons 2 r)))) r))"),
        "(1 (2 1))"
    );
    assert_eq!(
        ev("(let ((r nil)) (list (block b (unwind-protect (return-from b 1) (setq r 'c))) r))"),
        "(1 C)"
    );
    assert_eq!(ev("(unwind-protect (values 1 2) 3)"), "1 2");
    assert!(ev("(throw 'nope 1)").starts_with("ERROR: Throw to tag"));
}

#[test]
fn multiple_values() {
    assert_eq!(ev("(values 1 2 3)"), "1 2 3");
    assert_eq!(ev("(values)"), "");
    assert_eq!(ev("(multiple-value-call #'list (values 1 2) (values) 3)"), "(1 2 3)");
    assert_eq!(ev("(multiple-value-prog1 (values 1 2) 3)"), "1 2");
    assert_eq!(ev("(let ((x (values 1 2))) x)"), "1");
    assert_eq!(ev("(funcall #'(lambda () (values 4 5)))"), "4 5");
    assert_eq!(ev("(block b (return-from b (values 6 7)))"), "6 7");
}

#[test]
fn specials() {
    assert_eq!(
        // flags fixnum 2 = special (upstream wrapInteger(2))
        ev("(progn (setq *t-sp* 1) (%symbol-set-flags 2 '*t-sp*) 0)"),
        "0"
    );
    assert_eq!(
        ev("(progn (set-symbol-function #'(lambda () *t-sp*) 'get-t-sp) (list (let ((*t-sp* 2)) (get-t-sp)) (get-t-sp)))"),
        "(2 1)"
    );
    assert_eq!(ev("(progn (catch 'k (let ((*t-sp* 5)) (throw 'k nil))) *t-sp*)"), "1");
    assert_eq!(
        ev("(funcall #'(lambda (*t-sp*) (get-t-sp)) 9)"),
        "9"
    );
    assert_eq!(ev("*t-sp*"), "1");
    assert_eq!(
        ev("(let ((x 1)) (declare (special x)) (funcall #'(lambda () (declare (special x)) x)))"),
        "1"
    );
    assert!(ev("*unbound-test-var*").starts_with("ERROR: The variable"));
}

#[test]
fn bootstrap() {
    assert_eq!(
        evb("(defun fact (n) (if (< n 2) 1 (* n (fact (- n 1))))) (fact 15)"),
        "1307674368000"
    );
    assert_eq!(evb("(cond ((= 1 2) 'a) (t 'b))"), "B");
    assert_eq!(evb("(let ((r nil)) (dolist (x '(1 2 3) r) (push x r)))"), "(3 2 1)");
    assert_eq!(evb("(let ((r 0)) (dotimes (i 4 r) (setq r (+ r i))))"), "6");
    assert_eq!(
        evb("(do ((i 0 (1+ i)) (acc nil (cons i acc))) ((= i 4) acc))"),
        "(3 2 1 0)"
    );
    assert_eq!(evb("(list (and 1 2) (or nil 3) (when nil 1) (unless nil 2))"), "(2 3 NIL 2)");
    assert_eq!(evb("(multiple-value-list (truncate 7 2))"), "(3 1)");
    assert_eq!(evb("(list-length '(1 2 3 4 5))"), "5");
    assert_eq!(evb("(macroexpand '(push 1 x))"), "(SETQ X (CONS 1 X))");
    assert_eq!(evb("(let ((a 1) (b 2)) (psetq a b b a) (list a b))"), "(2 1)");
    assert_eq!(evb("(nth 2 '(a b c d))"), "C");
    assert_eq!(evb("(defmacro my-inc (v &optional (n 1)) `(setq ,v (+ ,v ,n))) (let ((z 1)) (my-inc z 5) z)"), "6");
}
