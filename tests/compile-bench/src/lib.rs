//! Phase 0 spike: synthetic schema + escalating join-count scope chains, to
//! empirically test whether the flat cons-list `Find<T, Idx>` design avoids
//! diesel's documented exponential `cargo check` blowup (diesel#3223) as
//! join count grows. See `/home/ryutaro/.claude/plans/async-inventing-snail.md`
//! Phase 0 for the go/no-go criteria this is gating.

use qbrs_core::scope::{MapNullable, Superset, Table};

macro_rules! declare_tables {
    ($($name:ident),* $(,)?) => {
        $(
            pub struct $name;
            impl Table for $name {
                const NAME: &'static str = stringify!($name);
            }
        )*
    };
}

// 100 synthetic tables — enough headroom to stress-test well past diesel's
// documented ~7-join blowup point (diesel#3223), including a full-superset
// worst case where every single table is required at once (O(n^2) Find
// resolutions if this design is merely polynomial, astronomically worse if
// it's exponential like diesel's join-tree).
declare_tables!(
    T00, T01, T02, T03, T04, T05, T06, T07, T08, T09, T10, T11, T12, T13, T14, T15, T16, T17, T18,
    T19, T20, T21, T22, T23, T24, T25, T26, T27, T28, T29, T30, T31, T32, T33, T34, T35, T36, T37,
    T38, T39, T40, T41, T42, T43, T44, T45, T46, T47, T48, T49, T50, T51, T52, T53, T54, T55, T56,
    T57, T58, T59, T60, T61, T62, T63, T64, T65, T66, T67, T68, T69, T70, T71, T72, T73, T74, T75,
    T76, T77, T78, T79, T80, T81, T82, T83, T84, T85, T86, T87, T88, T89, T90, T91, T92, T93, T94,
    T95, T96, T97, T98, T99,
);

/// Build a nested `Cons<TableSlot<Head, NotNull>, ...>` scope type from a
/// list of table types, innermost (first FROM'd table) last.
#[macro_export]
macro_rules! scope_of {
    () => { $crate::__private::Nil };
    ($head:ty $(, $tail:ty)* $(,)?) => {
        $crate::__private::Cons<$crate::__private::TableSlot<$head, $crate::__private::NotNull>, scope_of!($($tail),*)>
    };
}

/// Build a bare `Cons<Head, ...>` list of table types with *no* `TableSlot`
/// wrapper — this is what `Superset`'s `Req` argument expects, since a
/// "which tables does this expression touch" requirement doesn't carry
/// nullability the way an actual query `Scope` does. Mixing the two up
/// (reusing `scope_of!` to build a `Req`) is a real mistake this spike
/// caught: `Superset`'s `Head: Table` bound would then require
/// `TableSlot<T, N>: Table`, which is never implemented, so every
/// `Superset` check silently fails with a "not in scope" error that looks
/// like a missing join instead of a Req/Scope type confusion.
#[macro_export]
macro_rules! req_of {
    () => { $crate::__private::Nil };
    ($head:ty $(, $tail:ty)* $(,)?) => {
        $crate::__private::Cons<$head, req_of!($($tail),*)>
    };
}

// Re-exported under a stable path so the `scope_of!` macro above can refer
// to these types hygienically regardless of what the calling crate has
// imported.
#[doc(hidden)]
pub mod __private {
    pub use qbrs_core::scope::{Cons, Nil, NotNull, TableSlot};
}

/// Generic proof helper: "does `S` contain `T`". Exercises `Find` without
/// needing the caller to name the inferred `Idx`.
pub fn assert_contains<S, T: Table, I>()
where
    S: qbrs_core::scope::Find<T, I>,
{
}

/// Exercises `Superset<Req, _>` for a multi-table requirement list.
pub fn assert_superset<S, Req, Idxs>()
where
    S: Superset<Req, Idxs>,
{
}

/// Exercises `MapNullable` (the RIGHT/FULL JOIN nullability-flip operation)
/// over a scope of arbitrary depth.
pub fn assert_map_nullable<S: MapNullable>() {}
