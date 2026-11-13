//! Compiler front end: Lisp forms -> IR.
//!
//! PORT: upstream `Compx86.cpp` generates code in one pass and finds closed
//! over variables by scanning embedded lambdas (`findLambdas`,
//! `referencedByEmbeddedLambdas`). Here macros are expanded and variables
//! resolved into an IR first, which records exactly which variables are
//! captured (they get heap boxes) and which `return-from`/`go` forms must
//! exit non-locally (from an inner lambda, or across an unwind-protect).

use crate::error::lisp_error;
use crate::object::*;
use crate::printer::write_to_string;
use crate::qv::{nil, qv};
use crate::qv_index::*;
use crate::runtime::lisp_call;
use crate::symbols::{
    find_keyword, find_symbol, is_constant_symbol, is_special_symbol, symbol_function,
    symbol_function_type, symbol_name, symbol_value,
};

pub type VarId = usize;
pub type LambdaId = usize;
pub type BlockId = usize;
pub type TagbodyId = usize;

pub struct Var {
    pub name: LispObj,
    pub lambda: LambdaId,
    /// Captured by an inner lambda: lives in a heap box (a cons, value in
    /// the CAR) shared with the closures.
    pub boxed: bool,
}

pub struct BlockInfo {
    pub lambda: LambdaId,
    up_depth: usize,
    /// Some `return-from` must throw: the block establishes a catch frame.
    pub nonlocal: bool,
    /// Holds the block's unique catch tag when non-local.
    pub tag_var: VarId,
}

pub struct TagbodyInfo {
    pub lambda: LambdaId,
    up_depth: usize,
    tags: Vec<LispObj>,
    pub nonlocal: bool,
    pub tag_var: VarId,
}

#[derive(Clone, Copy, Debug)]
pub enum Bind {
    Lex(VarId),
    Special(LispObj),
}

pub struct OptParam {
    pub var: Bind,
    pub init: Node,
    pub svar: Option<Bind>,
}

pub struct KeyParam {
    pub key: LispObj,
    pub var: Bind,
    pub init: Node,
    pub svar: Option<Bind>,
}

pub struct LambdaInfo {
    pub parent: Option<LambdaId>,
    pub name: LispObj,
    /// Variables of outer lambdas referenced here (or by inner lambdas):
    /// the closure environment, in this order.
    pub free: Vec<VarId>,
    pub req: Vec<Bind>,
    pub opt: Vec<OptParam>,
    pub rest: Option<Bind>,
    pub has_keys: bool,
    pub keys: Vec<KeyParam>,
    pub allow_other_keys: bool,
    pub aux: Vec<(Bind, Node)>,
    pub body: Node,
    up_depth: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineOp {
    Cons,
    Car,
    Cdr,
    Eq,
    Null,
    Add2,
    Sub2,
    NumEq2,
    Lt2,
    Gt2,
    Le2,
    Ge2,
}

pub enum Callee {
    Global(LispObj),
    Lex(VarId),
    Expr(Box<Node>),
}

pub enum TagItem {
    Tag(usize),
    Form(Node),
}

pub enum Node {
    Const(LispObj),
    LexRef(VarId),
    LexSet(VarId, Box<Node>),
    SpecialRef(LispObj),
    SpecialSet(LispObj, Box<Node>),
    /// `(function sym)` for a global function.
    FunctionGlobal(LispObj),
    If(Box<Node>, Box<Node>, Box<Node>),
    Progn(Vec<Node>),
    Let {
        binds: Vec<(Bind, Node)>,
        sequential: bool,
        body: Box<Node>,
    },
    Lambda(LambdaId),
    Call(Callee, Vec<Node>),
    Inline(InlineOp, Vec<Node>),
    Block(BlockId, Box<Node>),
    ReturnFrom {
        block: BlockId,
        value: Box<Node>,
        nonlocal: bool,
    },
    Tagbody(TagbodyId, Vec<TagItem>),
    Go {
        tagbody: TagbodyId,
        index: usize,
        nonlocal: bool,
    },
    Catch(Box<Node>, Box<Node>),
    Throw(Box<Node>, Box<Node>),
    UnwindProtect(Box<Node>, Box<Node>),
    MvCall(Box<Node>, Vec<Node>),
    MvProg1(Box<Node>, Box<Node>),
}

enum Scope {
    Var(LispObj, VarId),
    Special(LispObj),
    SymMacro(LispObj, LispObj),
    Fun(LispObj, VarId),
    /// MACROLET definition: name and expander function.
    Macro(LispObj, LispObj),
    Block(LispObj, BlockId),
    Tags(TagbodyId),
}

enum VarRef {
    Lex(VarId),
    Special,
    SymMacro(LispObj),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sop {
    Quote,
    Function,
    Setq,
    If,
    Progn,
    Let,
    LetStar,
    Flet,
    Labels,
    Block,
    ReturnFrom,
    Tagbody,
    Go,
    Catch,
    Throw,
    UnwindProtect,
    MvCall,
    MvProg1,
    EvalWhen,
    The,
    Locally,
    LoadTimeValue,
    SymbolMacrolet,
    Macrolet,
    Progv,
}

pub const SPECIAL_OPERATORS: &[&str] = &[
    "QUOTE",
    "FUNCTION",
    "SETQ",
    "IF",
    "PROGN",
    "LET",
    "LET*",
    "FLET",
    "LABELS",
    "BLOCK",
    "RETURN-FROM",
    "TAGBODY",
    "GO",
    "CATCH",
    "THROW",
    "UNWIND-PROTECT",
    "MULTIPLE-VALUE-CALL",
    "MULTIPLE-VALUE-PROG1",
    "EVAL-WHEN",
    "THE",
    "LOCALLY",
    "LOAD-TIME-VALUE",
    "SYMBOL-MACROLET",
    "MACROLET",
    "PROGV",
];

const SOPS: &[Sop] = &[
    Sop::Quote,
    Sop::Function,
    Sop::Setq,
    Sop::If,
    Sop::Progn,
    Sop::Let,
    Sop::LetStar,
    Sop::Flet,
    Sop::Labels,
    Sop::Block,
    Sop::ReturnFrom,
    Sop::Tagbody,
    Sop::Go,
    Sop::Catch,
    Sop::Throw,
    Sop::UnwindProtect,
    Sop::MvCall,
    Sop::MvProg1,
    Sop::EvalWhen,
    Sop::The,
    Sop::Locally,
    Sop::LoadTimeValue,
    Sop::SymbolMacrolet,
    Sop::Macrolet,
    Sop::Progv,
];

const INLINES: &[(&str, InlineOp, usize)] = &[
    ("CONS", InlineOp::Cons, 2),
    ("CAR", InlineOp::Car, 1),
    ("CDR", InlineOp::Cdr, 1),
    ("EQ", InlineOp::Eq, 2),
    ("NULL", InlineOp::Null, 1),
    ("NOT", InlineOp::Null, 1),
    ("+", InlineOp::Add2, 2),
    ("-", InlineOp::Sub2, 2),
    ("=", InlineOp::NumEq2, 2),
    ("<", InlineOp::Lt2, 2),
    (">", InlineOp::Gt2, 2),
    ("<=", InlineOp::Le2, 2),
    (">=", InlineOp::Ge2, 2),
];

struct Syms {
    sops: Vec<LispObj>,
    inlines: Vec<LispObj>,
    declare: LispObj,
    special: LispObj,
    lambda: LambdaSyms,
    funcall: LispObj,
}

struct LambdaSyms {
    lambda: LispObj,
    optional: LispObj,
    rest: LispObj,
    body: LispObj,
    key: LispObj,
    aux: LispObj,
    allow_other_keys: LispObj,
}

impl Syms {
    fn new() -> Syms {
        Syms {
            sops: SPECIAL_OPERATORS.iter().map(|n| find_symbol(n)).collect(),
            inlines: INLINES.iter().map(|(n, _, _)| find_symbol(n)).collect(),
            declare: qv(DECLARE_Index),
            special: qv(SPECIAL_Index),
            lambda: LambdaSyms {
                lambda: qv(LAMBDA_Index),
                optional: find_symbol("&OPTIONAL"),
                rest: find_symbol("&REST"),
                body: find_symbol("&BODY"),
                key: find_symbol("&KEY"),
                aux: find_symbol("&AUX"),
                allow_other_keys: find_symbol("&ALLOW-OTHER-KEYS"),
            },
            funcall: find_symbol("FUNCALL"),
        }
    }
}

pub struct Frontend {
    pub vars: Vec<Var>,
    pub lambdas: Vec<LambdaInfo>,
    pub blocks: Vec<BlockInfo>,
    pub tagbodies: Vec<TagbodyInfo>,
    scope: Vec<Scope>,
    cur: LambdaId,
    syms: Syms,
}

// ---------------------------------------------------------------------------
// List helpers

fn list_items(mut l: LispObj) -> Vec<LispObj> {
    let mut v = Vec::new();
    while l.is_cons() {
        v.push(l.car());
        l = l.cdr();
    }
    v
}

fn nth(l: LispObj, n: usize) -> LispObj {
    let mut p = l;
    for _ in 0..n {
        if !p.is_cons() {
            return nil();
        }
        p = p.cdr();
    }
    if p.is_cons() { p.car() } else { nil() }
}

fn list_len(l: LispObj) -> usize {
    list_items(l).len()
}

fn err_form(msg: &str, form: LispObj) -> ! {
    lisp_error(format!("{msg}: {}", write_to_string(form)))
}

/// True if `sym` names a global macro.
pub fn macro_function(sym: LispObj) -> Option<LispObj> {
    if sym.is_symbol() && symbol_function_type(sym) == qv(MACRO_Index) {
        let f = symbol_function(sym);
        if f != UNINITIALIZED {
            return Some(f);
        }
    }
    None
}

impl Default for Frontend {
    fn default() -> Self {
        Self::new()
    }
}

impl Frontend {
    pub fn new() -> Frontend {
        Frontend {
            vars: Vec::new(),
            lambdas: Vec::new(),
            blocks: Vec::new(),
            tagbodies: Vec::new(),
            scope: Vec::new(),
            cur: 0,
            syms: Syms::new(),
        }
    }

    /// Convert a top-level lambda expression `(lambda ll . body)`.
    pub fn convert_toplevel_lambda(&mut self, lambda: LispObj, name: LispObj) -> LambdaId {
        if !lambda.is_cons() || lambda.car() != self.syms.lambda.lambda || !lambda.cdr().is_cons() {
            err_form("Not a lambda expression", lambda);
        }
        self.convert_lambda(lambda.cdr().car(), lambda.cdr().cdr(), name, true)
    }

    fn new_var(&mut self, name: LispObj) -> VarId {
        self.vars.push(Var {
            name,
            lambda: self.cur,
            boxed: false,
        });
        self.vars.len() - 1
    }

    /// Note a reference to `id` from the current lambda: if it belongs to
    /// an outer lambda it becomes boxed and joins the free lists.
    fn reference(&mut self, id: VarId) {
        let owner = self.vars[id].lambda;
        let mut l = self.cur;
        while l != owner {
            self.vars[id].boxed = true;
            if !self.lambdas[l].free.contains(&id) {
                self.lambdas[l].free.push(id);
            }
            l = self.lambdas[l].parent.expect("variable owner not found");
        }
    }

    fn lookup_var(&mut self, sym: LispObj) -> VarRef {
        let mut found = None;
        for s in self.scope.iter().rev() {
            match *s {
                Scope::Var(n, id) if n == sym => {
                    found = Some(VarRef::Lex(id));
                    break;
                }
                Scope::Special(n) if n == sym => return VarRef::Special,
                Scope::SymMacro(n, e) if n == sym => return VarRef::SymMacro(e),
                _ => {}
            }
        }
        match found {
            Some(VarRef::Lex(id)) => {
                self.reference(id);
                VarRef::Lex(id)
            }
            _ => VarRef::Special,
        }
    }

    /// Innermost lexical function or macro binding of `sym`.
    fn lookup_lexical_fun(&self, sym: LispObj) -> Option<&Scope> {
        self.scope.iter().rev().find(|s| match **s {
            Scope::Fun(n, _) | Scope::Macro(n, _) => n == sym,
            _ => false,
        })
    }

    fn lookup_fun(&mut self, sym: LispObj) -> Option<VarId> {
        let id = match self.lookup_lexical_fun(sym)? {
            Scope::Fun(_, id) => *id,
            _ => return None,
        };
        self.reference(id);
        Some(id)
    }

    fn lexical_macro(&self, sym: LispObj) -> Option<LispObj> {
        match self.lookup_lexical_fun(sym)? {
            Scope::Macro(_, f) => Some(*f),
            _ => None,
        }
    }

    /// Call macro expander `m` on `form`. If lexical macros or symbol
    /// macros are in scope, `*LEXICAL-MACROS*` and `*LEXICAL-SYMBOL-MACROS*`
    /// are bound to describe them (in the format expand.lisp uses), so
    /// that MACROEXPAND called by the expander sees them.
    fn expand_macro(&self, m: LispObj, form: LispObj) -> LispObj {
        let lexical = self
            .scope
            .iter()
            .any(|s| matches!(s, Scope::Macro(..) | Scope::SymMacro(..)));
        if !lexical {
            return lisp_call(m, &[form, nil()]);
        }
        let (mut lm, mut lsm) = (nil(), nil());
        let l = crate::heap::list;
        for s in &self.scope {
            match *s {
                Scope::Macro(n, f) => lm = crate::heap::cons(l(&[n, f]), lm),
                Scope::Fun(n, _) => lm = crate::heap::cons(l(&[n, nil()]), lm),
                Scope::SymMacro(n, e) => lsm = crate::heap::cons(l(&[n, e]), lsm),
                Scope::Var(n, _) | Scope::Special(n) => lsm = crate::heap::cons(n, lsm),
                _ => {}
            }
        }
        lisp_call(super::macro_caller(), &[m, form, lm, lsm])
    }

    fn up_depth(&self) -> usize {
        self.lambdas[self.cur].up_depth
    }

    // -----------------------------------------------------------------------
    // Forms

    pub fn convert(&mut self, x: LispObj) -> Node {
        if x.is_symbol() {
            self.convert_symbol(x)
        } else if x.is_cons() {
            self.convert_compound(x)
        } else {
            Node::Const(x)
        }
    }

    fn convert_symbol(&mut self, sym: LispObj) -> Node {
        if sym == nil() {
            return Node::Const(nil());
        }
        if is_constant_symbol(sym) {
            return Node::Const(symbol_value(sym));
        }
        match self.lookup_var(sym) {
            VarRef::Lex(id) => Node::LexRef(id),
            VarRef::Special => Node::SpecialRef(sym),
            VarRef::SymMacro(e) => self.convert(e),
        }
    }

    fn convert_progn(&mut self, body: LispObj) -> Node {
        let mut v: Vec<Node> = list_items(body).into_iter().map(|f| self.convert(f)).collect();
        if v.len() == 1 {
            v.pop().unwrap()
        } else {
            Node::Progn(v)
        }
    }

    fn convert_args(&mut self, args: LispObj) -> Vec<Node> {
        list_items(args).into_iter().map(|f| self.convert(f)).collect()
    }

    fn convert_compound(&mut self, x: LispObj) -> Node {
        let head = x.car();
        let args = x.cdr();
        if head.is_symbol() {
            if let Some(fv) = self.lookup_fun(head) {
                return Node::Call(Callee::Lex(fv), self.convert_args(args));
            }
            if let Some(i) = self.syms.sops.iter().position(|&s| s == head) {
                return self.convert_special(SOPS[i], x);
            }
            if let Some(m) = self.lexical_macro(head) {
                let expansion = self.expand_macro(m, x);
                return self.convert(expansion);
            }
            if let Some(m) = macro_function(head) {
                let expansion = self.expand_macro(m, x);
                return self.convert(expansion);
            }
            if head == self.syms.lambda.lambda {
                return self.convert_function(head, x);
            }
            if let Some(i) = self.syms.inlines.iter().position(|&s| s == head) {
                let (_, op, n) = INLINES[i];
                if list_len(args) == n {
                    return Node::Inline(op, self.convert_args(args));
                }
            }
            if head == self.syms.funcall && args.is_cons() {
                let f = self.convert(args.car());
                return Node::Call(Callee::Expr(Box::new(f)), self.convert_args(args.cdr()));
            }
            return Node::Call(Callee::Global(head), self.convert_args(args));
        }
        if head.is_cons() && head.car() == self.syms.lambda.lambda {
            let f = self.convert_function(head, head);
            return Node::Call(Callee::Expr(Box::new(f)), self.convert_args(args));
        }
        err_form("Cannot compile form", x)
    }

    /// Split leading declarations (and a doc string if `allow_doc`) off a
    /// body. Returns the declared special symbols and the remaining forms.
    fn parse_body(&self, mut body: LispObj, allow_doc: bool) -> (Vec<LispObj>, LispObj) {
        let mut specials = Vec::new();
        loop {
            if !body.is_cons() {
                break;
            }
            let f = body.car();
            if allow_doc && f.is_string() && body.cdr().is_cons() {
                body = body.cdr();
                continue;
            }
            if !(f.is_cons() && f.car() == self.syms.declare) {
                break;
            }
            for spec in list_items(f.cdr()) {
                if spec.is_cons() && spec.car() == self.syms.special {
                    specials.extend(list_items(spec.cdr()));
                }
            }
            body = body.cdr();
        }
        (specials, body)
    }

    fn is_special_binding(sym: LispObj, specials: &[LispObj]) -> bool {
        specials.contains(&sym) || is_special_symbol(sym)
    }

    /// Bind `sym` in the current scope; returns the binding.
    fn bind(&mut self, sym: LispObj, specials: &[LispObj]) -> Bind {
        if !sym.is_symbol() || sym == nil() || is_constant_symbol(sym) {
            err_form("Cannot bind", sym);
        }
        if Self::is_special_binding(sym, specials) {
            self.scope.push(Scope::Special(sym));
            Bind::Special(sym)
        } else {
            let id = self.new_var(sym);
            self.scope.push(Scope::Var(sym, id));
            Bind::Lex(id)
        }
    }

    /// Declare specials not bound by the form itself.
    fn push_free_specials(&mut self, specials: &[LispObj], bound: &[LispObj]) {
        for &s in specials {
            if !bound.contains(&s) {
                self.scope.push(Scope::Special(s));
            }
        }
    }

    fn convert_special(&mut self, op: Sop, x: LispObj) -> Node {
        let args = x.cdr();
        let a1 = nth(args, 0);
        match op {
            Sop::Quote => Node::Const(a1),
            Sop::Function => self.convert_function(a1, x),
            Sop::Setq => self.convert_setq(args),
            Sop::If => {
                let n = list_len(args);
                if !(2..=3).contains(&n) {
                    err_form("Invalid IF form", x);
                }
                Node::If(
                    Box::new(self.convert(a1)),
                    Box::new(self.convert(nth(args, 1))),
                    Box::new(self.convert(nth(args, 2))),
                )
            }
            Sop::Progn | Sop::EvalWhen => {
                let body = if op == Sop::Progn { args } else { args.cdr() };
                self.convert_progn(body)
            }
            Sop::Let | Sop::LetStar => self.convert_let(a1, args.cdr(), op == Sop::LetStar),
            Sop::Flet | Sop::Labels => self.convert_flet(a1, args.cdr(), op == Sop::Labels),
            Sop::Block => {
                let tag_var = self.new_var(nil());
                self.blocks.push(BlockInfo {
                    lambda: self.cur,
                    up_depth: self.up_depth(),
                    nonlocal: false,
                    tag_var,
                });
                let id = self.blocks.len() - 1;
                let mark = self.scope.len();
                self.scope.push(Scope::Block(a1, id));
                let body = self.convert_progn(args.cdr());
                self.scope.truncate(mark);
                Node::Block(id, Box::new(body))
            }
            Sop::ReturnFrom => {
                let id = self
                    .scope
                    .iter()
                    .rev()
                    .find_map(|s| match *s {
                        Scope::Block(n, id) if n == a1 => Some(id),
                        _ => None,
                    })
                    .unwrap_or_else(|| err_form("No block named", a1));
                let b = &self.blocks[id];
                let nonlocal = b.lambda != self.cur || b.up_depth != self.up_depth();
                if nonlocal {
                    self.blocks[id].nonlocal = true;
                    let tv = self.blocks[id].tag_var;
                    self.reference(tv);
                }
                let value = self.convert(nth(args, 1));
                Node::ReturnFrom {
                    block: id,
                    value: Box::new(value),
                    nonlocal,
                }
            }
            Sop::Tagbody => self.convert_tagbody(args),
            Sop::Go => {
                let (id, index) = self
                    .scope
                    .iter()
                    .rev()
                    .find_map(|s| match *s {
                        Scope::Tags(id) => self.tagbodies[id]
                            .tags
                            .iter()
                            .position(|&t| t == a1)
                            .map(|i| (id, i)),
                        _ => None,
                    })
                    .unwrap_or_else(|| err_form("No tag named", a1));
                let tb = &self.tagbodies[id];
                let nonlocal = tb.lambda != self.cur || tb.up_depth != self.up_depth();
                if nonlocal {
                    self.tagbodies[id].nonlocal = true;
                    let tv = self.tagbodies[id].tag_var;
                    self.reference(tv);
                }
                Node::Go {
                    tagbody: id,
                    index,
                    nonlocal,
                }
            }
            Sop::Catch => {
                let tag = self.convert(a1);
                let body = self.convert_progn(args.cdr());
                Node::Catch(Box::new(tag), Box::new(body))
            }
            Sop::Throw => {
                let tag = self.convert(a1);
                let v = self.convert(nth(args, 1));
                Node::Throw(Box::new(tag), Box::new(v))
            }
            Sop::UnwindProtect => {
                self.lambdas[self.cur].up_depth += 1;
                let p = self.convert(a1);
                self.lambdas[self.cur].up_depth -= 1;
                let c = self.convert_progn(args.cdr());
                Node::UnwindProtect(Box::new(p), Box::new(c))
            }
            Sop::MvCall => {
                let f = self.convert(a1);
                Node::MvCall(Box::new(f), self.convert_args(args.cdr()))
            }
            Sop::MvProg1 => {
                let f = self.convert(a1);
                let rest = self.convert_progn(args.cdr());
                Node::MvProg1(Box::new(f), Box::new(rest))
            }
            Sop::The => self.convert(nth(args, 1)),
            Sop::Locally => {
                let (specials, body) = self.parse_body(args, false);
                let mark = self.scope.len();
                self.push_free_specials(&specials, &[]);
                let n = self.convert_progn(body);
                self.scope.truncate(mark);
                n
            }
            Sop::LoadTimeValue => Node::Const(super::eval(a1)),
            Sop::SymbolMacrolet => {
                let mark = self.scope.len();
                for b in list_items(a1) {
                    let (sym, exp) = (nth(b, 0), nth(b, 1));
                    if !sym.is_symbol() {
                        err_form("Invalid SYMBOL-MACROLET binding", b);
                    }
                    self.scope.push(Scope::SymMacro(sym, exp));
                }
                let (specials, body) = self.parse_body(args.cdr(), false);
                self.push_free_specials(&specials, &[]);
                let n = self.convert_progn(body);
                self.scope.truncate(mark);
                n
            }
            Sop::Macrolet => {
                let mark = self.scope.len();
                let defmacro = find_symbol("DEFMACRO");
                for d in list_items(a1) {
                    if !d.is_cons() || !d.car().is_symbol() || !d.cdr().is_cons() {
                        err_form("Invalid MACROLET definition", d);
                    }
                    // As expand.lisp: define a macro on a fresh symbol with
                    // DEFMACRO and take its expander.
                    let tmp = crate::symbols::symbol_node(symbol_name(d.car()));
                    let def = crate::heap::cons(defmacro, crate::heap::cons(tmp, d.cdr()));
                    super::eval(def);
                    let f = macro_function(tmp).unwrap_or_else(|| err_form("Invalid MACROLET definition", d));
                    self.scope.push(Scope::Macro(d.car(), f));
                }
                let (specials, body) = self.parse_body(args.cdr(), false);
                self.push_free_specials(&specials, &[]);
                let n = self.convert_progn(body);
                self.scope.truncate(mark);
                n
            }
            Sop::Progv => {
                // (%PROGV symbols values #'(lambda () . body)); the
                // primitive does the binding (runtime::progv_prim).
                let l = crate::heap::list;
                if list_len(args) < 2 {
                    err_form("Invalid PROGV form", x);
                }
                let body = args.cdr().cdr();
                let thunk = crate::heap::cons(self.syms.lambda.lambda, crate::heap::cons(nil(), body));
                let f = l(&[find_symbol("FUNCTION"), thunk]);
                self.convert(l(&[find_symbol("%PROGV"), a1, nth(args, 1), f]))
            }
        }
    }

    fn convert_function(&mut self, f: LispObj, form: LispObj) -> Node {
        if f.is_symbol() {
            if let Some(v) = self.lookup_fun(f) {
                return Node::LexRef(v);
            }
            return Node::FunctionGlobal(f);
        }
        if f.is_cons() && f.car() == self.syms.lambda.lambda && f.cdr().is_cons() {
            let id = self.convert_lambda(f.cdr().car(), f.cdr().cdr(), nil(), true);
            return Node::Lambda(id);
        }
        err_form("Invalid FUNCTION form", form)
    }

    fn convert_setq(&mut self, args: LispObj) -> Node {
        let items = list_items(args);
        if !items.len().is_multiple_of(2) {
            err_form("Odd number of arguments to SETQ", args);
        }
        let mut v = Vec::new();
        for pair in items.chunks(2) {
            let (sym, val) = (pair[0], pair[1]);
            if !sym.is_symbol() {
                err_form("Cannot SETQ", sym);
            }
            if sym == nil() || is_constant_symbol(sym) {
                err_form("Cannot assign to constant", sym);
            }
            let n = match self.lookup_var(sym) {
                VarRef::Lex(id) => Node::LexSet(id, Box::new(self.convert(val))),
                VarRef::Special => Node::SpecialSet(sym, Box::new(self.convert(val))),
                VarRef::SymMacro(e) => {
                    let setf = find_symbol("SETF");
                    let f = crate::heap::list(&[setf, e, val]);
                    self.convert(f)
                }
            };
            v.push(n);
        }
        match v.len() {
            0 => Node::Const(nil()),
            1 => v.pop().unwrap(),
            _ => Node::Progn(v),
        }
    }

    fn convert_let(&mut self, bindings: LispObj, body: LispObj, sequential: bool) -> Node {
        let (specials, body) = self.parse_body(body, false);
        let mark = self.scope.len();
        let mut binds = Vec::new();
        let mut bound = Vec::new();
        let items = list_items(bindings);
        let parsed: Vec<(LispObj, LispObj)> = items
            .iter()
            .map(|&b| {
                if b.is_symbol() {
                    (b, nil())
                } else if b.is_cons() && b.car().is_symbol() {
                    (b.car(), nth(b, 1))
                } else {
                    err_form("Invalid LET binding", b)
                }
            })
            .collect();
        if sequential {
            for (sym, init) in parsed {
                let n = self.convert(init);
                let b = self.bind(sym, &specials);
                bound.push(sym);
                binds.push((b, n));
            }
        } else {
            let inits: Vec<Node> = parsed.iter().map(|&(_, i)| self.convert(i)).collect();
            for ((sym, _), n) in parsed.into_iter().zip(inits) {
                let b = self.bind(sym, &specials);
                bound.push(sym);
                binds.push((b, n));
            }
        }
        self.push_free_specials(&specials, &bound);
        let body = self.convert_progn(body);
        self.scope.truncate(mark);
        Node::Let {
            binds,
            sequential,
            body: Box::new(body),
        }
    }

    /// Keep leading declarations / doc string, wrap the rest in a block.
    fn wrap_block(&self, name: LispObj, body: LispObj) -> LispObj {
        let mut head = Vec::new();
        let mut b = body;
        while b.is_cons() {
            let f = b.car();
            let is_decl = f.is_cons() && f.car() == self.syms.declare;
            let is_doc = f.is_string() && b.cdr().is_cons();
            if !(is_decl || is_doc) {
                break;
            }
            head.push(f);
            b = b.cdr();
        }
        let block = crate::heap::cons(find_symbol("BLOCK"), crate::heap::cons(name, b));
        head.push(block);
        crate::heap::list(&head)
    }

    fn convert_flet(&mut self, defs: LispObj, body: LispObj, labels: bool) -> Node {
        let defs = list_items(defs);
        let (specials, body) = self.parse_body(body, false);
        let mark = self.scope.len();
        let mut parsed = Vec::new();
        for d in &defs {
            if !d.is_cons() || !d.car().is_symbol() || !d.cdr().is_cons() {
                err_form("Invalid function definition", *d);
            }
            parsed.push((d.car(), d.cdr().car(), self.wrap_block(d.car(), d.cdr().cdr())));
        }
        let node = if labels {
            let vars: Vec<VarId> = parsed
                .iter()
                .map(|&(name, _, _)| {
                    let id = self.new_var(name);
                    self.scope.push(Scope::Fun(name, id));
                    id
                })
                .collect();
            let mut sets = Vec::new();
            for (&(name, ll, b), &v) in parsed.iter().zip(&vars) {
                let l = self.convert_lambda(ll, b, name, true);
                sets.push(Node::LexSet(v, Box::new(Node::Lambda(l))));
            }
            self.push_free_specials(&specials, &[]);
            sets.push(self.convert_progn(body));
            Node::Let {
                binds: vars.iter().map(|&v| (Bind::Lex(v), Node::Const(nil()))).collect(),
                sequential: false,
                body: Box::new(Node::Progn(sets)),
            }
        } else {
            let lambdas: Vec<LambdaId> = parsed
                .iter()
                .map(|&(name, ll, b)| self.convert_lambda(ll, b, name, true))
                .collect();
            let mut binds = Vec::new();
            for (&(name, _, _), l) in parsed.iter().zip(lambdas) {
                let id = self.new_var(name);
                self.scope.push(Scope::Fun(name, id));
                binds.push((Bind::Lex(id), Node::Lambda(l)));
            }
            self.push_free_specials(&specials, &[]);
            let body = self.convert_progn(body);
            Node::Let {
                binds,
                sequential: false,
                body: Box::new(body),
            }
        };
        self.scope.truncate(mark);
        node
    }

    fn convert_tagbody(&mut self, items: LispObj) -> Node {
        let items = list_items(items);
        let tags: Vec<LispObj> = items
            .iter()
            .copied()
            .filter(|t| t.is_symbol() || t.is_fixnum())
            .collect();
        for (i, t) in tags.iter().enumerate() {
            if tags[..i].contains(t) {
                err_form("Duplicate tag in TAGBODY", *t);
            }
        }
        let tag_var = self.new_var(nil());
        self.tagbodies.push(TagbodyInfo {
            lambda: self.cur,
            up_depth: self.up_depth(),
            tags: tags.clone(),
            nonlocal: false,
            tag_var,
        });
        let id = self.tagbodies.len() - 1;
        let mark = self.scope.len();
        self.scope.push(Scope::Tags(id));
        let mut out = Vec::new();
        for it in items {
            if it.is_symbol() || it.is_fixnum() {
                let i = tags.iter().position(|&t| t == it).unwrap();
                out.push(TagItem::Tag(i));
            } else {
                out.push(TagItem::Form(self.convert(it)));
            }
        }
        self.scope.truncate(mark);
        Node::Tagbody(id, out)
    }

    fn convert_lambda(
        &mut self,
        ll: LispObj,
        body: LispObj,
        name: LispObj,
        allow_doc: bool,
    ) -> LambdaId {
        self.lambdas.push(LambdaInfo {
            parent: if self.lambdas.is_empty() { None } else { Some(self.cur) },
            name,
            free: Vec::new(),
            req: Vec::new(),
            opt: Vec::new(),
            rest: None,
            has_keys: false,
            keys: Vec::new(),
            allow_other_keys: false,
            aux: Vec::new(),
            body: Node::Const(nil()),
            up_depth: 0,
        });
        let id = self.lambdas.len() - 1;
        let saved = self.cur;
        self.cur = id;
        let mark = self.scope.len();
        let (specials, body) = self.parse_body(body, allow_doc);

        #[derive(PartialEq)]
        enum St {
            Req,
            Opt,
            Rest,
            Key,
            Aux,
        }
        let ls = &self.syms.lambda;
        let (l_opt, l_rest, l_body, l_key, l_aux, l_aok) =
            (ls.optional, ls.rest, ls.body, ls.key, ls.aux, ls.allow_other_keys);
        let mut st = St::Req;
        let mut bound = Vec::new();
        let mut req = Vec::new();
        let mut opt = Vec::new();
        let mut rest = None;
        let mut has_keys = false;
        let mut keys = Vec::new();
        let mut aok = false;
        let mut aux = Vec::new();
        let mut p = ll;
        while p.is_cons() {
            let x = p.car();
            p = p.cdr();
            if x == l_opt {
                st = St::Opt;
                continue;
            } else if x == l_rest || x == l_body {
                st = St::Rest;
                continue;
            } else if x == l_key {
                st = St::Key;
                has_keys = true;
                continue;
            } else if x == l_aux {
                st = St::Aux;
                continue;
            } else if x == l_aok {
                aok = true;
                continue;
            }
            match st {
                St::Req => {
                    req.push(self.bind(x, &specials));
                    bound.push(x);
                }
                St::Opt | St::Key => {
                    let (spec, init, svar) = if x.is_cons() {
                        (x.car(), nth(x, 1), nth(x, 2))
                    } else {
                        (x, nil(), nil())
                    };
                    let (key, sym) = if spec.is_cons() {
                        (spec.car(), nth(spec, 1))
                    } else if spec.is_symbol() {
                        (find_keyword(&symbol_name(spec).string_to_rust()), spec)
                    } else {
                        err_form("Invalid lambda list element", x)
                    };
                    let init = self.convert(init);
                    let var = self.bind(sym, &specials);
                    bound.push(sym);
                    let svar = if svar != nil() {
                        bound.push(svar);
                        Some(self.bind(svar, &specials))
                    } else {
                        None
                    };
                    if st == St::Opt {
                        opt.push(OptParam { var, init, svar });
                    } else {
                        keys.push(KeyParam {
                            key,
                            var,
                            init,
                            svar,
                        });
                    }
                }
                St::Rest => {
                    if rest.is_some() {
                        err_form("Invalid &REST", ll);
                    }
                    rest = Some(self.bind(x, &specials));
                    bound.push(x);
                }
                St::Aux => {
                    let (sym, init) = if x.is_cons() { (x.car(), nth(x, 1)) } else { (x, nil()) };
                    let init = self.convert(init);
                    aux.push((self.bind(sym, &specials), init));
                    bound.push(sym);
                }
            }
        }
        if p != nil() {
            err_form("Invalid lambda list", ll);
        }
        self.push_free_specials(&specials, &bound);
        let body = self.convert_progn(body);
        let info = &mut self.lambdas[id];
        info.req = req;
        info.opt = opt;
        info.rest = rest;
        info.has_keys = has_keys;
        info.keys = keys;
        info.allow_other_keys = aok;
        info.aux = aux;
        info.body = body;
        self.scope.truncate(mark);
        self.cur = saved;
        id
    }
}
