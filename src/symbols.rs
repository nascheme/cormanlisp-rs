//! Symbols and packages. Port of `initLisp`/`initSymbols`, `findSymbol`,
//! `searchSymbol`, `addSymbol`, `symbolNode`, `packageNode` from upstream
//! `Lisp.cpp`.
#![allow(dead_code)]

use crate::error::lisp_error;
use crate::heap::{self, cons, string_node, vector_node};
use crate::object::*;
use crate::qv::{nil, qv, set_qv, t};
use crate::qv_index::*;

// ---------------------------------------------------------------------------
// Symbol accessors

pub fn symbol_name(sym: LispObj) -> LispObj {
    sym.uref(SYMBOL_NAME)
}

/// Upstream `symbolValue`: honors a per-thread special binding slot.
pub fn symbol_value(sym: LispObj) -> LispObj {
    value_cell(sym).car()
}

pub fn set_symbol_value(sym: LispObj, v: LispObj) {
    value_cell(sym).set_car(v)
}

fn value_cell(sym: LispObj) -> LispObj {
    let vt = sym.uref(SYMBOL_VAR_TABLE);
    if vt.raw() != 0 {
        qv(vt.integer() as usize)
    } else {
        sym.uref(SYMBOL_VALUE)
    }
}

pub fn is_bound(sym: LispObj) -> bool {
    symbol_value(sym) != SYMBOL_UNBOUND
}

pub fn symbol_function(sym: LispObj) -> LispObj {
    sym.uref(SYMBOL_FUNCTION).car()
}

pub fn symbol_function_type(sym: LispObj) -> LispObj {
    sym.uref(SYMBOL_FUNCTION_TYPE)
}

/// Upstream `setSymbolFunction`. PORT: the jump table update is added with
/// the compiler (M1).
pub fn set_symbol_function(sym: LispObj, func: LispObj, ty: LispObj) {
    if !func.is_function() {
        lisp_error("Not a function");
    }
    sym.uref(SYMBOL_FUNCTION).set_car(func);
    sym.uset(SYMBOL_FUNCTION_TYPE, ty);
}

pub fn set_special_symbol(sym: LispObj) {
    let f = sym.uref(SYMBOL_FLAGS).raw();
    sym.uset(SYMBOL_FLAGS, LispObj(f | SYMBOL_SPECIAL_FLAG));
}

pub fn set_constant_symbol(sym: LispObj) {
    let f = sym.uref(SYMBOL_FLAGS).raw();
    sym.uset(SYMBOL_FLAGS, LispObj(f | SYMBOL_CONSTANT_FLAG));
}

pub fn is_special_symbol(sym: LispObj) -> bool {
    sym.uref(SYMBOL_FLAGS).raw() & SYMBOL_SPECIAL_FLAG != 0
}

pub fn is_constant_symbol(sym: LispObj) -> bool {
    sym.uref(SYMBOL_FLAGS).raw() & SYMBOL_CONSTANT_FLAG != 0
}

/// Upstream `symbolNode`: a fresh uninterned symbol.
pub fn symbol_node(name: LispObj) -> LispObj {
    let sym = heap::alloc_uvector(SYMBOL_SIZE, SYMBOL_TYPE);
    let t1 = cons(UNINITIALIZED, nil());
    let t2 = cons(UNINITIALIZED, nil());
    sym.uset(SYMBOL_NAME, name);
    sym.uset(SYMBOL_VALUE, t1);
    sym.uset(SYMBOL_PACKAGE, nil());
    sym.uset(SYMBOL_PROPERTY_LIST, nil());
    sym.uset(SYMBOL_CONSTANT, LispObj(0));
    sym.uset(SYMBOL_FUNCTION, t2);
    sym.uset(SYMBOL_FUNCTION_TYPE, nil());
    sym.uset(SYMBOL_JUMP_TABLE, LispObj(0));
    sym.uset(SYMBOL_VAR_TABLE, LispObj(0));
    sym
}

// ---------------------------------------------------------------------------
// Packages

pub fn package_capacity(pk: LispObj) -> usize {
    pk.uref(PACKAGE_CAPACITY).integer() as usize
}

fn package_entry(pk: LispObj, n: usize) -> *mut LispObj {
    unsafe { pk.uref(PACKAGE_TABLE).array_start().add(3 * n) }
}

fn entry_get(pk: LispObj, n: usize, k: usize) -> LispObj {
    unsafe { *package_entry(pk, n).add(k) }
}

fn entry_set(pk: LispObj, n: usize, k: usize, v: LispObj) {
    unsafe { *package_entry(pk, n).add(k) = v }
}

fn units_of(str: LispObj) -> &'static [LispChar] {
    let len = str.vector_length().integer() as usize;
    unsafe { std::slice::from_raw_parts(str.char_array_start(), len) }
}

pub fn lisp_strings_equal(a: LispObj, b: LispObj) -> bool {
    units_of(a) == units_of(b)
}

/// Upstream kernel `PACKAGE-HASH-INDEX`. Arithmetic is done in 32 bits to
/// match the upstream kernel exactly.
pub fn package_hash_index(pk: LispObj, str: LispObj) -> usize {
    let mut h: i32 = 0;
    for &c in units_of(str) {
        h ^= c as i32;
        h = h.wrapping_shl(5);
    }
    let h = (h as i64).abs();
    (h % package_capacity(pk) as i64) as usize
}

/// Upstream `searchSymbol`. Returns UNINITIALIZED if not present.
pub fn search_symbol(pk: LispObj, str: LispObj) -> LispObj {
    let size = package_capacity(pk) as i64;
    let mut h = package_hash_index(pk, str) as i64;
    let key = entry_get(pk, h as usize, 0);
    if !key.is_string() {
        return UNINITIALIZED;
    }
    if lisp_strings_equal(str, key) {
        return entry_get(pk, h as usize, 1);
    }
    let mut offset = (size - 1) - h;
    if offset == 0 {
        offset = 1;
    }
    loop {
        h -= offset;
        if h < 0 {
            h += size;
        }
        let key = entry_get(pk, h as usize, 0);
        if !key.is_string() {
            return UNINITIALIZED;
        }
        if lisp_strings_equal(str, key) {
            return entry_get(pk, h as usize, 1);
        }
    }
}

/// Upstream `addSymbol`.
pub fn add_symbol(pk: LispObj, key: LispObj, value: LispObj, external: bool) -> LispObj {
    let size = package_capacity(pk) as i64;
    let count = pk.uref(PACKAGE_COUNT).integer();
    if count as f64 > PACKAGE_RESIZE_THRESHOLD * size as f64 {
        lisp_error("Package capacity exceeded"); // upstream: growPackage unimplemented
    }
    let mut h = package_hash_index(pk, key) as i64;
    let k0 = entry_get(pk, h as usize, 0);
    if k0.is_string() && !lisp_strings_equal(k0, key) {
        let mut offset = (size - 1) - h;
        if offset == 0 {
            offset = 1;
        }
        loop {
            h -= offset;
            if h < 0 {
                h += size;
            }
            let k = entry_get(pk, h as usize, 0);
            if !k.is_string() || lisp_strings_equal(k, key) {
                break;
            }
        }
    }
    let h = h as usize;
    if !entry_get(pk, h, 0).is_string() {
        entry_set(pk, h, 0, key);
    }
    entry_set(pk, h, 1, value);
    entry_set(pk, h, 2, LispObj::fixnum(external as i64));
    pk.uset(PACKAGE_COUNT, LispObj::fixnum(count + 1));
    value.uset(SYMBOL_PACKAGE, pk);
    value
}

/// Upstream `packageNode`.
pub fn package_node(name: LispObj) -> LispObj {
    let p = heap::alloc_uvector(PACKAGE_SIZE, PACKAGE_TYPE);
    let table = vector_node(1009 * 3);
    p.uset(PACKAGE_NAME, name);
    p.uset(PACKAGE_NICKNAMES, nil());
    p.uset(PACKAGE_USE_LIST, nil());
    p.uset(PACKAGE_USED_BY_LIST, nil());
    p.uset(PACKAGE_SHADOWING_SYMBOLS, nil());
    p.uset(PACKAGE_CAPACITY, LispObj::fixnum(1009));
    p.uset(PACKAGE_COUNT, LispObj(0));
    p.uset(PACKAGE_TABLE, table);
    p.uset(PACKAGE_SYNC, nil());
    p
}

fn find_in(pk: LispObj, name: &str) -> LispObj {
    let str = string_node(name);
    let sym = search_symbol(pk, str);
    if sym != UNINITIALIZED {
        return sym;
    }
    add_symbol(pk, str, symbol_node(str), false)
}

/// Upstream `findSymbol`: intern `name` in COMMON-LISP.
pub fn find_symbol(name: &str) -> LispObj {
    find_in(qv(CL_PACKAGE_Index), name)
}

/// Intern a symbol whose name is a Lisp string, in COMMON-LISP.
pub fn find_symbol_str(str: LispObj) -> LispObj {
    let pk = qv(CL_PACKAGE_Index);
    let sym = search_symbol(pk, str);
    if sym != UNINITIALIZED {
        return sym;
    }
    add_symbol(pk, str, symbol_node(str), false)
}

/// Upstream `findPLSymbol`: intern in CORMANLISP.
pub fn find_pl_symbol(name: &str) -> LispObj {
    find_in(qv(CORMANLISP_PACKAGE_Index), name)
}

/// Upstream `findKeyword`.
pub fn find_keyword(name: &str) -> LispObj {
    let pk = qv(KEYWORD_PACKAGE_Index);
    let str = string_node(name);
    let sym = search_symbol(pk, str);
    if sym != UNINITIALIZED {
        return sym;
    }
    let sym = symbol_node(str);
    add_symbol(pk, str, sym, false);
    set_symbol_value(sym, sym);
    set_constant_symbol(sym);
    set_special_symbol(sym);
    sym
}

// ---------------------------------------------------------------------------
// Initialization

/// QV slots bound to `findSymbol(<name>)` at startup, from upstream
/// `initSymbols` (`INIT_SYMBOL(X)` is `X = findSymbol("X")`).
const QV_SYMBOLS: &[(usize, &str)] = &[
    (QUOTE_Index, "QUOTE"),
    (FUNCTION_Index, "FUNCTION"),
    (LIST_Index, "LIST"),
    (APPEND_Index, "APPEND"),
    (MACRO_Index, "MACRO"),
    (LAMBDA_Index, "LAMBDA"),
    (SETQ_Index, "SETQ"),
    (FUNCALL_Index, "FUNCALL"),
    (BLOCK_Index, "BLOCK"),
    (PROGN_Index, "PROGN"),
    (COMPILER_STACK_ENV_Index, "COMPILER_STACK_ENV"),
    (COMPILER_HEAP_ENV_Index, "COMPILER_HEAP_ENV"),
    (EBP_Index, "EBP"),
    (EBX_Index, "EBX"),
    (ARG_ENV_Index, "ARG_ENV"),
    (ARG_NUM_Index, "ARG_NUM"),
    (IF_Index, "IF"),
    (LET_Index, "LET"),
    (TAGBODY_Index, "TAGBODY"),
    (GO_Index, "GO"),
    (RETURN_Index, "RETURN"),
    (COMPILER_LABELS_Index, "COMPILER_LABELS"),
    (COMPILER_BLOCKS_Index, "COMPILER_BLOCKS"),
    (CONS_Index, "CONS"),
    (COMPILER_CLEANUPS_Index, "COMPILER_CLEANUPS"),
    (COMPILER_RUNTIME_Index, "COMPILER_RUNTIME"),
    (CATCH_Index, "CATCH"),
    (THROW_Index, "THROW"),
    (VALUES_Index, "VALUES"),
    (APPLY_Index, "APPLY"),
    (FLET_Index, "FLET"),
    (LABELS_Index, "LABELS"),
    (MACROLET_Index, "MACROLET"),
    (ENVIRONMENT_Index, "ENVIRONMENT"),
    (INTERNAL_Index, "INTERNAL"),
    (EXTERNAL_Index, "EXTERNAL"),
    (STRING_Index, "STRING"),
    (READ_Index, "READ"),
    (INTERN_Index, "INTERN"),
    (DECLARE_Index, "DECLARE"),
    (SPECIAL_Index, "SPECIAL"),
    (EVAL_Index, "EVAL"),
    (BIT_Index, "BIT"),
    (THE_Index, "THE"),
    (TYPEP_Index, "TYPEP"),
    (FORMAT_Index, "FORMAT"),
    (AREF_Index, "AREF"),
    (SYMBOL_Index, "SYMBOL"),
    (FIXNUM_Index, "FIXNUM"),
    (BIGNUM_Index, "BIGNUM"),
    (FLOAT_Index, "FLOAT_SYM"), // sic: upstream INIT_SYMBOL(FLOAT_SYM)
    (COMPLEX_Index, "COMPLEX"),
    (RATIO_Index, "RATIO"),
    (NUMBER_Index, "NUMBER"),
    (LETSTAR_Index, "LET*"),
    (RETURN_FROM_Index, "RETURN-FROM"),
    (EVAL_WHEN_Index, "EVAL-WHEN"),
    (SYMBOL_VALUE_Index, "SYMBOL-VALUE"),
    (SYMBOL_FUNCTION_SYM_Index, "SYMBOL-FUNCTION"),
    (LISTSTAR_Index, "LIST*"),
    (SPECIAL_OPERATOR_Index, "SPECIAL-OPERATOR"),
    (CEof_Index, "EOF"),
    (COMMA_TOKEN_Index, "%__COMMA__"),
    (COMMA_DOT_TOKEN_Index, "%__COMMA_DOT__"),
    (COMMA_ATSIGN_TOKEN_Index, "%__COMMA_ATSIGN__"),
    (CREATE_CLOSURE_Index, "%CREATE-CLOSURE"),
    (DOT_Index, "."),
    (RIGHT_PAREN_Index, ")"),
    (READTABLE_Index, "*READTABLE*"),
    (STRINGNODE_Index, "%STRINGNODE"),
    (STANDARD_INPUT_Index, "*STANDARD-INPUT*"),
    (STANDARD_OUTPUT_Index, "*STANDARD-OUTPUT*"),
    (WRONG_NUMBER_OF_ARGS_Index, "%WRONG-NUMBER-OF-ARGS"),
    (INVALID_FIXNUM_Index, "%INVALID-FIXNUM"),
    (UNBOUND_VARIABLE_Index, "%UNBOUND-VARIABLE"),
    (CHECK_LIST_Index, "%CHECK-LIST"),
    (UNDEFINED_FUNCTION_Index, "%UNDEFINED-FUNCTION"),
    (PLUS_Index, "+"),
    (MINUS_Index, "-"),
    (ONEPLUS_Index, "1+"),
    (ONEMINUS_Index, "1-"),
    (NUMERIC_EQUAL_Index, "="),
    (LESS_EQUAL_Index, "<="),
    (GREATER_EQUAL_Index, ">="),
    (LESS_Index, "<"),
    (GREATER_Index, ">"),
    (ALLOC_CONS_Index, "%ALLOC-CONS"),
    (ALLOC_VECTOR_Index, "%ALLOC-VECTOR"),
    (UNWIND_PROTECT_Index, "UNWIND-PROTECT"),
    (PUSH_CATCHER_Index, "%PUSH_CATCHER"),
    (POP_CATCHER_Index, "%POP_CATCHER"),
    (POP_SPECIALS_Index, "%POP_SPECIALS"),
    (THROW_EXCEPTION_Index, "%THROW_EXCEPTION"),
    (THROW_TAG_Index, "%THROW_TAG"),
    (THROW_RETURN_VALUE_Index, "%THROW_RETURN_VALUE"),
    (THROW_NUM_RETURN_VALUES_Index, "%THROW_NUM_RETURN_VALUES"),
    (LBYTE_Index, "BYTE"),
    (MULTIPLE_VALUE_CALL_Index, "MULTIPLE-VALUE-CALL"),
    (MACROEXPAND_ALL_Index, "MACROEXPAND-ALL"),
    (MACROEXPAND_ALL_EXCEPT_TOP_Index, "MACROEXPAND-ALL-EXCEPT-TOP"),
    (MACROEXPAND_INLINE_Index, "MACROEXPAND-INLINE"),
    (EMBEDDED_LAMBDAS_Index, "*EMBEDDED-LAMBDAS*"),
    (ENV_COUNTER_Index, "*ENV-COUNTER*"),
    (CONSOLE_INPUT_Index, "CONSOLE-INPUT"),
    (CONSOLE_OUTPUT_Index, "CONSOLE-OUTPUT"),
    (FILE_INPUT_Index, "FILE-INPUT"),
    (FILE_OUTPUT_Index, "FILE-OUTPUT*"),
    (STRING_INPUT_Index, "STRING-INPUT"),
    (STRING_OUTPUT_Index, "STRING-OUTPUT"),
    (TOP_LEVEL_Index, "*TOP-LEVEL*"),
    (LISPERROR_Index, "ERROR"),
    (LAMBDA_OPTIONAL_Index, "&OPTIONAL"),
    (LAMBDA_REST_Index, "&REST"),
    (LAMBDA_KEY_Index, "&KEY"),
    (LAMBDA_AUX_Index, "&AUX"),
    (LAMBDA_ALLOW_OTHER_KEYS_Index, "&ALLOW-OTHER-KEYS"),
    (LAMBDA_WHOLE_Index, "&WHOLE"),
    (LAMBDA_BODY_Index, "&BODY"),
    (WHITESPACE_CHAR_TYPE_Index, "WHITESPACE-CHAR-TYPE"),
    (CONSTITUENT_CHAR_TYPE_Index, "CONSTITUENT-CHAR-TYPE"),
    (TERMINATING_MACRO_CHAR_TYPE_Index, "TERMINATING-MACRO-CHAR-TYPE"),
    (NON_TERMINATING_MACRO_CHAR_TYPE_Index, "NON-TERMINATING-MACRO-CHAR-TYPE"),
    (DISPATCHING_TERMINATING_MACRO_CHAR_TYPE_Index, "DISPATCHING-TERMINATING-MACRO-CHAR-TYPE"),
    (DISPATCHING_NON_TERMINATING_MACRO_CHAR_TYPE_Index, "DISPATCHING-NON-TERMINATING-MACRO-CHAR-TYPE"),
    (SINGLE_ESCAPE_CHAR_TYPE_Index, "SINGLE-ESCAPE-CHAR-TYPE"),
    (MULTIPLE_ESCAPE_CHAR_TYPE_Index, "MULTIPLE-ESCAPE-CHAR-TYPE"),
    (ILLEGAL_CHAR_TYPE_Index, "ILLEGAL-CHAR-TYPE"),
    (PACKAGE_Index, "*PACKAGE*"),
    (PACKAGE_LIST_Index, "*PACKAGE-LIST*"),
    (EQ_Index, "EQ"),
    (CAR_SYM_Index, "CAR"),
    (CDR_SYM_Index, "CDR"),
    (NULL_SYM_Index, "NULL"),
    (NOT_Index, "NOT"),
    (LAMBDA_SPECIAL_VARS_Index, "*LAMBDA-SPECIAL-VARS*"),
    (LAMBDA_DECLARATIONS_Index, "*LAMBDA-DECLARATIONS*"),
    (LAMBDA_SPECIAL_DECS_Index, "*LAMBDA-SPECIAL-DECS*"),
    (POP_SPECIAL_BINDINGS_Index, "%POP-SPECIAL-BINDINGS"),
    (PUSH_SPECIAL_BINDINGS_Index, "%PUSH-SPECIAL-BINDINGS"),
    (ESTABLISH_SPECIAL_BINDINGS_Index, "%ESTABLISH-SPECIAL-BINDINGS"),
    (COMMON_LISP_READTABLE_Index, "*COMMON-LISP-READTABLE*"),
    (COMPILER_SAVE_LAMBDAS_Index, "*COMPILER-SAVE-LAMBDAS*"),
    (COMPILER_SAVE_LAMBDA_LISTS_Index, "*COMPILER-SAVE-LAMBDA-LISTS*"),
    (COMPILER_SAVE_STACK_FRAME_INFO_Index, "*COMPILER-SAVE-STACK-FRAME-INFO*"),
    (COMPILER_SAVE_ENVIRONMENT_INFO_Index, "*COMPILER-SAVE-ENVIRONMENT-INFO*"),
    (COMPILER_SAVE_DEBUG_INFO_Index, "*COMPILER-SAVE-DEBUG-INFO*"),
    (COMPILER_SAFETY_LEVEL_Index, "*COMPILER-SAFETY-LEVEL*"),
    (LAMBDA_LIST_Index, "LAMBDA-LIST"),
    (ENVIRONMENT_INFO_Index, "ENVIRONMENT-INFO"),
    (STACK_FRAME_Index, "STACK-FRAME"),
    (DEBUG_INFO_Index, "DEBUG-INFO"),
    (FUNCTION_NAME_Index, "FUNCTION-NAME"),
    (CREATE_BIG_NUM_Index, "%CREATE-BIG-NUM"),
    (MULTIPLE_VALUE_PROG1_Index, "MULTIPLE-VALUE-PROG1"),
    (CHECK_TYPE_BODY_Index, "CHECK-TYPE-BODY"),
    (HASH_TABLE_Index, "HASH-TABLE"),
    (FUNCALL_IGNORING_ERRORS_Index, "FUNCALL-IGNORING-ERRORS"),
    (SETF_AREF_Index, "(SETF AREF)"),
    (AREF_FUNCS_Index, "AREF-FUNCS"),
    (AREF_SETTER_FUNCS_Index, "AREF-SETTER-FUNCS"),
    (COMPILE_FUNCTION_CALL_FORM_Index, "COMPILE-FUNCTION-CALL-FORM"),
    (COMPILATION_SPEED_Index, "COMPILATION-SPEED"),
    (DEBUG_Index, "DEBUG"),
    (SAFETY_Index, "SAFETY"),
    (SPACE_Index, "SPACE"),
    (SPEED_Index, "SPEED"),
    (DECLARATION_Index, "DECLARATION"),
    (DYNAMIC_EXTENT_Index, "DYNAMIC-EXTENT"),
    (FTYPE_Index, "FTYPE"),
    (IGNORABLE_Index, "IGNORABLE"),
    (IGNORE_Index, "IGNORE"),
    (INLINE_Index, "INLINE"),
    (NOTINLINE_Index, "NOTINLINE"),
    (OPTIMIZE_Index, "OPTIMIZE"),
    (TYPE_Index, "TYPE"),
    (CONSOLE_INPUT_STREAM_Index, "CONSOLE-INPUT-STREAM"),
    (CONSOLE_OUTPUT_STREAM_Index, "CONSOLE-OUTPUT-STREAM"),
    (COMPILER_WARN_ON_UNDEFINED_FUNCTION_Index, "*COMPILER-WARN-ON-UNDEFINED-FUNCTION*"),
    (COMPILER_WARN_ON_DYNAMIC_RETURN_Index, "*COMPILER-WARN-ON-DYNAMIC-RETURN*"),
    (COMPILER_FUNCTION_NAME_Index, "*COMPILER-FUNCTION-NAME*"),
    (COMPILER_USES_ENV_Index, "*COMPILER-USES_ENV*"),
    (GET_INTERNAL_RUN_TIME_Index, "GET-INTERNAL-RUN-TIME"),
    (GC_TIME_COUNTER_Index, "*GC-TIME-COUNTER*"),
    (PATHNAME_Index, "PATHNAME"),
    (DEALLOCATE_C_HEAP_Index, "DEALLOCATE-C-HEAP"),
    (SYSTEM_INTERNALS_Index, "*SYSTEM-INTERNALS*"),
    (GC_CRITICAL_SECTION_Index, "*GC-CRITICAL-SECTION*"),
    (TQ_CRITICAL_SECTION_Index, "*TQ-CRITICAL-SECTION*"),
    (PAGE_TABLE_Index, "*PAGE-TABLE*"),
    (DISASSEMBLY_OUTPUT_BUF_Index, "*DISASSEMBLY-OUTPUT-BUF*"),
    (SAVE_IMAGE_CLEANUP_FUNCS_Index, "*SAVE-IMAGE-CLEANUP-FUNCS*"),
    (LOAD_IMAGE_RESTORE_FUNCS_Index, "*LOAD-IMAGE-RESTORE-FUNCS*"),
    (CONSOLE_STREAM_Index, "CONSOLE-STREAM"),
    (FILE_STREAM_Index, "FILE-STREAM"),
    (STRING_STREAM_Index, "STRING-STREAM"),
    (CONSOLE_OVERFLOW_FUNCTION_Index, "CONSOLE-OVERFLOW-FUNCTION"),
    (CONSOLE_UNDERFLOW_FUNCTION_Index, "CONSOLE-UNDERFLOW-FUNCTION"),
    (FILE_OVERFLOW_FUNCTION_Index, "FILE-OVERFLOW-FUNCTION"),
    (FILE_UNDERFLOW_FUNCTION_Index, "FILE-UNDERFLOW-FUNCTION"),
    (STRING_OVERFLOW_FUNCTION_Index, "STRING-OVERFLOW-FUNCTION"),
    (STRING_UNDERFLOW_FUNCTION_Index, "STRING-UNDERFLOW-FUNCTION"),
    (COLLECT_LITERALS_Index, "COLLECT-LITERALS"),
    (EXECUTION_ADDRESS_Index, "EXECUTION-ADDRESS"),
    (FIND_FUNCTION_CURR_OFFSET_Index, "FIND_FUNCTION_CURR_OFFSET"),
    (FIND_FUNCTION_CURR_FUNC_Index, "FIND_FUNCTION_CURR_FUNC"),
    (FIND_FUNCTION_CURR_ADDR_Index, "FIND_FUNCTION_CURR_ADDR"),
    (ADDRESS_FIND_FUNCTION_CALLBACK_Index, "ADDRESS-FIND-FUNCTION-CALLBACK"),
    (TERMINAL_IO_Index, "*TERMINAL-IO*"),
    (TYPEEXPAND_ALL_Index, "TYPEEXPAND-ALL"),
    (LOOKUP_FTYPE_Index, "LOOKUP-FTYPE"),
    (LOAD_QV_REG_Index, "%LOAD-QV-REG"),
    (GET_Index, "GET"),
    (TYPE_DISCRIMINATOR_Index, "TYPE-DISCRIMINATOR"),
    (HEAP_CHECKING_Index, "*HEAP-CHECKING*"),
    (INVALID_OBJECT_COUNT_Index, "*INVALID-OBJECT-COUNT*"),
    (SHORT_SYM_Index, "SHORT-INTEGER"),
    (CODE_JUMP_TABLE_REFS_Index, "*CODE-JUMP-TABLE-REFS*"),
    (CODE_VAR_TABLE_REFS_Index, "*CODE-VAR-TABLE-REFS*"),
    (CODE_ENV_TABLE_REFS_Index, "*CODE-ENV-TABLE-REFS*"),
    (PLUS_EAX_EDX_Index, "%PLUS_EAX_EDX"),
    (MINUS_EAX_EDX_Index, "%MINUS_EAX_EDX"),
    (ERROR_SIGNAL_Index, "%ERROR"),
    (INTERNAL_TIME_UNITS_PER_SECOND_Index, "INTERNAL-TIME-UNITS-PER-SECOND"),
    (GC_TIME_UNITS_PER_SECOND_Index, "GC-TIME-UNITS-PER-SECOND"),
    (APPEND_REFS_TO_CODE_Index, "*APPEND-REFS-TO-CODE*"),
    (SINGLE_FLOAT_Index, "SINGLE-FLOAT"),
    (DOUBLE_FLOAT_Index, "DOUBLE-FLOAT"),
    (SHORT_FLOAT_Index, "SHORT-FLOAT"),
    (LONG_FLOAT_Index, "LONG-FLOAT"),
    (READ_DEFAULT_FLOAT_FORMAT_Index, "*READ-DEFAULT-FLOAT-FORMAT*"),
    (SYS_GLOBALS_ADDRESS_Index, "SYS-GLOBALS-ADDRESS"),
    (EXIT_THREAD_TAG_Index, "%EXIT_THREAD_TAG"),
    (DEALLOCATE_CRITICAL_SECTION_Index, "DEALLOCATE-CRITICAL-SECTION"),
    (COMPRESS_IMG_Index, "*COMPRESS-IMG*"),
    (ENV_Index, "ENV"),
    (ARGS_Index, "ARGS"),
    (FRAME_Index, "FRAME"),
    (GLOBAL_Index, "GLOBAL"),
    (COMPILED_SPECIALS_Index, "*COMPILED-SPECIALS*"),
    (GET_CURRENT_ENVIRONMENT_Index, "GET-CURRENT-ENVIRONMENT"),
    (CAPTURE_COMPILER_ENVIRONMENT_Index, "CAPTURE-COMPILER-ENVIRONMENT"),
    (COMPILER_FRAME_INFO_Index, "*COMPILER-FRAME-INFO*"),
    (COMPILER_ENVIRONMENT_Index, "*COMPILER-ENVIRONMENT*"),
    (THROW_SYSTEM_EXCEPTION_Index, "%THROW-SYSTEM-EXCEPTION"),
    (WITH_ONLY_LEXICALS_Index, "WITH-ONLY-LEXICALS"),
    (COMPILER_WARN_ON_UNUSED_VAR_Index, "*COMPILER-WARN-ON-UNUSED-VARIABLE*"),
    (WRITE_Index, "WRITE"),
    (INLINE_PROCLAIM_P_Index, "INLINE-PROCLAIM-P"),
    (FINALIZATION_PENDING_Index, "*FINALIZATION-PENDING*"),
    (EXECUTE_FINALIZERS_Index, "%EXECUTE-FINALIZERS"),
    (UNDEFINED_FUNCTIONS_Index, "*UNDEFINED-FUNCTIONS*"),
    (GENSYM_Index, "GENSYM"),
    (COMPILER_CHECK_ARGS_NUM_Index, "COMPILER-CHECK-ARGS-NUM"),
    (COMPILER_CHECK_TYPES_Index, "COMPILER-CHECK-TYPES"),
    (COMPILER_FOLD_CONSTANTS_Index, "COMPILER-FOLD-CONSTANTS"),
    (COMPILER_INLINE_FUNCTIONS_Index, "COMPILER-INLINE-FUNCTIONS"),
    (COMPILER_OPTIMIZE_TAIL_RECURSION_Index, "COMPILER-OPTIMIZE-TAIL-RECURSION"),
    (LOAD_LOCAL_HEAP_Index, "%LOAD-LOCAL-HEAP"),
    (COLLECT_LEXICAL_MACROS_Index, "*COLLECT-LEXICAL-MACROS*"),
    (LEXICAL_MACROS_Index, "*LEXICAL-MACROS*"),
    (COLLECT_LEXICAL_SYMBOL_MACROS_Index, "*COLLECT-LEXICAL-SYMBOL-MACROS*"),
    (LEXICAL_SYMBOL_MACROS_Index, "*LEXICAL-SYMBOL-MACROS*"),
    (COMPILER_LAMBDA_LIST_Index, "*COMPILER-LAMBDA-LIST*"),
    (REMOVE_TAIL_RECURSION_Index, "REMOVE-TAIL-RECURSION"),
    (LOOKUP_SETF_FUNCTION_Index, "LOOKUP-SETF-FUNCTION"),
    (SETF_Index, "SETF"),
    (COMPILER_CHECK_KEY_ARGS_Index, "COMPILER-CHECK-KEY-ARGS"),
    (INVALID_KEY_ARG_Index, "INVALID-KEY-ARG"),
    (LOCALLY_Index, "LOCALLY"),
    (LOAD_TIME_VALUE_Index, "LOAD-TIME-VALUE"),
    (GET_CALLBACK_Index, "GET-CALLBACK"),
    (HEAP_FAULT_HANDLER_Index, "%HEAP_FAULT_HANDLER"),
    (ERROR_OUTPUT_Index, "*ERROR-OUTPUT*"),
    (COMPILER_WARN_ON_ASSUMED_SPECIAL_Index, "*COMPILER-WARN-ON-ASSUMED-SPECIAL*"),
    (COMPILING_LAMBDA_Index, "*COMPILING-LAMBDA*"),
    (CAPTURED_LEXICAL_CALLBACK_Index, "*CAPTURED-LEXICAL-CALLBACK*"),
    (WARN_Index, "WARN"),
    (RESET_HASH_ID_Index, "RESET-HASH-ID"),
    (LOAD_TIME_VALUES_Index, "*LOAD-TIME-VALUES*"),
    (HEAP_0_START_Index, "HEAP_0_START"),
    (HEAP_0_END_Index, "HEAP_0_END"),
    (HEAP_1_START_Index, "HEAP_1_START"),
    (HEAP_1_END_Index, "HEAP_1_END"),
    (HEAP_2_START_Index, "HEAP_2_START"),
    (HEAP_2_END_Index, "HEAP_2_END"),
    (HEAP_0_GC_ID_Index, "HEAP_0_GC_ID"),
    (HEAP_1_GC_ID_Index, "HEAP_1_GC_ID"),
    (HEAP_2_GC_ID_Index, "HEAP_2_GC_ID"),
];

/// Keywords bound to QV slots in upstream `initLisp`.
const QV_KEYWORDS: &[(usize, &str)] = &[
    (EXECUTE_KEY_Index, "EXECUTE"),
    (INPUT_KEY_Index, "INPUT"),
    (OUTPUT_KEY_Index, "OUTPUT"),
    (BIDIRECTIONAL_KEY_Index, "BIDIRECTIONAL"),
];

/// Create NIL, T and CHARACTER by hand, as upstream does, since
/// `symbol_node` needs NIL.
fn make_bootstrap_symbol(name: &str) -> LispObj {
    let sym = heap::alloc_uvector(SYMBOL_SIZE, SYMBOL_TYPE);
    sym.uset(SYMBOL_NAME, string_node(name));
    sym
}

fn finish_bootstrap_symbol(sym: LispObj, value: LispObj, flags: usize) {
    let n = nil();
    sym.uset(SYMBOL_VALUE, cons(value, n));
    sym.uset(SYMBOL_PACKAGE, n);
    sym.uset(SYMBOL_PROPERTY_LIST, n);
    sym.uset(SYMBOL_CONSTANT, LispObj(flags));
    sym.uset(SYMBOL_FUNCTION, cons(UNINITIALIZED, n));
    sym.uset(SYMBOL_FUNCTION_TYPE, n);
    sym.uset(SYMBOL_JUMP_TABLE, LispObj(0));
    sym.uset(SYMBOL_VAR_TABLE, LispObj(0));
}

/// Port of upstream `initSymbols`.
fn init_symbols() {
    let nil_sym = make_bootstrap_symbol("NIL");
    set_qv(Nil_Index, nil_sym);
    let t_sym = make_bootstrap_symbol("T");
    set_qv(T_Index, t_sym);
    finish_bootstrap_symbol(nil_sym, nil_sym, SYMBOL_CONSTANT_FLAG);
    finish_bootstrap_symbol(t_sym, t_sym, SYMBOL_CONSTANT_FLAG);
    let ch = make_bootstrap_symbol("CHARACTER");
    finish_bootstrap_symbol(ch, UNINITIALIZED, 0);
    set_qv(CHARACTER_Index, ch);

    let cl = qv(CL_PACKAGE_Index);
    add_symbol(cl, symbol_name(nil_sym), nil_sym, true);
    add_symbol(cl, symbol_name(t_sym), t_sym, true);
    add_symbol(cl, symbol_name(ch), ch, true);

    for i in 0..NUM_STACK_MARKERS {
        set_qv(STACK_MARKERS_Index + i, LispObj(0));
    }
    for i in 0..NUM_FOREIGN_CELLS {
        set_qv(FOREIGN_CELLS_Index + i, LispObj(0));
    }
    for i in 0..NUM_RETURN_VALUES {
        set_qv(RETURN_VALUES_Index + i, LispObj(0));
    }

    for &(idx, name) in QV_SYMBOLS {
        set_qv(idx, find_symbol(name));
    }
    set_qv(MULTIPLE_RETURN_VALUES_Index, nil());
}

/// Port of the package/symbol part of upstream `initLisp`. Called once by
/// `crate::init`.
pub(crate) fn init_packages() {
    // COMMON-LISP package; its name is filled in after NIL exists.
    let cl = heap::alloc_uvector(PACKAGE_SIZE, PACKAGE_TYPE);
    let table_size = 6007 * 3;
    // Table is created before NIL exists; zero filled entries are "empty"
    // (not strings), which is all searchSymbol checks.
    let table = heap::alloc_uvector(table_size + 1, SIMPLE_VECTOR_TYPE);
    table.uset(ARRAY_SIMPLE_VECTOR_LENGTH, LispObj::fixnum(table_size as i64));
    cl.uset(PACKAGE_CAPACITY, LispObj::fixnum(4001));
    cl.uset(PACKAGE_COUNT, LispObj::fixnum(0));
    cl.uset(PACKAGE_TABLE, table);
    set_qv(CL_PACKAGE_Index, cl);

    set_qv(SYMBOL_TABLE_COUNT_Index, LispObj::fixnum(FIRST_JUMP_TABLE_ENTRY as i64));
    set_qv(
        SYMBOL_TABLE_VAR_COUNT_Index,
        LispObj::fixnum(FIRST_SPECIAL_SYMBOL_ENTRY as i64),
    );

    init_symbols();
    let n = nil();

    set_symbol_value(qv(COMPILER_CLEANUPS_Index), n);
    set_symbol_value(qv(COMPILER_RUNTIME_Index), n);
    set_symbol_value(qv(MACROEXPAND_INLINE_Index), t());
    set_symbol_value(qv(EMBEDDED_LAMBDAS_Index), n);
    set_symbol_value(qv(TOP_LEVEL_Index), n);
    set_symbol_value(qv(PACKAGE_Index), cl);

    cl.uset(PACKAGE_NAME, string_node("COMMON-LISP"));
    for i in [
        PACKAGE_NICKNAMES,
        PACKAGE_USE_LIST,
        PACKAGE_USED_BY_LIST,
        PACKAGE_SHADOWING_SYMBOLS,
        PACKAGE_SYNC,
    ] {
        cl.uset(i, n);
    }

    let user = package_node(string_node("COMMON-LISP-USER"));
    let ccl = package_node(string_node("CORMANLISP"));
    let kw = package_node(string_node("KEYWORD"));
    set_qv(USER_PACKAGE_Index, user);
    set_qv(CORMANLISP_PACKAGE_Index, ccl);
    set_qv(KEYWORD_PACKAGE_Index, kw);
    user.uset(PACKAGE_USE_LIST, cons(ccl, n));
    ccl.uset(PACKAGE_USE_LIST, cons(cl, n));
    kw.uset(PACKAGE_USE_LIST, n);

    set_symbol_value(qv(PACKAGE_LIST_Index), heap::list(&[cl, user, ccl, kw]));

    for &(idx, name) in QV_KEYWORDS {
        set_qv(idx, find_keyword(name));
    }

    set_symbol_value(qv(COMPILER_SAVE_LAMBDAS_Index), t());
    set_symbol_value(qv(COMPILER_SAVE_LAMBDA_LISTS_Index), t());
    set_symbol_value(qv(COMPILER_SAVE_STACK_FRAME_INFO_Index), t());
    set_symbol_value(qv(COMPILER_SAVE_ENVIRONMENT_INFO_Index), t());
    set_symbol_value(qv(COMPILER_SAVE_DEBUG_INFO_Index), t());
    set_symbol_value(qv(COMPILER_SAFETY_LEVEL_Index), LispObj::fixnum(0));
    set_symbol_value(qv(GC_TIME_COUNTER_Index), LispObj::fixnum(0));
    // PORT: upstream holds a foreign pointer to QV here; no FFI yet.
    set_symbol_value(qv(SYSTEM_INTERNALS_Index), n);
    set_symbol_value(qv(INTERNAL_TIME_UNITS_PER_SECOND_Index), LispObj::fixnum(1000));
    set_symbol_value(qv(GC_TIME_UNITS_PER_SECOND_Index), LispObj::fixnum(1000));
    set_symbol_value(qv(COMPILER_LAMBDA_LIST_Index), n);
    init_more_symbols();
}

/// Rest of upstream `initLisp`: CORMANLISP package symbols, keywords and
/// the initial values of compiler and system variables.
fn init_more_symbols() {
    let n = nil();
    for &(idx, name) in &[
        (COMPILER_CODE_BUFFER_Index, "*COMPILER-CODE-BUFFER*"),
        (COMPILER_OPTIMIZE_SPEED_Index, "*COMPILER-OPTIMIZE-SPEED*"),
        (COMPILER_OPTIMIZE_SAFETY_Index, "*COMPILER-OPTIMIZE-SAFETY*"),
        (COMPILER_OPTIMIZE_SPACE_Index, "*COMPILER-OPTIMIZE-SPACE*"),
        (COMPILER_OPTIMIZE_DEBUG_Index, "*COMPILER-OPTIMIZE-DEBUG*"),
        (COMPILER_OPTIMIZE_COMPILATION_SPEED_Index, "*COMPILER-OPTIMIZE-COMPILATION-SPEED*"),
        (COMPILER_COLLECT_JUMP_TABLE_REFS_Index, "*COMPILER-COLLECT-JUMP-TABLE-REFS*"),
        (COMPILER_COLLECT_VAR_TABLE_REFS_Index, "*COMPILER-COLLECT-VAR-TABLE-REFS*"),
        (COMPILER_VARIABLE_TYPES_Index, "*COMPILER-VARIABLE-TYPES*"),
        (CORMANLISP_DIRECTORY_Index, "*CORMANLISP-DIRECTORY*"),
        (CURRENT_THREAD_ID_Index, "*CURRENT-THREAD-ID*"),
        (CURRENT_THREAD_HANDLE_Index, "*CURRENT-THREAD-HANDLE*"),
        (CURRENT_PROCESS_ID_Index, "*CURRENT-PROCESS-ID*"),
        (CURRENT_PROCESS_HANDLE_Index, "*CURRENT-PROCESS-HANDLE*"),
        (TRACE_EXCEPTIONS_Index, "*TRACE-EXCEPTIONS*"),
        (SOURCE_FILE_Index, "*SOURCE-FILE*"),
        (SOURCE_LINE_Index, "*SOURCE-LINE*"),
        (SAVE_DEBUG_INFO_Index, "*SOURCE-DEBUG-INFO*"),
        (SOURCE_LINES_Index, "*SOURCE-LINES*"),
        (CORMANLISP_SERVER_DIRECTORY_Index, "*CORMANLISP-SERVER-DIRECTORY*"),
    ] {
        set_qv(idx, find_pl_symbol(name));
    }
    for &(idx, name) in &[
        (UREF_Index, "UREF"),
        (UREF_SET_Index, "UREF-SET"),
        (COMPILER_DYNAMIC_EXTENT_VARS_Index, "*COMPILER-DYNAMIC-EXTENT-VARS*"),
        (ERROR_TRACE_Index, "*ERROR-TRACE*"),
    ] {
        set_qv(idx, find_symbol(name));
    }
    for &(idx, name) in &[
        (SYSTEM_EXCEPTION_Index, "SYSTEM-EXCEPTION"),
        (Dest_EAX_Index, "DEST-EAX"),
        (Dest_EAX_Operand_Index, "DEST-EAX-OPERAND"),
        (Dest_Stack_Index, "DEST-STACK"),
        (Dest_Zero_Flag_Index, "DEST-ZERO-FLAG"),
    ] {
        set_qv(idx, find_keyword(name));
    }
    // upstream createSymbolTableEntry: per-thread special variables
    for idx in [
        COMPILER_CLEANUPS_Index,
        EMBEDDED_LAMBDAS_Index,
        ENV_COUNTER_Index,
        LAMBDA_SPECIAL_VARS_Index,
        LAMBDA_DECLARATIONS_Index,
        LAMBDA_SPECIAL_DECS_Index,
        COMPILER_CODE_BUFFER_Index,
        COMPILER_FUNCTION_NAME_Index,
        COMPILER_USES_ENV_Index,
        COMPILER_OPTIMIZE_SPEED_Index,
        COMPILER_OPTIMIZE_SAFETY_Index,
        COMPILER_OPTIMIZE_SPACE_Index,
        COMPILER_OPTIMIZE_DEBUG_Index,
        COMPILER_OPTIMIZE_COMPILATION_SPEED_Index,
        COMPILER_RUNTIME_Index,
        COMPILER_VARIABLE_TYPES_Index,
        COMPILER_DYNAMIC_EXTENT_VARS_Index,
        SOURCE_FILE_Index,
        SOURCE_LINE_Index,
        SAVE_DEBUG_INFO_Index,
        SOURCE_LINES_Index,
        ERROR_OUTPUT_Index,
        COMPILER_COLLECT_JUMP_TABLE_REFS_Index,
        COMPILER_COLLECT_VAR_TABLE_REFS_Index,
        CODE_JUMP_TABLE_REFS_Index,
        CODE_ENV_TABLE_REFS_Index,
        CODE_VAR_TABLE_REFS_Index,
        CURRENT_THREAD_ID_Index,
        CURRENT_THREAD_HANDLE_Index,
        CURRENT_PROCESS_ID_Index,
        CURRENT_PROCESS_HANDLE_Index,
        COMPRESS_IMG_Index,
        COMPILED_SPECIALS_Index,
        COMPILER_FRAME_INFO_Index,
        COMPILER_SAVE_STACK_FRAME_INFO_Index,
        COMPILER_ENVIRONMENT_Index,
        COMPILER_WARN_ON_UNUSED_VAR_Index,
        COMPILER_WARN_ON_ASSUMED_SPECIAL_Index,
        COLLECT_LEXICAL_MACROS_Index,
        LEXICAL_MACROS_Index,
        COLLECT_LEXICAL_SYMBOL_MACROS_Index,
        LEXICAL_SYMBOL_MACROS_Index,
        COMPILER_LAMBDA_LIST_Index,
        COMPILING_LAMBDA_Index,
        FINALIZATION_PENDING_Index,
        CAPTURED_LEXICAL_CALLBACK_Index,
        LOAD_TIME_VALUES_Index,
        PAGE_TABLE_Index,
        DISASSEMBLY_OUTPUT_BUF_Index,
        PACKAGE_Index,
        READTABLE_Index,
    ] {
        set_special_symbol(qv(idx));
    }
    for idx in [
        COMPILER_CODE_BUFFER_Index,
        COMPILER_OPTIMIZE_SPACE_Index,
        COMPILER_OPTIMIZE_COMPILATION_SPEED_Index,
        COMPILER_VARIABLE_TYPES_Index,
        COMPILER_DYNAMIC_EXTENT_VARS_Index,
        COMPILER_WARN_ON_DYNAMIC_RETURN_Index,
        SAVE_IMAGE_CLEANUP_FUNCS_Index,
        LOAD_IMAGE_RESTORE_FUNCS_Index,
        COMPILER_COLLECT_JUMP_TABLE_REFS_Index,
        COMPILER_COLLECT_VAR_TABLE_REFS_Index,
        CODE_JUMP_TABLE_REFS_Index,
        CODE_VAR_TABLE_REFS_Index,
        CODE_ENV_TABLE_REFS_Index,
        HEAP_CHECKING_Index,
        ERROR_TRACE_Index,
        TRACE_EXCEPTIONS_Index,
        SOURCE_FILE_Index,
        SOURCE_LINE_Index,
        SAVE_DEBUG_INFO_Index,
        SOURCE_LINES_Index,
        LAMBDA_SPECIAL_DECS_Index,
        APPEND_REFS_TO_CODE_Index,
        CURRENT_THREAD_ID_Index,
        CURRENT_THREAD_HANDLE_Index,
        CURRENT_PROCESS_HANDLE_Index,
        COMPRESS_IMG_Index,
        COMPILED_SPECIALS_Index,
        COMPILER_FRAME_INFO_Index,
        COMPILER_ENVIRONMENT_Index,
        COMPILER_WARN_ON_UNUSED_VAR_Index,
        FINALIZATION_PENDING_Index,
        UNDEFINED_FUNCTIONS_Index,
        COLLECT_LEXICAL_MACROS_Index,
        LEXICAL_MACROS_Index,
        COLLECT_LEXICAL_SYMBOL_MACROS_Index,
        LEXICAL_SYMBOL_MACROS_Index,
        LAMBDA_SPECIAL_VARS_Index,
        COMPILING_LAMBDA_Index,
        CAPTURED_LEXICAL_CALLBACK_Index,
        LOAD_TIME_VALUES_Index,
    ] {
        set_symbol_value(qv(idx), n);
    }
    set_symbol_value(qv(COMPILER_OPTIMIZE_SPEED_Index), LispObj::fixnum(0));
    set_symbol_value(qv(COMPILER_OPTIMIZE_SAFETY_Index), LispObj::fixnum(3));
    set_symbol_value(qv(COMPILER_OPTIMIZE_DEBUG_Index), LispObj::fixnum(3));
    set_symbol_value(qv(COMPILER_WARN_ON_UNDEFINED_FUNCTION_Index), t());
    set_symbol_value(qv(COMPILER_WARN_ON_ASSUMED_SPECIAL_Index), t());
    set_symbol_value(qv(INVALID_OBJECT_COUNT_Index), LispObj::fixnum(0));
    set_symbol_value(qv(READ_DEFAULT_FLOAT_FORMAT_Index), qv(SINGLE_FLOAT_Index));
    set_symbol_value(
        qv(CURRENT_PROCESS_ID_Index),
        LispObj::fixnum(std::process::id() as i64),
    );
    let dir = std::env::current_dir()
        .map(|p| format!("{}/", p.display()))
        .unwrap_or_else(|_| "./".to_string());
    set_symbol_value(qv(CORMANLISP_DIRECTORY_Index), string_node(&dir));
    set_symbol_value(qv(CORMANLISP_SERVER_DIRECTORY_Index), string_node(&dir));
    for idx in [HEAP_0_GC_ID_Index, HEAP_1_GC_ID_Index, HEAP_2_GC_ID_Index] {
        set_symbol_value(qv(idx), LispObj::fixnum(0));
        set_special_symbol(qv(idx));
    }
}

/// Intern `name` and give it a value (convenience for kernel setup).
pub fn defvar(name: &str, value: LispObj) -> LispObj {
    let s = find_symbol(name);
    set_symbol_value(s, value);
    set_special_symbol(s);
    s
}
