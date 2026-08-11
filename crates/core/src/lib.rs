// Every builder struct carries a `PhantomData<fn() -> (D, Scope, ...)>`
// marker tuple to hold its compile-time-only type tags — clippy's
// `type_complexity` lint fires once a tuple gets past 2-3 elements, but
// there's no simpler form to "factor out" here: a `type Marker<A,B,C> =
// (A,B,C)` alias would just move the same tuple one level of indirection
// away without reducing anything real. This is a deliberate, repeated
// pattern (see the design plan's scope/select modules), not a one-off
// complexity smell, so it's suppressed crate-wide rather than
// `#[allow]`-annotated at each of the dozen or so occurrences.
#![allow(clippy::type_complexity)]

pub mod cte;
pub mod delete;
pub mod dialect;
pub mod expr;
pub mod insert;
pub mod prepare;
pub mod raw;
pub mod render;
pub mod scope;
pub mod select;
pub mod update;
pub mod window;
pub mod with_macro;
