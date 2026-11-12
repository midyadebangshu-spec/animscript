//! Syntax tree. Blocks are reference counted so running tasks can hold on to them cheaply.
use animscript_core::Span;
use std::rc::Rc;

pub type Block = Rc<Vec<Stmt>>;

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Num(String),
    Unit {
        num: String,
        unit: String,
    },
    Str(String),
    Bool(bool),
    Color([u8; 4]),
    Name(String),
    Member(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Arg>),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Tuple(Vec<Expr>),
    List(Vec<Expr>),
    /// Inclusive range `a..b`.
    Range(Box<Expr>, Box<Expr>),
    /// `v -> expr` or `(v, i) -> expr`.
    Lambda {
        params: Vec<String>,
        body: Rc<Expr>,
    },
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Let(String, Expr),
    ShapeDecl {
        name: String,
        expr: Expr,
        body: Option<Block>,
    },
    Assign(Expr, Expr),
    Expr {
        expr: Expr,
        over: Option<Expr>,
        ease: Option<Expr>,
    },
    For {
        var: String,
        iter: Expr,
        body: Block,
    },
    If {
        cond: Expr,
        then: Block,
        else_: Option<Block>,
    },
    Repeat {
        count: Expr,
        body: Block,
    },
    Wait(Expr),
    Together(Block),
    /// `[let name =] start CALL`: begin an action or modifier without waiting for it.
    Start {
        name: Option<String>,
        call: Expr,
        over: Option<Expr>,
        ease: Option<Expr>,
    },
    Cancel(Expr),
    Await(Expr),
    /// `target.keyframes over D:` followed by key blocks.
    Keyframes {
        target: Expr,
        over: Expr,
        opts: KeyOpts,
        keys: Vec<KeyBlock>,
    },
    Become {
        target: String,
        flag: String,
        set: bool,
    },
    Note(String),
    Assert(Expr),
    Pass,
}

/// One `prop = value` (or `prop += value`) line of a key block, with an optional easing.
#[derive(Clone, Debug)]
pub struct KeyAssign {
    pub lhs: Expr,
    pub add: bool,
    pub value: Expr,
    pub ease: Option<Expr>,
    pub span: Span,
}

/// `at TIME:` and the assignments under it.
#[derive(Clone, Debug)]
pub struct KeyBlock {
    pub at: Expr,
    pub assigns: Vec<KeyAssign>,
    pub span: Span,
}

#[derive(Clone, Debug, Default)]
pub struct KeyOpts {
    pub hold_before: bool,
    pub hold_after: bool,
    pub blend_in: Option<Expr>,
    pub blend_out: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    pub anim_version: Option<String>,
    pub canvas: (u32, u32),
    pub fps: (i64, i64),
    pub seed: i64,
    pub background: Option<[u8; 4]>,
    pub span: Span,
}

/// The `world:` block (physics settings); each value is an expression evaluated while building.
#[derive(Clone, Debug)]
pub struct WorldDecl {
    pub gravity: Option<Expr>,
    pub scale: Option<Expr>,
    pub substeps: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EntityDecl {
    pub kind: String,
    pub name: String,
    pub states: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    When,
    Whenever,
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub kind: RuleKind,
    pub trigger: Expr,
    pub guard: Option<Expr>,
    pub priority: i64,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum SceneLen {
    Auto,
    Expr(Expr),
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub name: String,
    pub length: SceneLen,
    pub setup: Option<Block>,
    pub main: Option<Block>,
    pub timeline: Vec<KeyBlock>,
    pub rules: Vec<Rule>,
    pub span: Span,
}

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub project: Option<Project>,
    pub world: Option<WorldDecl>,
    pub entities: Vec<EntityDecl>,
    pub top: Vec<Stmt>,
    pub scenes: Vec<Rc<Scene>>,
}
