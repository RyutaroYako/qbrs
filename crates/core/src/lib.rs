// Every builder struct carries a `PhantomData<fn() -> (D, Scope, ...)>`
// marker tuple of compile-time-only type tags. That's a deliberate,
// repeated pattern rather than a complexity smell, and there's no simpler
// form to factor it into, so the lint is suppressed crate-wide.
#![allow(clippy::type_complexity)]

pub mod cte;
pub mod delete;
pub mod dialect;
pub mod expr;
pub mod insert;
pub mod prepare;
pub mod raw;
pub mod render;
pub mod row;
pub mod scope;
pub mod select;
pub mod update;
pub mod window;
