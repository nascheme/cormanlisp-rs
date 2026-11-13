//! Kernel primitives (port of the `Lispfunc.cpp` function table, subset).
//!
//! A primitive is a Rust function taking its arguments as a slice. KFunction
//! objects store the primitive's **index** in this table as a fixnum in
//! `FUNCTION_ADDRESS`, never a raw Rust pointer, so heap images survive
//! kernel rebuilds. Compiled code reaches them through the funcall stub
//! (`runtime.rs`).
#![allow(dead_code)]

pub mod core;
pub mod kernel;

use crate::error::lisp_error;
use crate::heap;
use crate::object::*;
use crate::qv::{nil, qv};
use crate::qv_index::*;
use crate::symbols::{find_symbol, set_symbol_function, symbol_function, symbol_function_type};
use crate::{reader, streams};
use self::core as c;
use self::kernel as k;

pub type PrimFn = fn(&[LispObj]) -> LispObj;

pub struct Prim {
    pub name: &'static str,
    pub func: PrimFn,
}

pub static PRIMS: &[Prim] = &[
    // reader macros (Read.cpp)
    Prim { name: "%DOUBLEQUOTEMACRO", func: reader::doublequote_macro },
    Prim { name: "%QUOTEMACRO", func: reader::quote_macro },
    Prim { name: "%LEFTPARENMACRO", func: reader::leftparen_macro },
    Prim { name: "%RIGHTPARENMACRO", func: reader::rightparen_macro },
    Prim { name: "%COMMAMACRO", func: reader::comma_macro },
    Prim { name: "%SEMICOLONMACRO", func: reader::semicolon_macro },
    Prim { name: "%BACKQUOTEMACRO", func: reader::backquote_macro },
    Prim { name: "%POUNDQUOTEMACRO", func: reader::pound_quote_macro },
    Prim { name: "%POUNDLEFTPARENMACRO", func: reader::pound_leftparen_macro },
    Prim { name: "%POUNDBACKSLASHMACRO", func: reader::pound_backslash_macro },
    Prim { name: "%BRACKETEDCOMMENTMACRO", func: reader::bracketed_comment_macro },
    Prim { name: "READ", func: reader::read_prim },
    Prim { name: "READ-DELIMITED-LIST", func: reader::read_delimited_list_prim },
    // stream buffer functions
    Prim { name: "CONSOLE-UNDERFLOW-FUNCTION", func: streams::console_underflow },
    Prim { name: "CONSOLE-OVERFLOW-FUNCTION", func: streams::console_overflow },
    Prim { name: "FILE-UNDERFLOW-FUNCTION", func: streams::file_underflow },
    Prim { name: "STRING-UNDERFLOW-FUNCTION", func: streams::string_underflow },
    Prim { name: "STRING-OVERFLOW-FUNCTION", func: streams::string_overflow },
    // symbols and functions
    Prim { name: "SYMBOL-VALUE", func: c::symbol_value_prim },
    Prim { name: "SYMBOL-FUNCTION", func: c::symbol_function_prim },
    Prim { name: "MACRO-FUNCTION", func: c::macro_function_prim },
    Prim { name: "SET-SYMBOL-FUNCTION", func: c::set_symbol_function_prim },
    Prim { name: "SET-SYMBOL-MACRO", func: c::set_symbol_macro_prim },
    Prim { name: "%SYMBOL-GET-FLAGS", func: c::symbol_get_flags },
    Prim { name: "%SYMBOL-SET-FLAGS", func: c::symbol_set_flags },
    Prim { name: "CONSTANTP", func: c::constantp },
    Prim { name: "GENSYM", func: c::gensym },
    Prim { name: "FIND-KEYWORD", func: c::find_keyword_prim },
    Prim { name: "FUNCALL", func: c::funcall_prim },
    Prim { name: "%PROGV", func: crate::runtime::progv_prim },
    Prim { name: "APPLY", func: c::apply_prim },
    Prim { name: "VALUES", func: c::values_prim },
    Prim { name: "EVAL", func: c::eval_prim },
    Prim { name: "LOAD", func: c::load_prim },
    Prim { name: "COMPILE-FORM", func: c::compile_form_prim },
    Prim { name: "COMPILE-LAMBDA", func: c::compile_lambda_prim },
    Prim { name: "ERROR", func: c::error_prim },
    // lists
    Prim { name: "CAR", func: c::car_prim },
    Prim { name: "CDR", func: c::cdr_prim },
    Prim { name: "CONS", func: c::cons_prim },
    Prim { name: "RPLACA", func: c::rplaca },
    Prim { name: "RPLACD", func: c::rplacd },
    Prim { name: "LIST", func: c::list_prim },
    Prim { name: "LIST*", func: c::list_star },
    Prim { name: "APPEND", func: c::append_prim },
    // predicates
    Prim { name: "NULL", func: c::null_prim },
    Prim { name: "EQ", func: c::eq_prim },
    Prim { name: "CONSP", func: c::consp },
    Prim { name: "LISTP", func: c::listp },
    Prim { name: "SYMBOLP", func: c::symbolp },
    Prim { name: "STRINGP", func: c::stringp },
    Prim { name: "FIXNUMP", func: c::fixnump },
    Prim { name: "INTEGERP", func: c::integerp },
    Prim { name: "FLOATP", func: c::floatp },
    Prim { name: "CHARACTERP", func: c::characterp },
    Prim { name: "VECTORP", func: c::vectorp },
    Prim { name: "ARRAYP", func: c::arrayp },
    Prim { name: "FUNCTIONP", func: c::functionp },
    Prim { name: "STREAMP", func: c::streamp },
    Prim { name: "PACKAGEP", func: c::packagep },
    // numbers
    Prim { name: "+", func: c::plus },
    Prim { name: "-", func: c::minus },
    Prim { name: "*", func: c::times },
    Prim { name: "/", func: c::divide },
    Prim { name: "=", func: c::num_eq },
    Prim { name: "<", func: c::num_lt },
    Prim { name: "<=", func: c::num_le },
    Prim { name: ">", func: c::num_gt },
    Prim { name: ">=", func: c::num_ge },
    Prim { name: "/=", func: c::num_ne },
    Prim { name: "TRUNCATE", func: c::truncate },
    Prim { name: "FLOOR", func: c::floor },
    Prim { name: "MOD", func: c::mod_prim },
    Prim { name: "BIT-OR", func: c::bit_or },
    // arrays
    Prim { name: "ARRAY-DIMENSION", func: c::array_dimension },
    // output
    Prim { name: "WRITE", func: c::write_prim },
    Prim { name: "TERPRI", func: c::terpri },
    Prim { name: "FORCE-OUTPUT", func: c::force_output },
    // misc
    Prim { name: "GET-INTERNAL-RUN-TIME", func: c::get_internal_run_time },
    Prim { name: "GET-GC-TIME", func: c::get_gc_time },
    Prim { name: "GET-INTERNAL-TIME-UNITS-PER-SECOND", func: c::get_internal_time_units_per_second },
    Prim { name: "GET-INTERNAL-REAL-TIME", func: c::get_internal_real_time },
    Prim { name: "%CORMANLISP-VERSION-STRING", func: c::version_string },
    Prim { name: "EDITOR-SET-MESSAGE", func: c::ignore },
    Prim { name: "EDITOR-SET-DEFAULT-MESSAGE", func: c::ignore },
    Prim { name: "CORMANLISP-CLIENT-TYPE", func: c::client_type },
    Prim { name: "CEILING", func: c::ceiling },
    Prim { name: "ROUND", func: c::round },
    // M2: kernel functions used by the load-sys files (kernel.rs)
    Prim { name: "CHAR-INT", func: k::char_int },
    Prim { name: "INT-CHAR", func: k::int_char },
    Prim { name: "CHAR-UPCASE", func: k::char_upcase },
    Prim { name: "CHAR-DOWNCASE", func: k::char_downcase },
    Prim { name: "UREF", func: k::uref },
    Prim { name: "UREF-SET", func: k::uref_set },
    Prim { name: "ALLOC-UVECTOR", func: k::alloc_uvector },
    Prim { name: "ALLOC-CHAR-VECTOR", func: k::alloc_char_vector },
    Prim { name: "UVECTORP", func: k::uvectorp },
    Prim { name: "UVECTOR-TYPE-BITS", func: k::uvector_type_bits },
    Prim { name: "UVECTOR-NUM-SLOTS", func: k::uvector_num_slots },
    Prim { name: "TAG-BITS", func: k::tag_bits },
    Prim { name: "%UVECTOR-ADDRESS", func: k::uvector_address },
    Prim { name: "UNINITIALIZED-OBJECT-P", func: k::uninitialized_object_p },
    Prim { name: "MAKE-SYMBOL", func: k::make_symbol },
    Prim { name: "%FMAKUNBOUND", func: k::fmakunbound },
    Prim { name: "FUNCTION-ENVIRONMENT", func: k::function_environment },
    Prim { name: "FUNCTION-INFO-LIST", func: k::function_info_list },
    Prim { name: "EXECUTION-ADDRESS", func: k::execution_address },
    Prim { name: "STRUCTUREP", func: k::structurep },
    Prim { name: "SEQUENCEP", func: k::sequencep },
    Prim { name: "HASH-TABLE-P", func: k::hash_table_p },
    Prim { name: "READTABLEP", func: k::readtablep },
    Prim { name: "BIGNUMP", func: k::bignump },
    Prim { name: "RATIOP", func: k::ratiop },
    Prim { name: "COMPLEXP", func: k::complexp },
    Prim { name: "DOUBLE-FLOAT-P", func: k::double_float_p },
    Prim { name: "SINGLE-FLOAT-P", func: k::single_float_p },
    Prim { name: "SHORT-FLOAT-P", func: k::short_float_p },
    Prim { name: "FOREIGNP", func: k::foreignp },
    Prim { name: "FOREIGN-HEAP-P", func: k::foreign_heap_p },
    Prim { name: "NUMBERP", func: k::numberp },
    Prim { name: "ELT", func: k::elt },
    Prim { name: "SETELT", func: k::setelt },
    Prim { name: "ROW-MAJOR-AREF", func: k::row_major_aref },
    Prim { name: "(SETF ROW-MAJOR-AREF)", func: k::set_row_major_aref },
    Prim { name: "ARRAY-TYPE", func: k::array_type },
    Prim { name: "ARRAY-RANK", func: k::array_rank },
    Prim { name: "COERCE", func: k::coerce },
    Prim { name: "STRING=", func: k::string_equal },
    Prim { name: "PACKAGE-HASH-INDEX", func: k::package_hash_index },
    Prim { name: "%OUTPUT-CHAR", func: k::output_char_prim },
    Prim { name: "%OUTPUT-CHARS", func: k::output_chars },
    Prim { name: "%READ-CHAR", func: k::read_char_internal },
    Prim { name: "%READ-CHAR-WITH-ERROR", func: k::read_char_with_error },
    Prim { name: "READ-CHAR", func: k::read_char },
    Prim { name: "UNREAD-CHAR", func: k::unread_char },
    Prim { name: "CLOSE", func: k::close },
    Prim { name: "OPEN-INPUT-FILE", func: k::open_input_file },
    Prim { name: "PROBE-FILE", func: k::probe_file },
    Prim { name: "%CHARS-TO-FLOAT", func: k::chars_to_float },
    Prim { name: "FLOAT", func: k::float },
    Prim { name: "LOGAND", func: k::logand },
    Prim { name: "LOGIOR", func: k::logior },
    Prim { name: "LOGXOR", func: k::logxor },
    Prim { name: "LOGEQV", func: k::logeqv },
    Prim { name: "LOGNOT", func: k::lognot },
    Prim { name: "LOGANDC1", func: k::logandc1 },
    Prim { name: "LOGANDC2", func: k::logandc2 },
    Prim { name: "LOGNAND", func: k::lognand },
    Prim { name: "LOGNOR", func: k::lognor },
    Prim { name: "LOGORC1", func: k::logorc1 },
    Prim { name: "LOGORC2", func: k::logorc2 },
    Prim { name: "%ASH", func: k::ash },
    Prim { name: "SQRT", func: k::sqrt },
    Prim { name: "ISQRT", func: k::isqrt },
    Prim { name: "EXP", func: k::exp },
    Prim { name: "EXPT", func: k::expt },
    Prim { name: "LOG", func: k::log },
    Prim { name: "SIN", func: k::sin },
    Prim { name: "COS", func: k::cos },
    Prim { name: "COMPLEX", func: k::complex },
    Prim { name: "PRINT-FLOAT", func: k::print_float },
    Prim { name: "HASH-EQ-FUNCTION", func: k::hash_eq_function },
    Prim { name: "HASH-EQL-FUNCTION", func: k::hash_eql_function },
    Prim { name: "HASH-EQUAL-FUNCTION", func: k::hash_equal_function },
    Prim { name: "HASH-EQUALP-FUNCTION", func: k::hash_equal_function },
    Prim { name: "GET-GC-ID", func: k::get_gc_id },
    Prim { name: "GC", func: k::gc },
    Prim { name: "PROTECT-STACK", func: k::return_nil },
    Prim { name: "REGISTER-UNTAGGED-VALUES", func: k::return_nil },
    Prim { name: "REGISTER-FINALIZATION", func: k::identity_first },
    Prim { name: "ALLOCATE-CRITICAL-SECTION", func: k::return_nil },
    Prim { name: "DEALLOCATE-CRITICAL-SECTION", func: k::return_nil },
    Prim { name: "ENTER-CRITICAL-SECTION", func: k::return_nil },
    Prim { name: "LEAVE-CRITICAL-SECTION", func: k::return_nil },
    Prim { name: "STACK-TRACE", func: k::stack_trace },
    Prim { name: "%LISP-SHUTDOWN", func: k::lisp_shutdown },
    Prim { name: "GET-UNIVERSAL-TIME", func: k::get_universal_time },
    Prim { name: "ARRAY-INITIALIZE-CONTENTS", func: k::array_initialize_contents },
    Prim { name: "ARRAY-INITIALIZE-ELEMENT", func: k::array_initialize_element },
    Prim { name: "VECTOR-SLOT-INITIALIZED", func: k::vector_slot_initialized },
    Prim { name: "WHICH-HEAP", func: k::which_heap },
    Prim { name: "HASH-OBJ-ID", func: k::hash_obj_id },
    Prim { name: "HASH-CELL", func: k::hash_cell },
    Prim { name: "FIXNUM-MOD", func: k::fixnum_mod },
    Prim { name: "REGISTRATION-INFO", func: k::registration_info },
    Prim { name: "SAVE-IMAGE", func: crate::image::save_image_prim },
    Prim { name: "HEAP-CURRENTLY-USED", func: k::heap_currently_used },
    // M4: kernel-asm.lisp replacements
    Prim { name: "%ASIN", func: k::asin },
    Prim { name: "%ATAN", func: k::atan },
    Prim { name: "%MAKE-SINGLE-FLOAT", func: k::make_single_float },
    Prim { name: "%SINGLE-FLOAT-BITS", func: k::single_float_bits },
    Prim { name: "%MAKE-DOUBLE-FLOAT", func: k::make_double_float },
    Prim { name: "%DOUBLE-FLOAT-HIGH-BITS", func: k::double_float_high_bits },
    Prim { name: "%DOUBLE-FLOAT-LOW-BITS", func: k::double_float_low_bits },
    Prim { name: "SVREF", func: k::svref },
    Prim { name: "(SETF SVREF)", func: k::set_svref },
    Prim { name: "SET", func: k::set },
    Prim { name: "BOUNDP", func: k::boundp },
    Prim { name: "MAKUNBOUND", func: k::makunbound },
    Prim { name: "IDENTITY", func: k::identity_first },
    Prim { name: "%SVREF", func: k::sv_ref },
    Prim { name: "(SETF %SVREF)", func: k::sv_set },
    Prim { name: "%SVCHAR", func: k::sv_ref },
    Prim { name: "(SETF %SVCHAR)", func: k::sv_set },
    Prim { name: "%SVBIT", func: k::sv_ref },
    Prim { name: "(SETF %SVBIT)", func: k::sv_set },
    Prim { name: "%SVBYTE", func: k::sv_ref },
    Prim { name: "(SETF %SVBYTE)", func: k::sv_set },
    Prim { name: "%SVDOUBLE", func: k::sv_ref },
    Prim { name: "(SETF %SVDOUBLE)", func: k::sv_set },
    Prim { name: "%SVSINGLE", func: k::sv_ref },
    Prim { name: "(SETF %SVSINGLE)", func: k::sv_set },
    Prim { name: "%SVSHORT", func: k::sv_ref },
    Prim { name: "(SETF %SVSHORT)", func: k::sv_set },
];

/// Upstream `kernelFunctionNode`, with a table index instead of a pointer.
pub fn kernel_function_node(index: usize) -> LispObj {
    let f = heap::alloc_uvector(KFUNCTION_SIZE, KFUNCTION_TYPE);
    f.uset(FUNCTION_ENVIRONMENT, nil());
    f.uset(FUNCTION_ADDRESS, LispObj::fixnum(index as i64));
    f
}

/// Upstream `initKernelFunctions`.
pub(crate) fn init_kernel_functions() {
    for (i, p) in PRIMS.iter().enumerate() {
        let sym = find_symbol(p.name);
        set_symbol_function(sym, kernel_function_node(i), qv(FUNCTION_Index));
    }
    for name in crate::compiler::frontend::SPECIAL_OPERATORS {
        let sym = find_symbol(name);
        sym.uset(SYMBOL_FUNCTION_TYPE, qv(SPECIAL_OPERATOR_Index));
    }
}

/// The global function named by `sym` (error if undefined or not a
/// function). Function objects are returned as is.
pub fn symbol_function_checked(sym: LispObj) -> LispObj {
    if sym.is_function() {
        return sym;
    }
    if !sym.is_symbol() || symbol_function_type(sym) != qv(FUNCTION_Index) {
        let n = crate::printer::write_to_string(sym);
        lisp_error(format!("The function {n} is not defined"));
    }
    symbol_function(sym)
}

pub fn prim_index(name: &str) -> Option<usize> {
    PRIMS.iter().position(|p| p.name == name)
}

/// Call a function object (or a symbol naming one). Sets the returned
/// value count in QV.
///
/// Kernel functions are also called through `lisp_call`, which copies the
/// arguments to the stack: `args` may be in a Rust `Vec`, which is not a
/// GC root, and the primitive may allocate.
pub fn funcall(f: LispObj, args: &[LispObj]) -> LispObj {
    let f = if f.is_symbol() { symbol_function_checked(f) } else { f };
    if crate::runtime::is_funcallable(f) {
        crate::runtime::lisp_call(f, args)
    } else {
        lisp_error(format!("Not a function: {}", crate::printer::write_to_string(f)))
    }
}

/// Check argument count, upstream `checkNumArgsRange`.
pub fn check_args(args: &[LispObj], min: usize, max: usize) {
    if args.len() < min || args.len() > max {
        lisp_error(format!(
            "Wrong number of arguments: number passed = {}",
            args.len()
        ));
    }
}
