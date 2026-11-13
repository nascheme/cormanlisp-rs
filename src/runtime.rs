//! Runtime glue between compiled Lisp code and Rust: call stubs, multiple
//! values, the dynamic frame chain (catch, unwind-protect, special
//! bindings) and throw. Replaces upstream's `LispCall*` thunks,
//! `pushCatcher`/`popCatcher` and the throw code in `Lisp.cpp`.
//!
//! Lisp calling convention (x86-64):
//! - R15 = QV pointer (callee-saved in SysV, so it survives Rust calls).
//! - Arguments are stored on the stack in ascending address order: at the
//!   `call`, `[rsp]` = arg 0, `[rsp+8]` = arg 1, ... The caller pops them.
//! - RCX = argument count (raw) on entry; number of values (raw) on return.
//! - RAX = function object on entry (to the funcall stub), first value on
//!   return. If the value count is not 1, all values are also stored in
//!   QV `RETURN_VALUES` and RAX holds the first one (NIL if none).
//! - R13 = closure environment on entry to compiled code.
//! - All other registers are caller-saved; RBP is the frame pointer.
//!
//! Dynamic frames live on the Lisp stack and are linked from QV
//! `PORT_DYN_FRAME`. Layout (cells from the frame address):
//!   0 previous frame (raw address), 1 kind (fixnum), 2 tag / symbol,
//!   3 landing address (raw) / saved value, 4 saved RBP (raw).
//! A throw restores RSP to the frame address (catch) and jumps to its
//! landing code. Rust frames between a catcher and a throw are discarded
//! without unwinding: Rust code that may call Lisp must not rely on `Drop`.

use std::sync::atomic::{AtomicUsize, Ordering};

use iced_x86::IcedError;
use iced_x86::code_asm::CodeAssembler;

use crate::error::lisp_error;
use crate::heap::{self, cons};
use crate::object::*;
use crate::prims;
use crate::qv::{self, nil, qv, set_qv};
use crate::qv_index::*;
use crate::symbols;

pub const DF_PREV: usize = 0;
pub const DF_KIND: usize = 1;
pub const DF_TAG: usize = 2;
pub const DF_LANDING: usize = 3;
pub const DF_RBP: usize = 4;
pub const DF_CELLS: usize = 5;

pub const DF_CATCH: i64 = 1;
pub const DF_UNWIND_PROTECT: i64 = 2;
pub const DF_SPECIAL: i64 = 3;
/// Garbage collection inhibited (made from Rust by `without_gc`).
pub const DF_GC_INHIBIT: i64 = 4;

/// Compiled code objects (`COMPILED_CODE_TYPE` uvectors). PORT: own layout.
/// Cells: 1 entry offset (bytes from the uvector start, fixnum),
/// 2 literal area offset (bytes, fixnum), 3 number of literals (fixnum),
/// 4 name, 5 info. Machine code starts at `CODE_START`; the literal area
/// (8-byte tagged cells, possibly unaligned) follows the instructions.
pub const CODE_ENTRY_OFFSET: usize = 1;
pub const CODE_LITERALS_OFFSET: usize = 2;
pub const CODE_NUM_LITERALS: usize = 3;
pub const CODE_NAME: usize = 4;
pub const CODE_INFO: usize = 5;
pub const CODE_START: usize = 48;

// ---------------------------------------------------------------------------
// Rust helpers called from compiled code through the helper stub.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Helper {
    Cons,
    MakeRest,
    MakeClosure,
    WrongArgs,
    UnboundVariable,
    NotList,
    SaveValues,
    RestoreValues,
    Throw,
    TakePending,
    ContinueThrow,
    GetKey,
    CheckKeys,
    FreshTag,
    MvApply,
    SymbolFunction,
    Add2,
    Sub2,
    NumEq2,
    Lt2,
    Gt2,
    Le2,
    Ge2,
}

fn helper_addresses() -> Vec<(Helper, usize)> {
    use Helper::*;
    vec![
        (Cons, h_cons as *const () as usize),
        (MakeRest, h_make_rest as *const () as usize),
        (MakeClosure, h_make_closure as *const () as usize),
        (WrongArgs, h_wrong_args as *const () as usize),
        (UnboundVariable, h_unbound_variable as *const () as usize),
        (NotList, h_not_list as *const () as usize),
        (SaveValues, h_save_values as *const () as usize),
        (RestoreValues, h_restore_values as *const () as usize),
        (Throw, h_throw as *const () as usize),
        (TakePending, h_take_pending as *const () as usize),
        (ContinueThrow, h_continue_throw as *const () as usize),
        (GetKey, h_get_key as *const () as usize),
        (CheckKeys, h_check_keys as *const () as usize),
        (FreshTag, h_fresh_tag as *const () as usize),
        (MvApply, h_mv_apply as *const () as usize),
        (SymbolFunction, h_symbol_function as *const () as usize),
        (Add2, h_add2 as *const () as usize),
        (Sub2, h_sub2 as *const () as usize),
        (NumEq2, h_numeq2 as *const () as usize),
        (Lt2, h_lt2 as *const () as usize),
        (Gt2, h_gt2 as *const () as usize),
        (Le2, h_le2 as *const () as usize),
        (Ge2, h_ge2 as *const () as usize),
    ]
}

/// Helper names in table order (part of the image kernel id).
pub fn helper_names() -> Vec<String> {
    helper_addresses().iter().map(|(h, _)| format!("{h:?}")).collect()
}

extern "C" fn h_cons(a: LispObj, b: LispObj) -> LispObj {
    cons(a, b)
}

/// List of `n` arguments starting at `p` (ascending addresses).
extern "C" fn h_make_rest(p: *const LispObj, n: isize) -> LispObj {
    let mut r = nil();
    for i in (0..n.max(0) as usize).rev() {
        r = cons(unsafe { *p.add(i) }, r);
    }
    r
}

/// Closure over the `k` boxes at `p` (pushed in order, so box 0 is at the
/// highest address).
extern "C" fn h_make_closure(code: LispObj, p: *const LispObj, k: usize) -> LispObj {
    let env = heap::vector_node(k);
    for i in 0..k {
        env.svset(i, unsafe { *p.add(k - 1 - i) });
    }
    make_function(code, env)
}

extern "C" fn h_wrong_args(nargs: usize, name: LispObj) -> LispObj {
    let n = crate::printer::write_to_string(name);
    lisp_error(format!(
        "Wrong number of arguments to {n}: number passed = {nargs}"
    ))
}

extern "C" fn h_unbound_variable(sym: LispObj) -> LispObj {
    let n = crate::printer::write_to_string(sym);
    lisp_error(format!("The variable {n} is unbound"))
}

extern "C" fn h_not_list(x: LispObj) -> LispObj {
    let n = crate::printer::write_to_string(x);
    lisp_error(format!("Not a list: {n}"))
}

extern "C" fn h_save_values(rax: LispObj, rcx: usize) -> LispObj {
    values_to_list(rax, rcx)
}

extern "C" fn h_restore_values(list: LispObj) -> LispObj {
    set_values_from_list(list)
}

extern "C" fn h_throw(tag: LispObj, values: LispObj) -> LispObj {
    throw_to(tag, values)
}

extern "C" fn h_take_pending() -> LispObj {
    let p = qv(PORT_THROW_PENDING_Index);
    set_qv(PORT_THROW_PENDING_Index, nil());
    p
}

extern "C" fn h_continue_throw(pending: LispObj) -> LispObj {
    let target = pending.car().raw();
    let rest = pending.cdr();
    unwind_to(target, rest.car(), rest.cdr())
}

/// Value of keyword `key` in the plist `rest`, or `SYMBOL_UNBOUND`.
extern "C" fn h_get_key(rest: LispObj, key: LispObj) -> LispObj {
    let mut p = rest;
    while p.is_cons() && p.cdr().is_cons() {
        if p.car() == key {
            return p.cdr().car();
        }
        p = p.cdr().cdr();
    }
    SYMBOL_UNBOUND
}

/// Check a keyword argument list against the list of allowed keywords.
extern "C" fn h_check_keys(rest: LispObj, allowed: LispObj, name: LispObj) -> LispObj {
    let aok = symbols::find_keyword("ALLOW-OTHER-KEYS");
    let mut p = rest;
    let mut len = 0;
    while p.is_cons() {
        len += 1;
        p = p.cdr();
    }
    if len % 2 != 0 {
        let n = crate::printer::write_to_string(name);
        lisp_error(format!("Odd number of keyword arguments to {n}"));
    }
    if h_get_key(rest, aok) != SYMBOL_UNBOUND && h_get_key(rest, aok) != nil() {
        return nil();
    }
    let mut p = rest;
    while p.is_cons() {
        let k = p.car();
        if k != aok && !list_memq(k, allowed) {
            let n = crate::printer::write_to_string(name);
            let ks = crate::printer::write_to_string(k);
            lisp_error(format!("Invalid keyword argument {ks} to {n}"));
        }
        p = p.cdr().cdr();
    }
    nil()
}

fn list_memq(x: LispObj, mut l: LispObj) -> bool {
    while l.is_cons() {
        if l.car() == x {
            return true;
        }
        l = l.cdr();
    }
    false
}

extern "C" fn h_fresh_tag() -> LispObj {
    cons(nil(), nil())
}

/// MULTIPLE-VALUE-CALL: `k` value lists at `p` (pushed in order).
extern "C" fn h_mv_apply(f: LispObj, p: *const LispObj, k: usize) -> LispObj {
    let mut args = Vec::new();
    for i in (0..k).rev() {
        let mut l = unsafe { *p.add(i) };
        while l.is_cons() {
            args.push(l.car());
            l = l.cdr();
        }
    }
    lisp_call(f, &args)
}

extern "C" fn h_symbol_function(sym: LispObj) -> LispObj {
    prims::symbol_function_checked(sym)
}

extern "C" fn h_add2(a: LispObj, b: LispObj) -> LispObj {
    crate::numbers::add(a, b)
}
extern "C" fn h_sub2(a: LispObj, b: LispObj) -> LispObj {
    crate::numbers::sub(a, b)
}
extern "C" fn h_numeq2(a: LispObj, b: LispObj) -> LispObj {
    qv::lbool(crate::numbers::compare(a, b) == std::cmp::Ordering::Equal)
}
extern "C" fn h_lt2(a: LispObj, b: LispObj) -> LispObj {
    qv::lbool(crate::numbers::compare(a, b) == std::cmp::Ordering::Less)
}
extern "C" fn h_gt2(a: LispObj, b: LispObj) -> LispObj {
    qv::lbool(crate::numbers::compare(a, b) == std::cmp::Ordering::Greater)
}
extern "C" fn h_le2(a: LispObj, b: LispObj) -> LispObj {
    qv::lbool(crate::numbers::compare(a, b) != std::cmp::Ordering::Greater)
}
extern "C" fn h_ge2(a: LispObj, b: LispObj) -> LispObj {
    qv::lbool(crate::numbers::compare(a, b) != std::cmp::Ordering::Less)
}

// ---------------------------------------------------------------------------
// Errors raised from the stubs

extern "C" fn stub_not_a_function(x: LispObj) -> LispObj {
    let n = crate::printer::write_to_string(x);
    lisp_error(format!("Not a function: {n}"))
}

/// The discriminating function of standard generic function `x` (a CLOS
/// instance). Upstream does this in the DEFASM FUNCALL of clos.lisp.
pub fn generic_function_function(x: LispObj) -> Option<LispObj> {
    if !x.is_uvector_of(CLOS_INSTANCE_TYPE) {
        return None;
    }
    let sym = qv(PORT_STD_GF_SYM_Index);
    if !symbols::is_bound(sym) || x.uref(CLOS_INSTANCE_CLASS) != symbols::symbol_value(sym) {
        return None;
    }
    let f = x
        .uref(CLOS_INSTANCE_SLOTS)
        .uref(2 + SLOT_LOCATION_GF_DISCRIMINATING_FUNCTION);
    f.is_function().then_some(f)
}

/// A function object or a standard generic function.
pub fn is_funcallable(x: LispObj) -> bool {
    x.is_function() || generic_function_function(x).is_some()
}

extern "C" fn stub_instance_function(x: LispObj) -> LispObj {
    match generic_function_function(x) {
        Some(f) => f,
        None => stub_not_a_function(x),
    }
}

extern "C" fn stub_undefined_function(sym: LispObj) -> LispObj {
    let handler = symbols::find_symbol("SIGNAL-UNDEFINED-FUNCTION");
    if symbols::symbol_function_type(handler) == qv(FUNCTION_Index) {
        lisp_call(handler, &[sym]);
    }
    let n = crate::printer::write_to_string(sym);
    lisp_error(format!("The function {n} is not defined"))
}

/// Called (via the funcall stub) for kernel functions.
extern "C" fn prim_dispatch(idx: usize, args: *const LispObj, n: usize) -> LispObj {
    qv::set_num_return_values(1);
    let a = unsafe { std::slice::from_raw_parts(args, n) };
    (prims::PRIMS[idx].func)(a)
}

// ---------------------------------------------------------------------------
// Stubs (generated at startup into their own executable mapping)

static LISP_CALL_ENTRY: AtomicUsize = AtomicUsize::new(0);
static CATCH_ENTRY: AtomicUsize = AtomicUsize::new(0);
static JUMP_ENTRY: AtomicUsize = AtomicUsize::new(0);

const STUB_BYTES: usize = 4096;

fn qvd(i: usize) -> i32 {
    (i * CELL) as i32
}

fn fix(n: i64) -> i32 {
    (n << 3) as i32
}

fn gen_stubs(base: u64) -> Result<(Vec<u8>, [u64; 5]), IcedError> {
    use iced_x86::code_asm::*;
    let qv_addr = qv::qv_base() as u64;
    let mut a = CodeAssembler::new(64)?;
    let mut funcall = a.create_label();
    let mut helper = a.create_label();
    let mut lcall = a.create_label();
    let mut catch = a.create_label();
    let mut jump = a.create_label();

    // ---- funcall stub: RAX = function, RCX = nargs, args at [rsp+8].
    {
        let mut again = a.create_label();
        let mut notfun = a.create_label();
        let mut fun = a.create_label();
        let mut kfun = a.create_label();
        let mut undef = a.create_label();
        let mut inst = a.create_label();
        a.set_label(&mut funcall)?;
        a.nop()?;
        a.set_label(&mut again)?;
        a.mov(edx, eax)?;
        a.and(edx, 7)?;
        a.cmp(edx, UVECTOR_TAG as i32)?;
        a.jne(notfun)?;
        a.mov(rdx, qword_ptr(rax - 5))?;
        a.shr(edx, 3)?;
        a.and(edx, 0x1f)?;
        a.cmp(edx, FUNCTION_TYPE as i32)?;
        a.je(fun)?;
        a.cmp(edx, KFUNCTION_TYPE as i32)?;
        a.je(kfun)?;
        a.cmp(edx, CLOS_INSTANCE_TYPE as i32)?;
        a.je(inst)?;
        a.cmp(edx, SYMBOL_TYPE as i32)?;
        a.jne(notfun)?;
        a.mov(rdx, qword_ptr(rax + (SYMBOL_FUNCTION_TYPE * CELL) as i32 - 5))?;
        a.cmp(rdx, qword_ptr(r15 + qvd(FUNCTION_Index)))?;
        a.jne(undef)?;
        a.mov(rax, qword_ptr(rax + (SYMBOL_FUNCTION * CELL) as i32 - 5))?;
        a.mov(rax, qword_ptr(rax - 4))?;
        a.jmp(again)?;

        a.set_label(&mut fun)?;
        a.mov(r13, qword_ptr(rax + (FUNCTION_ENVIRONMENT * CELL) as i32 - 5))?;
        a.mov(rdx, qword_ptr(rax + (FUNCTION_ADDRESS * CELL) as i32 - 5))?;
        a.mov(r11, qword_ptr(rdx + (CODE_ENTRY_OFFSET * CELL) as i32 - 5))?;
        a.sar(r11, 3)?;
        a.lea(r11, qword_ptr(rdx + r11 - 5))?;
        a.jmp(r11)?;

        a.set_label(&mut kfun)?;
        a.push(rbp)?;
        a.mov(rbp, rsp)?;
        a.mov(rdi, qword_ptr(rax + (FUNCTION_ADDRESS * CELL) as i32 - 5))?;
        a.sar(rdi, 3)?;
        a.lea(rsi, qword_ptr(rbp + 16))?;
        a.mov(rdx, rcx)?;
        a.and(rsp, -16)?;
        a.mov(r11, prim_dispatch as *const () as usize as u64)?;
        a.call(r11)?;
        a.mov(rcx, qword_ptr(r15 + qvd(PORT_NUM_RETURN_VALUES_Index)))?;
        a.sar(rcx, 3)?;
        a.mov(rsp, rbp)?;
        a.pop(rbp)?;
        a.ret()?;

        // A CLOS instance: call its discriminating function (or error).
        a.set_label(&mut inst)?;
        a.push(rbp)?;
        a.mov(rbp, rsp)?;
        a.push(rcx)?;
        a.mov(rdi, rax)?;
        a.and(rsp, -16)?;
        a.mov(r11, stub_instance_function as *const () as usize as u64)?;
        a.call(r11)?;
        a.mov(rcx, qword_ptr(rbp - 8))?;
        a.mov(rsp, rbp)?;
        a.pop(rbp)?;
        a.jmp(again)?;

        a.set_label(&mut undef)?;
        a.mov(rdi, rax)?;
        a.and(rsp, -16)?;
        a.mov(r11, stub_undefined_function as *const () as usize as u64)?;
        a.call(r11)?;
        a.ud2()?;

        a.set_label(&mut notfun)?;
        a.mov(rdi, rax)?;
        a.and(rsp, -16)?;
        a.mov(r11, stub_not_a_function as *const () as usize as u64)?;
        a.call(r11)?;
        a.ud2()?;
    }

    // ---- helper stub: RAX = helper index, SysV args already in place.
    a.set_label(&mut helper)?;
    a.push(rbp)?;
    a.mov(rbp, rsp)?;
    a.and(rsp, -16)?;
    a.call(qword_ptr(r15 + rax * 8 + qvd(PORT_HELPERS_Index)))?;
    a.mov(rsp, rbp)?;
    a.pop(rbp)?;
    a.ret()?;

    // ---- lisp_call entry (SysV): rdi = function, rsi = args, rdx = nargs.
    {
        let mut lp = a.create_label();
        let mut done = a.create_label();
        a.set_label(&mut lcall)?;
        save_callee_saved(&mut a)?;
        a.mov(r15, qv_addr)?;
        a.mov(rax, rdi)?;
        a.mov(rcx, rdx)?;
        a.lea(r11, qword_ptr(rdx * 8))?;
        a.sub(rsp, r11)?;
        a.and(rsp, -16)?;
        a.xor(r10, r10)?;
        a.set_label(&mut lp)?;
        a.cmp(r10, rcx)?;
        a.jae(done)?;
        a.mov(r11, qword_ptr(rsi + r10 * 8))?;
        a.mov(qword_ptr(rsp + r10 * 8), r11)?;
        a.inc(r10)?;
        a.jmp(lp)?;
        a.set_label(&mut done)?;
        a.call(qword_ptr(r15 + qvd(PORT_FUNCALL_STUB_Index)))?;
        a.shl(rcx, 3)?;
        a.mov(qword_ptr(r15 + qvd(PORT_NUM_RETURN_VALUES_Index)), rcx)?;
        restore_callee_saved(&mut a)?;
    }

    // ---- catch entry (SysV): rdi = tag, rsi = fn(data), rdx = data.
    // Returns 0 normally, 1 if a throw to the tag landed here.
    {
        let mut landing = a.create_label();
        let mut done = a.create_label();
        a.set_label(&mut catch)?;
        save_callee_saved(&mut a)?;
        a.mov(r15, qv_addr)?;
        a.push(rbp)?;
        a.lea(r11, qword_ptr(landing))?;
        a.push(r11)?;
        a.push(rdi)?;
        a.push(fix(DF_CATCH))?;
        a.push(qword_ptr(r15 + qvd(PORT_DYN_FRAME_Index)))?;
        a.mov(qword_ptr(r15 + qvd(PORT_DYN_FRAME_Index)), rsp)?;
        a.mov(rdi, rdx)?;
        a.call(rsi)?;
        a.mov(rdx, qword_ptr(rsp))?;
        a.mov(qword_ptr(r15 + qvd(PORT_DYN_FRAME_Index)), rdx)?;
        a.xor(eax, eax)?;
        a.jmp(done)?;
        a.set_label(&mut landing)?;
        a.mov(rdx, qword_ptr(rsp))?;
        a.mov(qword_ptr(r15 + qvd(PORT_DYN_FRAME_Index)), rdx)?;
        a.mov(eax, 1)?;
        a.set_label(&mut done)?;
        restore_callee_saved(&mut a)?;
    }

    // ---- jump (SysV): rdi = new rsp, rsi = new rbp, rdx = target.
    a.set_label(&mut jump)?;
    a.mov(rsp, rdi)?;
    a.mov(rbp, rsi)?;
    a.mov(r15, qv_addr)?;
    a.jmp(rdx)?;

    let r = a.assemble_options(base, iced_x86::BlockEncoderOptions::RETURN_NEW_INSTRUCTION_OFFSETS)?;
    let addrs = [
        r.label_ip(&funcall)?,
        r.label_ip(&helper)?,
        r.label_ip(&lcall)?,
        r.label_ip(&catch)?,
        r.label_ip(&jump)?,
    ];
    Ok((r.inner.code_buffer, addrs))
}

/// push rbp; mov rbp,rsp; push rbx, r12-r15. RSP ends 16-byte aligned
/// (rbp - 40, rbp aligned) so 5-cell frames keep calls aligned.
fn save_callee_saved(a: &mut CodeAssembler) -> Result<(), IcedError> {
    use iced_x86::code_asm::*;
    a.push(rbp)?;
    a.mov(rbp, rsp)?;
    a.push(rbx)?;
    a.push(r12)?;
    a.push(r13)?;
    a.push(r14)?;
    a.push(r15)?;
    Ok(())
}

fn restore_callee_saved(a: &mut CodeAssembler) -> Result<(), IcedError> {
    use iced_x86::code_asm::*;
    a.lea(rsp, qword_ptr(rbp - 40))?;
    a.pop(r15)?;
    a.pop(r14)?;
    a.pop(r13)?;
    a.pop(r12)?;
    a.pop(rbx)?;
    a.pop(rbp)?;
    a.ret()?;
    Ok(())
}

/// Generate the stubs and fill the QV slots that compiled code uses.
/// Called at startup (and after loading an image).
pub(crate) fn init_runtime() {
    let mem = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            STUB_BYTES,
            libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    assert!(mem != libc::MAP_FAILED, "cannot map stub area");
    let base = mem as u64;
    let (code, addrs) = gen_stubs(base).expect("stub generation");
    assert!(code.len() <= STUB_BYTES);
    unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), mem as *mut u8, code.len()) };
    set_qv(PORT_FUNCALL_STUB_Index, LispObj(addrs[0] as usize));
    set_qv(PORT_HELPER_STUB_Index, LispObj(addrs[1] as usize));
    LISP_CALL_ENTRY.store(addrs[2] as usize, Ordering::Relaxed);
    CATCH_ENTRY.store(addrs[3] as usize, Ordering::Relaxed);
    JUMP_ENTRY.store(addrs[4] as usize, Ordering::Relaxed);
    let hs = helper_addresses();
    assert!(hs.len() <= PORT_MAX_HELPERS);
    for (i, (h, addr)) in hs.into_iter().enumerate() {
        assert_eq!(h as usize, i, "helper table out of order");
        set_qv(PORT_HELPERS_Index + i, LispObj(addr));
    }
    set_qv(PORT_DYN_FRAME_Index, LispObj(0));
    set_qv(PORT_THROW_PENDING_Index, nil());
}

// ---------------------------------------------------------------------------
// Function objects and calls

/// A function object for compiled code object `code` closed over `env`.
pub fn make_function(code: LispObj, env: LispObj) -> LispObj {
    let f = heap::alloc_uvector(FUNCTION_SIZE, FUNCTION_TYPE);
    f.uset(FUNCTION_ENVIRONMENT, env);
    f.uset(FUNCTION_ADDRESS, code);
    f
}

/// Call a Lisp function (or symbol) from Rust. The number of values is left
/// in QV (see `qv::num_return_values`), extra values in RETURN_VALUES.
pub fn lisp_call(f: LispObj, args: &[LispObj]) -> LispObj {
    let entry = LISP_CALL_ENTRY.load(Ordering::Relaxed);
    let fp: extern "C" fn(LispObj, *const LispObj, usize) -> LispObj =
        unsafe { std::mem::transmute(entry) };
    fp(f, args.as_ptr(), args.len())
}

struct CatchData<F, R> {
    f: Option<F>,
    r: Option<R>,
}

extern "C" fn catch_tramp<F: FnOnce() -> R, R>(d: *mut CatchData<F, R>) {
    let d = unsafe { &mut *d };
    let f = d.f.take().unwrap();
    d.r = Some(f());
}

/// A throw landed in `rust_catch`; the thrown values are in QV.
#[derive(Debug, Clone, Copy)]
pub struct Thrown;

/// Run `f` with a catcher for `tag` established. Returns `Err(Thrown)` if
/// a throw to `tag` landed.
pub fn rust_catch<R>(tag: LispObj, f: impl FnOnce() -> R) -> Result<R, Thrown> {
    fn run<F: FnOnce() -> R, R>(tag: LispObj, f: F) -> Result<R, Thrown> {
        let mut d = CatchData::<F, R> { f: Some(f), r: None };
        let entry = CATCH_ENTRY.load(Ordering::Relaxed);
        let fp: extern "C" fn(LispObj, usize, *mut CatchData<F, R>) -> usize =
            unsafe { std::mem::transmute(entry) };
        let flag = fp(tag, catch_tramp::<F, R> as *const () as usize, &mut d);
        if flag == 0 {
            Ok(d.r.take().unwrap())
        } else {
            Err(Thrown)
        }
    }
    run(tag, f)
}

// ---------------------------------------------------------------------------
// Multiple values

/// Return `vals` as the values of a primitive. Returns the first value.
pub fn set_values(vals: &[LispObj]) -> LispObj {
    if vals.len() > NUM_RETURN_VALUES {
        lisp_error("Too many return values");
    }
    for (i, &v) in vals.iter().enumerate() {
        set_qv(RETURN_VALUES_Index + i, v);
    }
    qv::set_num_return_values(vals.len());
    vals.first().copied().unwrap_or_else(nil)
}

/// The values described by RAX/RCX (and QV) as a list.
pub fn values_to_list(rax: LispObj, n: usize) -> LispObj {
    if n == 1 {
        return cons(rax, nil());
    }
    let mut r = nil();
    for i in (0..n).rev() {
        r = cons(qv(RETURN_VALUES_Index + i), r);
    }
    r
}

/// Set the current values from a list. Returns the first value.
pub fn set_values_from_list(list: LispObj) -> LispObj {
    let mut v = Vec::new();
    let mut p = list;
    while p.is_cons() {
        v.push(p.car());
        p = p.cdr();
    }
    set_values(&v)
}

// ---------------------------------------------------------------------------
// Dynamic frames and throw

#[inline]
fn df(f: usize, i: usize) -> LispObj {
    unsafe { *((f + i * CELL) as *const LispObj) }
}

fn dyn_head() -> usize {
    qv(PORT_DYN_FRAME_Index).raw()
}

fn set_dyn_head(f: usize) {
    set_qv(PORT_DYN_FRAME_Index, LispObj(f));
}

/// Innermost catch frame for `tag`, or 0.
pub fn find_catcher(tag: LispObj) -> usize {
    let mut f = dyn_head();
    while f != 0 {
        if df(f, DF_KIND).integer() == DF_CATCH && df(f, DF_TAG) == tag {
            return f;
        }
        f = df(f, DF_PREV).raw();
    }
    0
}

/// Throw `values` (a list) to the innermost catcher of `tag`.
pub fn throw_to(tag: LispObj, values: LispObj) -> ! {
    let target = find_catcher(tag);
    if target == 0 {
        let t = crate::printer::write_to_string(tag);
        lisp_error(format!("Throw to tag {t} with no catcher"));
    }
    unwind_to(target, tag, values)
}

/// Unwind the dynamic frames down to catch frame `target`, running
/// unwind-protect cleanups, then land there.
fn unwind_to(target: usize, tag: LispObj, values: LispObj) -> ! {
    loop {
        let f = dyn_head();
        if f == 0 {
            eprintln!("internal error: throw target vanished");
            std::process::abort();
        }
        if f == target {
            set_values_from_list(values);
            if qv::num_return_values() == 0 {
                set_qv(RETURN_VALUES_Index, nil());
            }
            jump(f, df(f, DF_RBP).raw(), df(f, DF_LANDING).raw());
        }
        let prev = df(f, DF_PREV).raw();
        match df(f, DF_KIND).integer() {
            DF_SPECIAL => {
                let sym = df(f, DF_TAG);
                symbols::set_symbol_value(sym, df(f, DF_LANDING));
                set_dyn_head(prev);
            }
            DF_CATCH => set_dyn_head(prev),
            DF_GC_INHIBIT => {
                heap::inhibit_gc(-1);
                set_dyn_head(prev);
            }
            DF_UNWIND_PROTECT => {
                set_dyn_head(prev);
                let pending = cons(LispObj(target), cons(tag, values));
                set_qv(PORT_THROW_PENDING_Index, pending);
                jump(f + DF_CELLS * CELL, df(f, DF_RBP).raw(), df(f, DF_LANDING).raw());
            }
            k => {
                eprintln!("internal error: bad dynamic frame kind {k}");
                std::process::abort();
            }
        }
    }
}

fn jump(rsp: usize, rbp: usize, target: usize) -> ! {
    let entry = JUMP_ENTRY.load(Ordering::Relaxed);
    let fp: extern "C" fn(usize, usize, usize) -> ! = unsafe { std::mem::transmute(entry) };
    fp(rsp, rbp, target)
}

/// Run `f` with garbage collection inhibited. A dynamic frame records the
/// inhibition so that a throw out of `f` (which skips Rust frames without
/// unwinding) re-enables collection.
pub fn without_gc<R>(f: impl FnOnce() -> R) -> R {
    let mut frame = [LispObj(0); DF_CELLS];
    frame[DF_PREV] = LispObj(dyn_head());
    frame[DF_KIND] = LispObj::fixnum(DF_GC_INHIBIT);
    let addr = std::hint::black_box(frame.as_mut_ptr()) as usize;
    set_dyn_head(addr);
    heap::inhibit_gc(1);
    let r = f();
    heap::inhibit_gc(-1);
    debug_assert_eq!(dyn_head(), addr);
    set_dyn_head(df(addr, DF_PREV).raw());
    std::hint::black_box(&mut frame);
    r
}

/// Number of frames on the dynamic chain (for tests and sanity checks).
pub fn dyn_depth() -> usize {
    let mut n = 0;
    let mut f = dyn_head();
    while f != 0 {
        n += 1;
        f = df(f, DF_PREV).raw();
    }
    n
}

/// `(%PROGV symbols values thunk)`: the frontend compiles PROGV into a
/// call of this with the body as a thunk. Binds each symbol in a special
/// frame on this function's (Rust) stack, as `without_gc` does, so a throw
/// out of the body restores the values; extra symbols become unbound.
pub fn progv_prim(a: &[LispObj]) -> LispObj {
    prims::check_args(a, 3, 3);
    progv_bind(a[0], a[1], a[2])
}

fn progv_bind(syms: LispObj, vals: LispObj, thunk: LispObj) -> LispObj {
    if !syms.is_cons() {
        if syms != nil() {
            lisp_error("PROGV: the symbol list is not a proper list");
        }
        return prims::funcall(thunk, &[]);
    }
    let sym = syms.car();
    if !sym.is_symbol() {
        lisp_error("PROGV: not a symbol");
    }
    let (val, rest) = if vals.is_cons() {
        (vals.car(), vals.cdr())
    } else {
        (SYMBOL_UNBOUND, nil())
    };
    let mut frame = [LispObj(0); DF_CELLS];
    frame[DF_PREV] = LispObj(dyn_head());
    frame[DF_KIND] = LispObj::fixnum(DF_SPECIAL);
    frame[DF_TAG] = sym;
    frame[DF_LANDING] = symbols::symbol_value(sym);
    let addr = std::hint::black_box(frame.as_mut_ptr()) as usize;
    set_dyn_head(addr);
    symbols::set_symbol_value(sym, val);
    let r = progv_bind(syms.cdr(), rest, thunk);
    debug_assert_eq!(dyn_head(), addr);
    symbols::set_symbol_value(sym, df(addr, DF_LANDING));
    set_dyn_head(df(addr, DF_PREV).raw());
    std::hint::black_box(&mut frame);
    r
}
