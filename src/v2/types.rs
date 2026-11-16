//! Semantic types of the v2 pipeline.

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Int,
    Bool,
    Str,
    Void,
    /// dynamically typed value: a boxed `(tag, payload)` pair
    Any,
    /// type of an expression that never completes normally
    Never,
    /// untyped 64-bit word; only the runtime prelude uses it
    Raw,
    /// not yet known (payload of a bare `None`, error side of a bare `Ok`)
    Infer,
    Option(Box<Ty>),
    Result(Box<Ty>, Box<Ty>),
    Tuple(Vec<Ty>),
    List(Box<Ty>),
    /// `Future<T>` and `Task<T>`
    Future(Box<Ty>),
    Channel(Box<Ty>),
    AsyncGen(Box<Ty>),
    Fn(Vec<Ty>, Box<Ty>),
    /// reference to an actor of `objects[i]` (also the head of a pipeline)
    Actor(usize),
    /// supervisor instance of `objects[i]`
    Object(usize),
    Observable(Box<Ty>),
    Subscription,
    Span,
    /// the `mail` parameter of a receive handler
    Mail(Box<Ty>),
}

impl Ty {
    pub fn has_hole(&self) -> bool {
        match self {
            Ty::Infer => true,
            Ty::Option(a) | Ty::List(a) | Ty::Future(a) | Ty::Channel(a) | Ty::AsyncGen(a) | Ty::Observable(a) | Ty::Mail(a) => {
                a.has_hole()
            }
            Ty::Result(a, b) => a.has_hole() || b.has_hole(),
            Ty::Tuple(v) => v.iter().any(Ty::has_hole),
            Ty::Fn(p, r) => p.iter().any(Ty::has_hole) || r.has_hole(),
            _ => false,
        }
    }
}

/// `actual` may be used where `want` is expected (ignoring `Any` boxing,
/// which the checker applies separately).
pub fn fits(actual: &Ty, want: &Ty) -> bool {
    match (actual, want) {
        (Ty::Raw, _) | (_, Ty::Raw) | (Ty::Never, _) | (Ty::Infer, _) | (_, Ty::Infer) => true,
        (Ty::Option(a), Ty::Option(b))
        | (Ty::List(a), Ty::List(b))
        | (Ty::Future(a), Ty::Future(b))
        | (Ty::Channel(a), Ty::Channel(b))
        | (Ty::AsyncGen(a), Ty::AsyncGen(b))
        | (Ty::Observable(a), Ty::Observable(b))
        | (Ty::Mail(a), Ty::Mail(b)) => fits(a, b),
        (Ty::Result(a, b), Ty::Result(c, d)) => fits(a, c) && fits(b, d),
        (Ty::Tuple(a), Ty::Tuple(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| fits(x, y)),
        (Ty::Fn(p, r), Ty::Fn(q, s)) => p.len() == q.len() && p.iter().zip(q).all(|(x, y)| fits(x, y)) && fits(r, s),
        _ => actual == want,
    }
}

/// The more specific of two compatible types (fills `Infer` holes).
pub fn merge(a: &Ty, b: &Ty) -> Ty {
    match (a, b) {
        (Ty::Infer, x) | (x, Ty::Infer) | (Ty::Never, x) | (x, Ty::Never) => x.clone(),
        (Ty::Option(x), Ty::Option(y)) => Ty::Option(Box::new(merge(x, y))),
        (Ty::List(x), Ty::List(y)) => Ty::List(Box::new(merge(x, y))),
        (Ty::Future(x), Ty::Future(y)) => Ty::Future(Box::new(merge(x, y))),
        (Ty::Channel(x), Ty::Channel(y)) => Ty::Channel(Box::new(merge(x, y))),
        (Ty::Result(x, e), Ty::Result(y, f)) => Ty::Result(Box::new(merge(x, y)), Box::new(merge(e, f))),
        (Ty::Tuple(x), Ty::Tuple(y)) if x.len() == y.len() => Ty::Tuple(x.iter().zip(y).map(|(p, q)| merge(p, q)).collect()),
        _ => a.clone(),
    }
}
