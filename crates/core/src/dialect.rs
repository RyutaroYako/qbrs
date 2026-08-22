//! Dialect markers and capability gating.
//!
//! Dialect-specific capabilities (Postgres `.returning()`, MySQL's lack of
//! it, etc.) are sealed marker traits implemented only for the dialects that
//! actually support them, so calling an unsupported method is a compile
//! error rather than a documentation footnote.
//!
//! Minimum supported version per dialect:
//! - **Postgres**: any currently-supported version.
//! - **MySQL**: 8.0+ (window functions/CTEs).
//! - **SQLite**: 3.39+ (2022-06), for `RIGHT`/`FULL JOIN` support.

mod private {
    pub trait Sealed {}
}

pub trait Dialect: 'static + private::Sealed {
    /// SQL identifier quote character, e.g. `"` for Postgres/SQLite, `` ` ``
    /// for MySQL.
    const IDENTIFIER_QUOTE: char;

    /// Renders the placeholder for the `n`th bound parameter (1-indexed).
    /// Postgres numbers them (`$1`, `$2`, ...); MySQL/SQLite are purely
    /// positional (`?` every time, matched by order of appearance).
    fn placeholder(n: usize) -> String {
        let _ = n;
        "?".to_string()
    }
}

pub struct Postgres;
impl private::Sealed for Postgres {}
impl Dialect for Postgres {
    const IDENTIFIER_QUOTE: char = '"';
    fn placeholder(n: usize) -> String {
        format!("${n}")
    }
}

pub struct MySql;
impl private::Sealed for MySql {}
impl Dialect for MySql {
    const IDENTIFIER_QUOTE: char = '`';
    // Uses the default `?` placeholder.
}

pub struct Sqlite;
impl private::Sealed for Sqlite {}
impl Dialect for Sqlite {
    const IDENTIFIER_QUOTE: char = '"';
    // Uses the default `?` placeholder.
}

/// `RETURNING` support (Postgres, SQLite 3.35+). MySQL has no equivalent
/// SQL construct at any version.
pub trait SupportsReturning: Dialect {}
impl SupportsReturning for Postgres {}
impl SupportsReturning for Sqlite {}

/// `ON CONFLICT DO UPDATE/NOTHING` support (Postgres, SQLite). MySQL's
/// differently-shaped `ON DUPLICATE KEY UPDATE` gets its own capability
/// trait when it lands.
pub trait SupportsOnConflict: Dialect {}
impl SupportsOnConflict for Postgres {}
impl SupportsOnConflict for Sqlite {}

/// `RIGHT JOIN` support: Postgres and MySQL always, SQLite 3.39+. Kept
/// separate from `SupportsFullOuterJoin` because MySQL has this one but not
/// that one.
pub trait SupportsRightJoin: Dialect {}
impl SupportsRightJoin for Postgres {}
impl SupportsRightJoin for MySql {}
impl SupportsRightJoin for Sqlite {}

/// `FULL JOIN` support: Postgres always, SQLite 3.39+, **not** MySQL at any
/// version. The usual MySQL workaround is a `UNION` of `LEFT` and `RIGHT`
/// joins — a different SQL shape, not something `.full_join()` should
/// silently rewrite into.
pub trait SupportsFullOuterJoin: Dialect {}
impl SupportsFullOuterJoin for Postgres {}
impl SupportsFullOuterJoin for Sqlite {}

/// An internal rendering-only wrapper: same identifier quoting as `D`, but
/// always emits `?` for placeholders. Used for a fragment that will be
/// spliced into a larger query (a subquery, CTE body, or `UNION` branch) and
/// renumbered into the outer query's placeholder sequence — it must not
/// pre-commit to `$N` numbers that would collide with the outer count.
pub(crate) struct RawEmbed<D>(std::marker::PhantomData<D>);
impl<D: Dialect> private::Sealed for RawEmbed<D> {}
impl<D: Dialect> Dialect for RawEmbed<D> {
    const IDENTIFIER_QUOTE: char = D::IDENTIFIER_QUOTE;
    // Uses the default `?` placeholder.
}
