//! M0 tests: heap layout, symbols/packages, reader/printer round trips.

use cormanlisp::error::catch_lisp_error;
use cormanlisp::heap;
use cormanlisp::object::*;
use cormanlisp::printer::write_to_string;
use cormanlisp::qv::{nil, qv, t};
use cormanlisp::qv_index::*;
use cormanlisp::symbols::*;
use cormanlisp::{read_all_from_str, with_lisp};

fn read1(s: &str) -> LispObj {
    let v = read_all_from_str(s);
    assert_eq!(v.len(), 1, "expected one form in {s:?}");
    v[0]
}

fn rp(s: &str) -> String {
    write_to_string(read1(s))
}

#[test]
fn cons_layout() {
    with_lisp(|| {
        let c = heap::cons(LispObj::fixnum(1), LispObj::fixnum(2));
        assert!(c.is_cons());
        assert_eq!(c.raw() & 15, CONS_TAG);
        assert_eq!(c.car().integer(), 1);
        assert_eq!(c.cdr().integer(), 2);
        c.set_cdr(nil());
        assert_eq!(c.cdr(), nil());
    });
}

#[test]
fn uvector_layout() {
    with_lisp(|| {
        // SYMBOL_SIZE = 9 cells + header = 10 cells = 5 units.
        let s = find_symbol("LAYOUT-TEST");
        assert_eq!(s.raw() & 15, UVECTOR_TAG);
        assert_eq!(s.uvector_type(), SYMBOL_TYPE);
        assert_eq!(s.uvector_size(), 5);
        assert!(s.uref(0).is_uvector_header());
        let v = heap::vector_node(3);
        assert_eq!(v.uvector_size(), 3); // 1 header + 1 length + 3 = 5 -> 3 units
        assert_eq!(v.vector_length().integer(), 3);
        let str = heap::string_node("hello");
        assert!(str.is_string());
        assert_eq!(str.uvector_size(), 2); // header + length + 2 char cells
        assert_eq!(str.string_to_rust(), "hello");
    });
}

#[test]
fn nil_and_t() {
    with_lisp(|| {
        assert_eq!(qv(Nil_Index), nil());
        assert_eq!(symbol_value(nil()), nil());
        assert_eq!(symbol_value(t()), t());
        assert!(is_constant_symbol(nil()));
        assert_eq!(find_symbol("NIL"), nil());
        assert_eq!(find_symbol("T"), t());
        assert_eq!(read1("nil"), nil());
        assert_eq!(read1("()"), nil());
    });
}

#[test]
fn interning() {
    with_lisp(|| {
        let a = find_symbol("INTERN-ME");
        assert_eq!(a, find_symbol("INTERN-ME"));
        assert_eq!(read1("intern-me"), a);
        assert_eq!(a.uref(SYMBOL_PACKAGE), qv(CL_PACKAGE_Index));
        assert_ne!(find_pl_symbol("INTERN-ME"), a);
        let k = find_keyword("TEST");
        assert_eq!(symbol_value(k), k);
        assert!(is_constant_symbol(k));
        assert_eq!(k.uref(SYMBOL_PACKAGE), qv(KEYWORD_PACKAGE_Index));
        assert_eq!(qv(LAMBDA_Index), find_symbol("LAMBDA"));
        assert_eq!(qv(RETURN_FROM_Index), find_symbol("RETURN-FROM"));
    });
}

#[test]
fn many_symbols_in_package() {
    with_lisp(|| {
        let p = qv(CORMANLISP_PACKAGE_Index);
        // A Lisp vector, not a Vec: the Rust heap is not a GC root.
        let syms = heap::vector_node(500);
        for i in 0..500 {
            let s = find_pl_symbol(&format!("SYM-{i}"));
            syms.svset(i, s);
        }
        for i in 0..500 {
            let found = search_symbol(p, heap::string_node(&format!("SYM-{i}")));
            assert_eq!(found, syms.svref(i));
        }
    });
}

#[test]
fn read_print_roundtrip() {
    with_lisp(|| {
        for (src, want) in [
            ("foo", "FOO"),
            ("(a b c)", "(A B C)"),
            ("(a . b)", "(A . B)"),
            ("(a b . (c d))", "(A B C D)"),
            ("((a) (b (c)))", "((A) (B (C)))"),
            ("'x", "'X"),
            ("(quote x)", "'X"),
            ("#'car", "(FUNCTION CAR)"),
            ("42", "42"),
            ("-17", "-17"),
            ("+5", "5"),
            ("10.", "10"),
            ("1152921504606846975", "1152921504606846975"),
            ("1.5", "1.500000"),
            ("-2.5e2", "-250.000000"),
            ("1+", "1+"),
            ("-", "-"),
            ("1.2.3", "1.2.3"),
            ("|aBc|", "aBc"),
            ("a\\bc", "AbC"),
            ("\"a \\\"q\\\" b\"", "a \"q\" b"),
            ("#\\a", "a"),
            ("(a ; comment\n b)", "(A B)"),
            ("(a ; comment before close\n)", "(A)"),
            ("(a #| block |# b)", "(A B)"),
            ("`(a ,b ,@c)", "(APPEND (LIST 'A) (LIST B) C)"),
            ("`(a (b ,c))", "(APPEND (LIST 'A) (LIST (APPEND (LIST 'B) (LIST C))))"),
            ("`x", "'X"),
            ("`(a . ,b)", "(APPEND (LIST 'A) B)"),
            ("`(f ,(g x))", "(APPEND (LIST 'F) (LIST (G X)))"),
        ] {
            assert_eq!(rp(src), want, "reading {src:?}");
        }
    });
}

#[test]
fn read_vector() {
    with_lisp(|| {
        let v = read1("#(1 2 3)");
        assert!(v.is_simple_generic_vector());
        assert_eq!(v.vector_length().integer(), 3);
        assert_eq!(v.svref(2).integer(), 3);
        let v = read1("#5(a)");
        assert_eq!(v.vector_length().integer(), 5);
        assert_eq!(v.svref(4), nil());
    });
}

#[test]
fn read_multiple_and_eof() {
    with_lisp(|| {
        let v = read_all_from_str("a (b) ; trailing comment");
        assert_eq!(v.len(), 2);
        assert!(read_all_from_str("  ; only a comment\n").is_empty());
    });
}

#[test]
fn reader_errors() {
    with_lisp(|| {
        for src in ["(a b", "\"abc", "(. a)", "(a . b c)", ",x", "99999999999999999999"] {
            assert!(
                catch_lisp_error(|| read_all_from_str(src)).is_err(),
                "expected error reading {src:?}"
            );
        }
    });
}

#[test]
fn long_input_crosses_buffers() {
    with_lisp(|| {
        let n = 3000;
        let text = format!("({})", vec!["xy"; n].join(" "));
        let x = read1(&text);
        let mut len = 0;
        let mut p = x;
        while p.is_cons() {
            len += 1;
            p = p.cdr();
        }
        assert_eq!(len, n);
        let printed = write_to_string(x);
        assert_eq!(printed, text.to_uppercase());
    });
}

#[test]
fn read_bootstrap_lisp() {
    // The upstream first boot file must at least read cleanly.
    let path = "/home/nas/pkg/cormanlisp/Sys/bootstrap.lisp";
    if !std::path::Path::new(path).exists() {
        return;
    }
    with_lisp(|| {
        let s = cormanlisp::streams::make_file_input_stream(path);
        let mut n = 0;
        while cormanlisp::reader::read(s).is_some() {
            n += 1;
        }
        assert!(n > 10);
    });
}
