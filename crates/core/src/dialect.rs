//! Dialect markers and capability gating.
//!
//! Dialect-specific capabilities (Postgres `.returning()`, MySQL's lack of
//! it, etc.) are expressed as sealed marker traits implemented only for the
//! dialects that actually support them, so calling an unsupported method is
//! a compile error rather than a silent runtime gap or a documentation
//! footnote (contrast Drizzle, which mostly just documents around this).
//!
//! Minimum supported version per dialect (verified independently rather
//! than copied from any other project's compatibility matrix, per the
//! design plan's Phase 0 findings — e.g. the common claim "SQLite has no
//! RIGHT/FULL JOIN" is stale as of SQLite 3.39, 2022):
//! - **Postgres**: any currently-supported version (`RETURNING` since 8.2,
//!   `ON CONFLICT` since 9.5 — both far below any version still in use).
//! - **MySQL**: 8.0+ (needed for window functions/CTEs down the line;
//!   `RETURNING`/`ON CONFLICT DO UPDATE` are simply not part of MySQL's SQL
//!   dialect at any version — see `SupportsReturning`/`SupportsOnConflict`
//!   below — MySQL's own equivalent is `ON DUPLICATE KEY UPDATE`, added as
//!   its own capability trait rather than force-fit into `SupportsOnConflict`).
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

/// `RETURNING` support (Postgres, SQLite 3.35+ — not MySQL, which has no
/// equivalent SQL construct at any version).
pub trait SupportsReturning: Dialect {}
impl SupportsReturning for Postgres {}
impl SupportsReturning for Sqlite {}

/// `ON CONFLICT DO UPDATE/NOTHING` support (Postgres, SQLite — MySQL uses
/// the differently-shaped `ON DUPLICATE KEY UPDATE`, its own capability
/// trait, added when upsert support lands).
pub trait SupportsOnConflict: Dialect {}
impl SupportsOnConflict for Postgres {}
impl SupportsOnConflict for Sqlite {}

/// `RIGHT JOIN` support: Postgres always; MySQL always (it lacks `FULL
/// JOIN` but has always supported `RIGHT JOIN`); SQLite only 3.39+
/// (2022-06). Kept separate from `SupportsFullOuterJoin` specifically
/// because MySQL supports one but not the other — folding them into one
/// capability would either wrongly grant MySQL `FULL JOIN` or wrongly deny
/// it `RIGHT JOIN`.
pub trait SupportsRightJoin: Dialect {}
impl SupportsRightJoin for Postgres {}
impl SupportsRightJoin for MySql {}
impl SupportsRightJoin for Sqlite {}

/// `FULL JOIN` support: Postgres always; SQLite 3.39+ (2022-06); **not**
/// MySQL at any version — it has no native `FULL JOIN` (the common
/// workaround is a `UNION` of `LEFT JOIN` and `RIGHT JOIN`, which is a
/// different SQL shape entirely, not something `.full_join()` should
/// silently rewrite into).
pub trait SupportsFullOuterJoin: Dialect {}
impl SupportsFullOuterJoin for Postgres {}
impl SupportsFullOuterJoin for Sqlite {}

/// An internal rendering-only wrapper: same identifier quoting as `D`, but
/// always emits `?` for placeholders regardless of `D`'s normal style.
/// Used when rendering a subquery destined to be spliced into a larger
/// query as an `ExprKind::Raw` fragment (see `select::Select::exists`) —
/// the fragment's placeholders get renumbered into the *outer* query's
/// sequence at splice time (the same mechanism `sql!{}` already uses), so
/// the subquery must not pre-commit to Postgres-style `$N` numbers that
/// would collide with (or be orphaned from) the outer query's own count.
pub(crate) struct RawEmbed<D>(std::marker::PhantomData<D>);
impl<D: Dialect> private::Sealed for RawEmbed<D> {}
impl<D: Dialect> Dialect for RawEmbed<D> {
    const IDENTIFIER_QUOTE: char = D::IDENTIFIER_QUOTE;
    // Uses the default `?` placeholder.
}
