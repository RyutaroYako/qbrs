//! The query-building half of [qbrs](https://docs.rs/qbrs): scope tracking,
//! expressions, the `SELECT`/`INSERT`/`UPDATE`/`DELETE` builders, and the SQL
//! renderer. No I/O, no async runtime, no driver — and no dependency at all
//! unless a schema asks for one (the `chrono`, `uuid` and `decimal` features).
//!
//! Depend on `qbrs` rather than this crate directly; it re-exports everything
//! here alongside `#[derive(Table)]`, `#[derive(FromRow)]`, `label!` and
//! `with!`, which have to be proc macros and so live in `qbrs-macros`.
//!
//! [`scope`] is the load-bearing module: a query's FROM/JOIN set is a flat
//! type-level list, and every "is this column in scope, and is the join making
//! it nullable?" question is a lookup in it. Reading that module first makes
//! the rest of the crate follow.

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
pub mod statement;
pub mod update;
pub mod window;

#[cfg(test)]
mod tests {
    use crate::expr::Value;

    #[test]
    fn a_value_names_the_sql_type_it_came_from() {
        // `qbrs-sqlx` reports this name when a column type is enabled here
        // and not there, so every variant that can exist has to have one.
        assert_eq!(Value::I64(1).type_name(), "BigInt");
        assert_eq!(Value::NullText.type_name(), "Text");
        #[cfg(feature = "chrono")]
        assert_eq!(Value::NullTimestamptz.type_name(), "Timestamptz");
        #[cfg(feature = "decimal")]
        assert_eq!(Value::NullNumeric.type_name(), "Numeric");
    }
}
