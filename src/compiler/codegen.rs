//! Code generator: IR -> x86-64 machine code (via iced-x86), one compiled
//! code object per lambda. See `runtime.rs` for the calling convention.
//!
//! Every value is computed into RAX. Frame layout:
//!   [rbp+16+8i] argument i, [rbp+8] return address, [rbp] saved rbp,
//!   [rbp-8] closure environment (R13 on entry), [rbp-16] argument count,
//!   [rbp-8s] slot s >= 3: locals, temporaries and dynamic frames.
//! The compiler tracks the stack depth (in cells below RBP) statically.
//! Constants are loaded RIP-relative from a literal area after the code.

use std::collections::HashMap;

use iced_x86::code_asm::*;

use super::frontend::*;
use crate::error::lisp_error;
use crate::heap;
use crate::object::*;
use crate::qv::nil;
use crate::qv_index::*;
use crate::runtime::*;

type R<T = ()> = Result<T, IcedError>;

const NIL_MEM: i32 = (Nil_Index * CELL) as i32;
const T_MEM: i32 = (T_Index * CELL) as i32;

fn qvd(i: usize) -> i32 {
    (i * CELL) as i32
}

fn fix(n: i64) -> i32 {
    (n << 3) as i32
}

/// Where a variable lives.
#[derive(Clone, Copy)]
enum Loc {
    Arg(usize),
    Slot(usize),
    Env(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DynKind {
    Special,
    Catch,
}

struct BlockGen {
    exit: CodeLabel,
    depth: usize,
    dyn_len: usize,
}

struct TagbodyGen {
    labels: Vec<CodeLabel>,
    depth: usize,
    dyn_len: usize,
}

pub static DISASSEMBLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct FnGen<'a> {
    fe: &'a Frontend,
    lambda: LambdaId,
    a: CodeAssembler,
    depth: usize,
    /// Dynamic frames pushed by this function: kind and slot of cell 0.
    dyn_stack: Vec<(DynKind, usize)>,
    locs: HashMap<VarId, Loc>,
    literals: Vec<(LispObj, CodeLabel)>,
    blocks: HashMap<BlockId, BlockGen>,
    tagbodies: HashMap<TagbodyId, TagbodyGen>,
    last_label_at: Option<usize>,
}

/// Compile lambda `id` (and, recursively, its inner lambdas) into a
/// compiled code object.
pub fn compile_lambda(fe: &Frontend, id: LambdaId) -> LispObj {
    let mut g = FnGen {
        fe,
        lambda: id,
        a: CodeAssembler::new(64).unwrap(),
        depth: 0,
        dyn_stack: Vec::new(),
        locs: HashMap::new(),
        literals: Vec::new(),
        blocks: HashMap::new(),
        tagbodies: HashMap::new(),
        last_label_at: None,
    };
    for (i, &v) in fe.lambdas[id].free.iter().enumerate() {
        g.locs.insert(v, Loc::Env(i));
    }
    match g.gen_lambda().and_then(|_| g.finish()) {
        Ok(code) => code,
        Err(e) => lisp_error(format!("Compiler error: {e}")),
    }
}

impl FnGen<'_> {
    // -----------------------------------------------------------------------
    // Small helpers

    fn slot(s: usize) -> AsmMemoryOperand {
        qword_ptr(rbp - (s * CELL) as i32)
    }

    fn label(&mut self) -> CodeLabel {
        self.a.create_label()
    }

    /// Define label `l` at the current position (iced allows only one
    /// label per instruction, so pad with a NOP when needed).
    fn here(&mut self, l: &mut CodeLabel) -> R {
        let n = self.a.instructions().len();
        if self.last_label_at == Some(n) {
            self.a.nop()?;
        }
        self.last_label_at = Some(self.a.instructions().len());
        self.a.set_label(l)
    }

    fn literal(&mut self, x: LispObj) -> CodeLabel {
        if let Some(&(_, l)) = self.literals.iter().find(|(o, _)| *o == x) {
            return l;
        }
        let l = self.a.create_label();
        self.literals.push((x, l));
        l
    }

    /// Load constant `x` into `r`.
    fn load_const(&mut self, r: AsmRegister64, x: LispObj) -> R {
        if x == nil() {
            self.a.mov(r, qword_ptr(r15 + NIL_MEM))
        } else if x == crate::qv::t() {
            self.a.mov(r, qword_ptr(r15 + T_MEM))
        } else if (x.is_fixnum() || x.is_character()) && (x.raw() as i64) == (x.raw() as i32 as i64) {
            self.a.mov(r, x.raw() as i32 as i64 as u64)
        } else {
            let l = self.literal(x);
            self.a.mov(r, qword_ptr(l))
        }
    }

    fn push_rax(&mut self) -> R {
        self.depth += 1;
        self.a.push(rax)
    }

    fn pop_to(&mut self, r: AsmRegister64) -> R {
        self.depth -= 1;
        self.a.pop(r)
    }

    fn drop_cells(&mut self, n: usize) -> R {
        if n > 0 {
            self.depth -= n;
            self.a.add(rsp, (n * CELL) as i32)?;
        }
        Ok(())
    }

    fn set_one_value(&mut self, mv: bool) -> R {
        if mv {
            self.a.mov(ecx, 1)?;
        }
        Ok(())
    }

    /// Call Rust helper `h` (arguments already in rdi, rsi, rdx, ...).
    fn helper(&mut self, h: Helper) -> R {
        self.a.mov(eax, h as u32)?;
        self.a.call(qword_ptr(r15 + qvd(PORT_HELPER_STUB_Index)))
    }

    /// RCX = value count left in QV by a helper.
    fn load_value_count(&mut self) -> R {
        self.a.mov(rcx, qword_ptr(r15 + qvd(PORT_NUM_RETURN_VALUES_Index)))?;
        self.a.sar(rcx, 3)
    }

    /// RAX = T if the flags say `cond`, else NIL.
    fn bool_from_flags(&mut self, equal: bool) -> R {
        self.a.mov(rax, qword_ptr(r15 + NIL_MEM))?;
        if equal {
            self.a.cmove(rax, qword_ptr(r15 + T_MEM))
        } else {
            self.a.cmovne(rax, qword_ptr(r15 + T_MEM))
        }
    }

    // -----------------------------------------------------------------------
    // Variables

    fn boxed(&self, v: VarId) -> bool {
        self.fe.vars[v].boxed
    }

    /// Load the variable's cell contents (the box itself for boxed vars).
    fn load_loc(&mut self, r: AsmRegister64, v: VarId) -> R {
        match self.locs[&v] {
            Loc::Arg(i) => self.a.mov(r, qword_ptr(rbp + (16 + i * CELL) as i32)),
            Loc::Slot(s) => self.a.mov(r, Self::slot(s)),
            Loc::Env(i) => {
                self.a.mov(r, qword_ptr(rbp - 8))?;
                self.a.mov(
                    r,
                    qword_ptr(r + ((ARRAY_SIMPLE_VECTOR_START + i) * CELL) as i32 - 5),
                )
            }
        }
    }

    fn load_var(&mut self, v: VarId) -> R {
        self.load_loc(rax, v)?;
        if self.boxed(v) {
            self.a.mov(rax, qword_ptr(rax - 4))?;
        }
        Ok(())
    }

    /// Store RAX into the variable.
    fn store_var(&mut self, v: VarId) -> R {
        if self.boxed(v) {
            self.load_loc(rdx, v)?;
            self.a.mov(qword_ptr(rdx - 4), rax)
        } else {
            match self.locs[&v] {
                Loc::Arg(i) => self.a.mov(qword_ptr(rbp + (16 + i * CELL) as i32), rax),
                Loc::Slot(s) => self.a.mov(Self::slot(s), rax),
                Loc::Env(_) => unreachable!("closure variables are boxed"),
            }
        }
    }

    /// Bind lexical `v` to RAX in a new slot (boxing if needed).
    fn bind_lex_new_slot(&mut self, v: VarId) -> R {
        if self.boxed(v) {
            self.a.mov(rdi, rax)?;
            self.a.mov(rsi, qword_ptr(r15 + NIL_MEM))?;
            self.helper(Helper::Cons)?;
        }
        self.push_rax()?;
        self.locs.insert(v, Loc::Slot(self.depth));
        Ok(())
    }

    /// Bind `b` to the value in RAX.
    fn bind(&mut self, b: Bind) -> R {
        match b {
            Bind::Lex(v) => self.bind_lex_new_slot(v),
            Bind::Special(sym) => self.push_special_binding(sym),
        }
    }

    fn symbol_value_cell(&mut self, r: AsmRegister64, sym: LispObj) -> R {
        self.load_const(r, sym)?;
        self.a.mov(r, qword_ptr(r + (SYMBOL_VALUE * CELL) as i32 - 5))
    }

    /// Dynamically bind `sym` to RAX (pushes a special frame).
    fn push_special_binding(&mut self, sym: LispObj) -> R {
        self.a.push(rbp)?;
        self.symbol_value_cell(r11, sym)?;
        self.a.push(qword_ptr(r11 - 4))?;
        self.load_const(rdx, sym)?;
        self.a.push(rdx)?;
        self.a.push(fix(DF_SPECIAL))?;
        self.a.push(qword_ptr(r15 + qvd(PORT_DYN_FRAME_Index)))?;
        self.a.mov(qword_ptr(r15 + qvd(PORT_DYN_FRAME_Index)), rsp)?;
        self.a.mov(qword_ptr(r11 - 4), rax)?;
        self.depth += DF_CELLS;
        self.dyn_stack.push((DynKind::Special, self.depth));
        Ok(())
    }

    /// Push a catch-type frame (catch or unwind-protect) with the tag in
    /// RAX and landing address `landing`.
    fn push_catch_frame(&mut self, kind: i64, landing: CodeLabel) -> R {
        self.a.push(rbp)?;
        self.a.lea(rdx, qword_ptr(landing))?;
        self.a.push(rdx)?;
        self.a.push(rax)?;
        self.a.push(fix(kind))?;
        self.a.push(qword_ptr(r15 + qvd(PORT_DYN_FRAME_Index)))?;
        self.a.mov(qword_ptr(r15 + qvd(PORT_DYN_FRAME_Index)), rsp)?;
        self.depth += DF_CELLS;
        if kind == DF_CATCH {
            self.dyn_stack.push((DynKind::Catch, self.depth));
        }
        Ok(())
    }

    /// Undo dynamic frame (kind, slot) without touching RAX/RCX. Does not
    /// change RSP.
    fn emit_undo_frame(&mut self, kind: DynKind, s: usize) -> R {
        let base = -((s * CELL) as i32);
        if kind == DynKind::Special {
            self.a.mov(rdx, qword_ptr(rbp + base + (DF_TAG * CELL) as i32))?;
            self.a.mov(r11, qword_ptr(rdx + (SYMBOL_VALUE * CELL) as i32 - 5))?;
            self.a.mov(rdx, qword_ptr(rbp + base + (DF_LANDING * CELL) as i32))?;
            self.a.mov(qword_ptr(r11 - 4), rdx)?;
        }
        self.a.mov(rdx, qword_ptr(rbp + base))?;
        self.a.mov(qword_ptr(r15 + qvd(PORT_DYN_FRAME_Index)), rdx)
    }

    /// Pop the innermost dynamic frame (must be at the top of the stack).
    fn pop_dyn_frame(&mut self) -> R {
        let (kind, s) = self.dyn_stack.pop().unwrap();
        debug_assert_eq!(s, self.depth);
        self.emit_undo_frame(kind, s)?;
        self.drop_cells(DF_CELLS)
    }

    /// Undo (without popping from the compile-time stack) the frames above
    /// `len`, innermost first: used by local exits.
    fn unwind_dyn_to(&mut self, len: usize) -> R {
        let frames: Vec<_> = self.dyn_stack[len..].iter().rev().copied().collect();
        for (k, s) in frames {
            self.emit_undo_frame(k, s)?;
        }
        Ok(())
    }

    fn reset_rsp(&mut self, depth: usize) -> R {
        self.a.lea(rsp, Self::slot(depth))
    }

    // -----------------------------------------------------------------------
    // Lambda

    fn gen_lambda(&mut self) -> R {
        let info = &self.fe.lambdas[self.lambda];
        let fe = self.fe;
        let nreq = info.req.len();
        let nopt = info.opt.len();
        let mut wrong = self.label();

        self.a.push(rbp)?;
        self.a.mov(rbp, rsp)?;
        self.a.push(r13)?;
        self.a.push(rcx)?;
        self.depth = 2;

        // argument count check
        if nopt == 0 && info.rest.is_none() && !info.has_keys {
            self.a.cmp(rcx, nreq as i32)?;
            self.a.jne(wrong)?;
        } else {
            if nreq > 0 {
                self.a.cmp(rcx, nreq as i32)?;
                self.a.jb(wrong)?;
            }
            if info.rest.is_none() && !info.has_keys {
                self.a.cmp(rcx, (nreq + nopt) as i32)?;
                self.a.ja(wrong)?;
            }
        }

        for (i, &b) in info.req.iter().enumerate() {
            match b {
                Bind::Lex(v) if !self.boxed(v) => {
                    self.locs.insert(v, Loc::Arg(i));
                }
                _ => {
                    self.a.mov(rax, qword_ptr(rbp + (16 + i * CELL) as i32))?;
                    self.bind(b)?;
                }
            }
        }
        for (j, o) in info.opt.iter().enumerate() {
            let k = nreq + j;
            let mut dflt = self.label();
            let mut done = self.label();
            self.a.cmp(qword_ptr(rbp - 16), k as i32)?;
            self.a.jbe(dflt)?;
            self.a.mov(rax, qword_ptr(rbp + (16 + k * CELL) as i32))?;
            self.a.jmp(done)?;
            self.here(&mut dflt)?;
            self.emit(&o.init, false)?;
            self.here(&mut done)?;
            self.bind(o.var)?;
            if let Some(sv) = o.svar {
                self.a.cmp(qword_ptr(rbp - 16), k as i32)?;
                self.a.mov(rax, qword_ptr(r15 + NIL_MEM))?;
                self.a.cmova(rax, qword_ptr(r15 + T_MEM))?;
                self.bind(sv)?;
            }
        }
        let mut rest_slot = None;
        if info.rest.is_some() || info.has_keys {
            let k = nreq + nopt;
            self.a.lea(rdi, qword_ptr(rbp + (16 + k * CELL) as i32))?;
            self.a.mov(rsi, qword_ptr(rbp - 16))?;
            self.a.sub(rsi, k as i32)?;
            self.helper(Helper::MakeRest)?;
            self.push_rax()?;
            let rs = self.depth;
            if let Some(b) = info.rest {
                self.a.mov(rax, Self::slot(rs))?;
                self.bind(b)?;
            }
            rest_slot = Some(rs);
        }
        if info.has_keys {
            let rs = rest_slot.unwrap();
            if !info.allow_other_keys {
                let allowed = heap::list(&info.keys.iter().map(|k| k.key).collect::<Vec<_>>());
                self.a.mov(rdi, Self::slot(rs))?;
                self.load_const(rsi, allowed)?;
                self.load_const(rdx, info.name)?;
                self.helper(Helper::CheckKeys)?;
            }
            for kp in &info.keys {
                self.a.mov(rdi, Self::slot(rs))?;
                self.load_const(rsi, kp.key)?;
                self.helper(Helper::GetKey)?;
                self.push_rax()?;
                let tmp = self.depth;
                let mut have = self.label();
                self.a.cmp(rax, SYMBOL_UNBOUND.raw() as i32)?;
                self.a.jne(have)?;
                self.emit(&kp.init, false)?;
                self.here(&mut have)?;
                self.bind(kp.var)?;
                if let Some(sv) = kp.svar {
                    self.a.cmp(Self::slot(tmp), SYMBOL_UNBOUND.raw() as i32)?;
                    self.bool_from_flags(false)?;
                    self.bind(sv)?;
                }
            }
        }
        for (b, init) in &info.aux {
            self.emit(init, false)?;
            self.bind(*b)?;
        }

        self.emit(&fe.lambdas[self.lambda].body, true)?;

        while let Some((k, s)) = self.dyn_stack.pop() {
            self.emit_undo_frame(k, s)?;
        }
        self.a.mov(rsp, rbp)?;
        self.a.pop(rbp)?;
        self.a.ret()?;

        self.here(&mut wrong)?;
        self.a.mov(rdi, rcx)?;
        self.load_const(rsi, info.name)?;
        self.helper(Helper::WrongArgs)?;
        self.a.ud2()?;
        Ok(())
    }

    /// Assemble and build the code object.
    fn finish(mut self) -> R<LispObj> {
        let lits = std::mem::take(&mut self.literals);
        let mut first = None;
        for (x, mut l) in lits.iter().copied() {
            self.a.set_label(&mut l)?;
            self.a.dq(&[x.raw() as u64])?;
            first.get_or_insert(l);
        }
        let r = self
            .a
            .assemble_options(0, iced_x86::BlockEncoderOptions::RETURN_NEW_INSTRUCTION_OFFSETS)?;
        let lit_off = match first {
            Some(l) => r.label_ip(&l)? as usize,
            None => r.inner.code_buffer.len(),
        };
        let bytes = &r.inner.code_buffer;
        let total = CODE_START + bytes.len();
        let cells = total.div_ceil(CELL);
        let code = heap::alloc_uvector(cells - 1, COMPILED_CODE_TYPE);
        code.uset(CODE_ENTRY_OFFSET, LispObj::fixnum(CODE_START as i64));
        code.uset(
            CODE_LITERALS_OFFSET,
            LispObj::fixnum((CODE_START + lit_off) as i64),
        );
        code.uset(CODE_NUM_LITERALS, LispObj::fixnum(lits.len() as i64));
        code.uset(CODE_NAME, self.fe.lambdas[self.lambda].name);
        code.uset(CODE_INFO, nil());
        unsafe {
            let dst = (code.uvector_ptr() as *mut u8).add(CODE_START);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
        }
        if DISASSEMBLE.load(std::sync::atomic::Ordering::Relaxed) {
            disassemble(code, lit_off);
        }
        Ok(code)
    }

    // -----------------------------------------------------------------------
    // Nodes

    /// Generate `n` into RAX. If `mv`, RCX (and QV) describe all values.
    fn emit(&mut self, n: &Node, mv: bool) -> R {
        match n {
            Node::Const(x) => {
                self.load_const(rax, *x)?;
                self.set_one_value(mv)
            }
            Node::LexRef(v) => {
                self.load_var(*v)?;
                self.set_one_value(mv)
            }
            Node::LexSet(v, val) => {
                self.emit(val, false)?;
                self.store_var(*v)?;
                self.set_one_value(mv)
            }
            Node::SpecialRef(sym) => {
                let mut ok = self.label();
                self.symbol_value_cell(rax, *sym)?;
                self.a.mov(rax, qword_ptr(rax - 4))?;
                self.a.cmp(rax, SYMBOL_UNBOUND.raw() as i32)?;
                self.a.jne(ok)?;
                self.load_const(rdi, *sym)?;
                self.helper(Helper::UnboundVariable)?;
                self.here(&mut ok)?;
                self.set_one_value(mv)
            }
            Node::SpecialSet(sym, val) => {
                self.emit(val, false)?;
                self.symbol_value_cell(rdx, *sym)?;
                self.a.mov(qword_ptr(rdx - 4), rax)?;
                self.set_one_value(mv)
            }
            Node::FunctionGlobal(sym) => {
                self.load_const(rdi, *sym)?;
                self.helper(Helper::SymbolFunction)?;
                self.set_one_value(mv)
            }
            Node::If(c, t, e) => {
                let mut els = self.label();
                let mut end = self.label();
                self.emit(c, false)?;
                self.a.cmp(rax, qword_ptr(r15 + NIL_MEM))?;
                self.a.je(els)?;
                let d = self.depth;
                self.emit(t, mv)?;
                self.a.jmp(end)?;
                self.here(&mut els)?;
                self.depth = d;
                self.emit(e, mv)?;
                self.here(&mut end)
            }
            Node::Progn(v) => {
                if v.is_empty() {
                    self.load_const(rax, nil())?;
                    return self.set_one_value(mv);
                }
                for (i, f) in v.iter().enumerate() {
                    self.emit(f, mv && i + 1 == v.len())?;
                }
                Ok(())
            }
            Node::Let {
                binds,
                sequential,
                body,
            } => self.gen_let(binds, *sequential, body, mv),
            Node::Lambda(id) => {
                self.gen_closure(*id)?;
                self.set_one_value(mv)
            }
            Node::Call(callee, args) => self.gen_call(callee, args),
            Node::Inline(op, args) => {
                self.gen_inline(*op, args)?;
                self.set_one_value(mv)
            }
            Node::Block(id, body) => self.gen_block(*id, body),
            Node::ReturnFrom {
                block,
                value,
                nonlocal,
            } => self.gen_return_from(*block, value, *nonlocal),
            Node::Tagbody(id, items) => {
                self.gen_tagbody(*id, items)?;
                self.set_one_value(mv)
            }
            Node::Go {
                tagbody,
                index,
                nonlocal,
            } => self.gen_go(*tagbody, *index, *nonlocal),
            Node::Catch(tag, body) => self.gen_catch(tag, body),
            Node::Throw(tag, val) => {
                self.emit(tag, false)?;
                self.push_rax()?;
                self.emit(val, true)?;
                self.save_values_to_rax()?;
                self.a.mov(rsi, rax)?;
                self.pop_to(rdi)?;
                self.helper(Helper::Throw)?;
                self.a.ud2()
            }
            Node::UnwindProtect(p, c) => self.gen_unwind_protect(p, c),
            Node::MvCall(f, args) => {
                self.emit(f, false)?;
                self.push_rax()?;
                for x in args {
                    self.emit(x, true)?;
                    self.save_values_to_rax()?;
                    self.push_rax()?;
                }
                let k = args.len();
                self.a.mov(rdi, Self::slot(self.depth - k))?;
                self.a.mov(rsi, rsp)?;
                self.a.mov(edx, k as u32)?;
                self.helper(Helper::MvApply)?;
                self.load_value_count()?;
                self.drop_cells(k + 1)
            }
            Node::MvProg1(first, rest) => {
                if mv {
                    self.emit(first, true)?;
                    self.save_values_to_rax()?;
                    self.push_rax()?;
                    self.emit(rest, false)?;
                    self.pop_to(rdi)?;
                    self.helper(Helper::RestoreValues)?;
                    self.load_value_count()
                } else {
                    self.emit(first, false)?;
                    self.push_rax()?;
                    self.emit(rest, false)?;
                    self.pop_to(rax)
                }
            }
        }
    }

    /// RAX = list of the current values (RAX/RCX/QV).
    fn save_values_to_rax(&mut self) -> R {
        self.a.mov(rdi, rax)?;
        self.a.mov(rsi, rcx)?;
        self.helper(Helper::SaveValues)
    }

    fn gen_let(&mut self, binds: &[(Bind, Node)], sequential: bool, body: &Node, mv: bool) -> R {
        let d0 = self.depth;
        let dyn0 = self.dyn_stack.len();
        if sequential {
            for (b, init) in binds {
                self.emit(init, false)?;
                self.bind(*b)?;
            }
        } else {
            // Evaluate all inits into slots first; then box / bind specials.
            let mut slots = Vec::new();
            for (_, init) in binds {
                self.emit(init, false)?;
                self.push_rax()?;
                slots.push(self.depth);
            }
            for ((b, _), &s) in binds.iter().zip(&slots) {
                match *b {
                    Bind::Lex(v) => {
                        if self.boxed(v) {
                            self.a.mov(rdi, Self::slot(s))?;
                            self.a.mov(rsi, qword_ptr(r15 + NIL_MEM))?;
                            self.helper(Helper::Cons)?;
                            self.a.mov(Self::slot(s), rax)?;
                        }
                        self.locs.insert(v, Loc::Slot(s));
                    }
                    Bind::Special(sym) => {
                        self.a.mov(rax, Self::slot(s))?;
                        self.push_special_binding(sym)?;
                    }
                }
            }
        }
        self.emit(body, mv)?;
        while self.dyn_stack.len() > dyn0 {
            let (k, s) = self.dyn_stack.pop().unwrap();
            self.emit_undo_frame(k, s)?;
        }
        let n = self.depth - d0;
        self.drop_cells(n)
    }

    fn gen_closure(&mut self, id: LambdaId) -> R {
        let code = compile_lambda(self.fe, id);
        let free = &self.fe.lambdas[id].free;
        if free.is_empty() {
            let f = make_function(code, nil());
            return self.load_const(rax, f);
        }
        let k = free.len();
        for &v in free.clone().iter() {
            self.load_loc(rax, v)?;
            self.push_rax()?;
        }
        self.load_const(rdi, code)?;
        self.a.mov(rsi, rsp)?;
        self.a.mov(edx, k as u32)?;
        self.helper(Helper::MakeClosure)?;
        self.drop_cells(k)
    }

    fn gen_call(&mut self, callee: &Callee, args: &[Node]) -> R {
        let n = args.len();
        let mut fslot = None;
        if let Callee::Expr(f) = callee {
            self.emit(f, false)?;
            self.push_rax()?;
            fslot = Some(self.depth);
        }
        if n > 0 {
            self.a.sub(rsp, (n * CELL) as i32)?;
            self.depth += n;
        }
        let d = self.depth;
        for (i, x) in args.iter().enumerate() {
            self.emit(x, false)?;
            self.a.mov(Self::slot(d - i), rax)?;
        }
        match callee {
            Callee::Global(sym) => self.load_const(rax, *sym)?,
            Callee::Lex(v) => self.load_var(*v)?,
            Callee::Expr(_) => self.a.mov(rax, Self::slot(fslot.unwrap()))?,
        }
        self.a.mov(ecx, n as u32)?;
        self.a.call(qword_ptr(r15 + qvd(PORT_FUNCALL_STUB_Index)))?;
        self.drop_cells(n + fslot.map_or(0, |_| 1))
    }

    fn gen_two_args(&mut self, args: &[Node]) -> R {
        self.emit(&args[0], false)?;
        self.push_rax()?;
        self.emit(&args[1], false)?;
        self.a.mov(rsi, rax)?;
        self.pop_to(rdi)
    }

    fn gen_inline(&mut self, op: InlineOp, args: &[Node]) -> R {
        match op {
            InlineOp::Car | InlineOp::Cdr => {
                let mut notcons = self.label();
                let mut done = self.label();
                self.emit(&args[0], false)?;
                self.a.mov(edx, eax)?;
                self.a.and(edx, 7)?;
                self.a.cmp(edx, CONS_TAG as i32)?;
                self.a.jne(notcons)?;
                let off = if op == InlineOp::Car { -4 } else { 4 };
                self.a.mov(rax, qword_ptr(rax + off))?;
                self.a.jmp(done)?;
                self.here(&mut notcons)?;
                self.a.cmp(rax, qword_ptr(r15 + NIL_MEM))?;
                self.a.je(done)?;
                self.a.mov(rdi, rax)?;
                self.helper(Helper::NotList)?;
                self.here(&mut done)
            }
            InlineOp::Cons => {
                self.gen_two_args(args)?;
                self.helper(Helper::Cons)
            }
            InlineOp::Eq => {
                self.gen_two_args(args)?;
                self.a.cmp(rdi, rsi)?;
                self.bool_from_flags(true)
            }
            InlineOp::Null => {
                self.emit(&args[0], false)?;
                self.a.cmp(rax, qword_ptr(r15 + NIL_MEM))?;
                self.bool_from_flags(true)
            }
            InlineOp::Add2
            | InlineOp::Sub2
            | InlineOp::NumEq2
            | InlineOp::Lt2
            | InlineOp::Gt2
            | InlineOp::Le2
            | InlineOp::Ge2 => {
                self.gen_two_args(args)?;
                let h = match op {
                    InlineOp::Add2 => Helper::Add2,
                    InlineOp::Sub2 => Helper::Sub2,
                    InlineOp::NumEq2 => Helper::NumEq2,
                    InlineOp::Lt2 => Helper::Lt2,
                    InlineOp::Gt2 => Helper::Gt2,
                    InlineOp::Le2 => Helper::Le2,
                    _ => Helper::Ge2,
                };
                self.helper(h)
            }
        }
    }

    /// Landing code after a throw to a catch frame: values from QV.
    fn load_thrown_values(&mut self) -> R {
        self.load_value_count()?;
        self.a.mov(rax, qword_ptr(r15 + qvd(RETURN_VALUES_Index)))
    }

    fn gen_block(&mut self, id: BlockId, body: &Node) -> R {
        let info = &self.fe.blocks[id];
        let d0 = self.depth;
        let mut exit = self.label();
        self.blocks.insert(
            id,
            BlockGen {
                exit,
                depth: d0,
                dyn_len: self.dyn_stack.len(),
            },
        );
        if !info.nonlocal {
            self.emit(body, true)?;
            return self.here(&mut exit);
        }
        let mut landing = self.label();
        let mut popped = self.label();
        self.helper(Helper::FreshTag)?;
        self.bind_lex_new_slot(info.tag_var)?;
        self.load_var(info.tag_var)?;
        self.push_catch_frame(DF_CATCH, landing)?;
        self.emit(body, true)?;
        self.pop_dyn_frame()?;
        self.here(&mut popped)?;
        self.drop_cells(1)?;
        self.here(&mut exit)?;
        let after = self.depth;
        // out of line landing pad
        let mut skip = self.label();
        self.a.jmp(skip)?;
        self.here(&mut landing)?;
        self.load_thrown_values()?;
        self.a.mov(rdx, qword_ptr(rsp))?;
        self.a.mov(qword_ptr(r15 + qvd(PORT_DYN_FRAME_Index)), rdx)?;
        self.a.add(rsp, (DF_CELLS * CELL) as i32)?;
        self.a.jmp(popped)?;
        self.here(&mut skip)?;
        self.depth = after;
        Ok(())
    }

    fn gen_return_from(&mut self, id: BlockId, value: &Node, nonlocal: bool) -> R {
        self.emit(value, true)?;
        if nonlocal {
            self.save_values_to_rax()?;
            self.push_rax()?;
            let tv = self.fe.blocks[id].tag_var;
            self.load_var(tv)?;
            self.a.mov(rdi, rax)?;
            self.pop_to(rsi)?;
            self.helper(Helper::Throw)?;
            return self.a.ud2();
        }
        let (exit, depth, dyn_len) = {
            let b = &self.blocks[&id];
            (b.exit, b.depth, b.dyn_len)
        };
        self.unwind_dyn_to(dyn_len)?;
        if self.depth != depth {
            self.reset_rsp(depth)?;
        }
        self.a.jmp(exit)
    }

    fn gen_tagbody(&mut self, id: TagbodyId, items: &[TagItem]) -> R {
        let info = &self.fe.tagbodies[id];
        let d0 = self.depth;
        let ntags = items.iter().filter(|i| matches!(i, TagItem::Tag(_))).count();
        let labels: Vec<CodeLabel> = (0..ntags).map(|_| self.a.create_label()).collect();
        let mut landing = self.label();
        if info.nonlocal {
            self.helper(Helper::FreshTag)?;
            self.bind_lex_new_slot(info.tag_var)?;
            self.load_var(info.tag_var)?;
            self.push_catch_frame(DF_CATCH, landing)?;
        }
        let body_depth = self.depth;
        self.tagbodies.insert(
            id,
            TagbodyGen {
                labels: labels.clone(),
                depth: body_depth,
                dyn_len: self.dyn_stack.len(),
            },
        );
        let mut labels = labels;
        for it in items {
            match it {
                TagItem::Tag(i) => self.here(&mut labels[*i])?,
                TagItem::Form(f) => {
                    self.emit(f, false)?;
                    self.depth = body_depth;
                }
            }
        }
        if info.nonlocal {
            self.pop_dyn_frame()?;
            self.drop_cells(1)?;
            let mut skip = self.label();
            self.a.jmp(skip)?;
            // landing: the frame stays linked; RSP = frame address
            self.here(&mut landing)?;
            self.a.mov(rax, qword_ptr(r15 + qvd(RETURN_VALUES_Index)))?;
            let labels = self.tagbodies[&id].labels.clone();
            for (i, l) in labels.iter().enumerate() {
                self.a.cmp(rax, fix(i as i64))?;
                self.a.je(*l)?;
            }
            self.a.ud2()?;
            self.here(&mut skip)?;
        }
        debug_assert_eq!(self.depth, d0);
        self.load_const(rax, nil())
    }

    fn gen_go(&mut self, id: TagbodyId, index: usize, nonlocal: bool) -> R {
        if nonlocal {
            let tv = self.fe.tagbodies[id].tag_var;
            self.load_var(tv)?;
            self.a.mov(rdi, rax)?;
            let vals = heap::list(&[LispObj::fixnum(index as i64)]);
            self.load_const(rsi, vals)?;
            self.helper(Helper::Throw)?;
            return self.a.ud2();
        }
        let (label, depth, dyn_len) = {
            let t = &self.tagbodies[&id];
            (t.labels[index], t.depth, t.dyn_len)
        };
        self.unwind_dyn_to(dyn_len)?;
        if self.depth != depth {
            self.reset_rsp(depth)?;
        }
        self.a.jmp(label)
    }

    fn gen_catch(&mut self, tag: &Node, body: &Node) -> R {
        let mut landing = self.label();
        let mut end = self.label();
        self.emit(tag, false)?;
        self.push_catch_frame(DF_CATCH, landing)?;
        self.emit(body, true)?;
        self.pop_dyn_frame()?;
        self.a.jmp(end)?;
        self.here(&mut landing)?;
        self.load_thrown_values()?;
        self.a.mov(rdx, qword_ptr(rsp))?;
        self.a.mov(qword_ptr(r15 + qvd(PORT_DYN_FRAME_Index)), rdx)?;
        self.a.add(rsp, (DF_CELLS * CELL) as i32)?;
        self.here(&mut end)
    }

    fn gen_unwind_protect(&mut self, protected: &Node, cleanup: &Node) -> R {
        let mut landing = self.label();
        let mut end = self.label();
        let d0 = self.depth;
        self.load_const(rax, nil())?;
        self.push_catch_frame(DF_UNWIND_PROTECT, landing)?;
        self.emit(protected, true)?;
        self.save_values_to_rax()?;
        // pop the frame (not on dyn_stack: local exits across it throw)
        self.a.mov(rdx, qword_ptr(rsp))?;
        self.a.mov(qword_ptr(r15 + qvd(PORT_DYN_FRAME_Index)), rdx)?;
        self.drop_cells(DF_CELLS)?;
        self.push_rax()?;
        self.emit(cleanup, false)?;
        self.pop_to(rdi)?;
        self.helper(Helper::RestoreValues)?;
        self.load_value_count()?;
        self.a.jmp(end)?;
        // landing from a throw: frame already unlinked, RSP at d0
        self.here(&mut landing)?;
        self.depth = d0;
        self.helper(Helper::TakePending)?;
        self.push_rax()?;
        self.emit(cleanup, false)?;
        self.pop_to(rdi)?;
        self.helper(Helper::ContinueThrow)?;
        self.a.ud2()?;
        self.here(&mut end)
    }
}

fn disassemble(code: LispObj, lit_off: usize) {
    use iced_x86::{Decoder, DecoderOptions, Formatter, IntelFormatter};
    let start = unsafe { (code.uvector_ptr() as *const u8).add(CODE_START) };
    let bytes = unsafe { std::slice::from_raw_parts(start, lit_off) };
    let ip = start as u64;
    let mut d = Decoder::with_ip(64, bytes, ip, DecoderOptions::NONE);
    let mut f = IntelFormatter::new();
    let mut out = String::new();
    let name = crate::printer::write_to_string(code.uref(CODE_NAME));
    eprintln!(";; code object {:#x} ({name})", code.raw());
    for ins in &mut d {
        out.clear();
        f.format(&ins, &mut out);
        eprintln!("  {:6x}  {out}", ins.ip() - ip);
    }
    let n = code.uref(CODE_NUM_LITERALS).integer() as usize;
    for i in 0..n {
        let v = unsafe { (start.add(lit_off) as *const LispObj).add(i).read_unaligned() };
        eprintln!("  lit {i}: {}", crate::printer::write_to_string(v));
    }
}
