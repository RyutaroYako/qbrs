//! Execution integration between qbrs's type-safe query builder and a real
//! Postgres via `sqlx`. This crate owns *only* the value-binding and
//! row-decoding glue — the query building, SQL rendering, and all
//! compile-time safety guarantees live in `qbrs-core` and stay entirely
//! independent of any particular async runtime or driver, mirroring how
//! sea-query pairs with sea-query-binder rather than owning a driver layer
//! itself (see the design plan section 6).

use qbrs_core::delete::{Delete, DeleteReturning};
use qbrs_core::dialect::Postgres;
use qbrs_core::expr::{Column, Expr, SqlType, Value};
use qbrs_core::insert::{Insert, InsertReturning, InsertRow};
use qbrs_core::scope::{Superset, Table};
use qbrs_core::select::{DynSelect, Prepared, PreparedParams, Select, Selection, SetOp};
use qbrs_core::update::{Update, UpdateReturning};
use sqlx::Row;
use sqlx::postgres::PgRow;

/// Errors from executing a qbrs query against Postgres via `sqlx`.
///
/// Kept as a real enum (rather than surfacing raw `sqlx::Error` for
/// everything) so a qbrs-level misuse — an unresolved `prepare!{}`
/// placeholder — is distinguishable from an actual driver/database error
/// without string-matching a message. Previously both cases collapsed into
/// `sqlx::Error::Configuration(String)`, which looked identical to a real
/// sqlx-level configuration problem.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A real error from Postgres or the `sqlx` driver: a failed
    /// connection, constraint violation, decode failure, etc.
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),

    /// A named placeholder built via `prepare!{}` was never resolved
    /// before execution — either `Prepared::resolve()` found no matching
    /// field in `Params`, or `.load()`/`.execute()` was called directly on
    /// a query still holding an unresolved `Value::Placeholder` instead of
    /// going through `.prepare()` + `Prepared::resolve()`. Not a driver
    /// error, so kept out of the `Sqlx` variant.
    #[error(transparent)]
    UnresolvedPlaceholder(#[from] qbrs_core::select::UnresolvedPlaceholder),
}

/// This crate's `Result`, parameterized only over the success type — same
/// shape as `sqlx::Result`, with `qbrs_sqlx::Error` as the fixed error type.
pub type Result<T> = std::result::Result<T, Error>;

/// Binds a closed `Value` to a real Postgres query parameter. Typed `NullX`
/// variants (see `qbrs_core::expr::Value`'s doc comment) are what make this
/// possible without knowing the surrounding column's type separately —
/// binding a bare untyped NULL can fail Postgres's query planner even
/// though the value itself is NULL, because the wire protocol still
/// declares a parameter type.
///
/// Fallible because of `Value::Placeholder`: reaching this function means
/// a `prepare!{}`-style named placeholder was never resolved via
/// `Prepared::resolve()` before execution (e.g. `.load()` was called
/// directly on a query built with the low-level, doc-hidden
/// `expr::placeholder()` instead of going through `.prepare()`) — a real
/// misuse this crate can't prevent at compile time, so it's surfaced as an
/// `Error::UnresolvedPlaceholder` here rather than silently binding the
/// wrong thing or panicking.
fn bind_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    v: Value,
) -> Result<sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>> {
    Ok(match v {
        Value::I32(x) => query.bind(x),
        Value::I64(x) => query.bind(x),
        Value::F64(x) => query.bind(x),
        Value::Text(x) => query.bind(x),
        Value::Bool(x) => query.bind(x),
        Value::Bytes(x) => query.bind(x),
        Value::NullI32 => query.bind(None::<i32>),
        Value::NullI64 => query.bind(None::<i64>),
        Value::NullF64 => query.bind(None::<f64>),
        Value::NullText => query.bind(None::<String>),
        Value::NullBool => query.bind(None::<bool>),
        Value::NullBytes => query.bind(None::<Vec<u8>>),
        Value::Placeholder(name) => {
            return Err(qbrs_core::select::UnresolvedPlaceholder(name).into());
        }
    })
}

fn bind_all<'q>(
    mut query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    params: Vec<Value>,
) -> Result<sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>> {
    for p in params {
        query = bind_value(query, p)?;
    }
    Ok(query)
}

/// Decodes a `Selection<Scope, Idx>` positionally out of a real `PgRow`.
/// Kept as a separate trait (rather than folded into `Selection` itself) so
/// `qbrs-core` never has to depend on `sqlx`; every impl here just adds a
/// `sqlx::Decode`/`Type` bound on top of an existing `Selection` impl,
/// which is also what makes join-derived `Option<T>` wrapping "just work"
/// here for free — `Selection::Output` already resolved that, and
/// `Option<T>: Decode` has a blanket impl in sqlx itself.
pub trait PgDecode<Scope, Idx>: Selection<Scope, Idx> {
    #[doc(hidden)]
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Output>;
}

impl<T: Table, S: SqlType, Scope, Idx> PgDecode<Scope, Idx> for Column<T, S>
where
    Self: Selection<Scope, Idx>,
    <Self as Selection<Scope, Idx>>::Output:
        for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Output> {
        let v = row.try_get::<Self::Output, _>(*idx)?;
        *idx += 1;
        Ok(v)
    }
}

impl<Req, S: SqlType, Scope, Idx> PgDecode<Scope, Idx> for Expr<Req, S>
where
    Scope: Superset<Req, Idx>,
    S::Native: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Output> {
        let v = row.try_get::<S::Native, _>(*idx)?;
        *idx += 1;
        Ok(v)
    }
}

impl<Scope, Idx, A: PgDecode<Scope, Idx>> PgDecode<Scope, Idx> for (A,) {
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Output> {
        Ok((A::decode_at(row, idx)?,))
    }
}

impl<Scope, IdxA, IdxB, A: PgDecode<Scope, IdxA>, B: PgDecode<Scope, IdxB>>
    PgDecode<Scope, (IdxA, IdxB)> for (A, B)
{
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Output> {
        Ok((A::decode_at(row, idx)?, B::decode_at(row, idx)?))
    }
}

impl<
    Scope,
    IdxA,
    IdxB,
    IdxC,
    A: PgDecode<Scope, IdxA>,
    B: PgDecode<Scope, IdxB>,
    C: PgDecode<Scope, IdxC>,
> PgDecode<Scope, (IdxA, IdxB, IdxC)> for (A, B, C)
{
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Output> {
        Ok((
            A::decode_at(row, idx)?,
            B::decode_at(row, idx)?,
            C::decode_at(row, idx)?,
        ))
    }
}

impl<
    Scope,
    IdxA,
    IdxB,
    IdxC,
    IdxD,
    A: PgDecode<Scope, IdxA>,
    B: PgDecode<Scope, IdxB>,
    C: PgDecode<Scope, IdxC>,
    D: PgDecode<Scope, IdxD>,
> PgDecode<Scope, (IdxA, IdxB, IdxC, IdxD)> for (A, B, C, D)
{
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Output> {
        Ok((
            A::decode_at(row, idx)?,
            B::decode_at(row, idx)?,
            C::decode_at(row, idx)?,
            D::decode_at(row, idx)?,
        ))
    }
}

impl<
    Scope,
    IdxA,
    IdxB,
    IdxC,
    IdxD,
    IdxE,
    A: PgDecode<Scope, IdxA>,
    B: PgDecode<Scope, IdxB>,
    C: PgDecode<Scope, IdxC>,
    D: PgDecode<Scope, IdxD>,
    E: PgDecode<Scope, IdxE>,
> PgDecode<Scope, (IdxA, IdxB, IdxC, IdxD, IdxE)> for (A, B, C, D, E)
{
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Output> {
        Ok((
            A::decode_at(row, idx)?,
            B::decode_at(row, idx)?,
            C::decode_at(row, idx)?,
            D::decode_at(row, idx)?,
            E::decode_at(row, idx)?,
        ))
    }
}

/// Generic over `E: sqlx::PgExecutor` (rather than hardcoding `&PgPool`) so
/// every public `.load()`/`.execute()` method here works unchanged against
/// either a plain `&PgPool` or a `&mut sqlx::PgTransaction<'_>` — sqlx
/// itself only implements `Executor` for `&mut PgConnection` (which
/// `Transaction` derefs to), not `Transaction` directly, so callers pass
/// `&mut *tx`, matching sqlx's own transaction usage pattern.
async fn fetch_all<'e, Scope, Idx, Sel: PgDecode<Scope, Idx>, E: sqlx::PgExecutor<'e>>(
    executor: E,
    sql: &str,
    params: Vec<Value>,
) -> Result<Vec<Sel::Output>> {
    let rows = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql)), params)?
        .fetch_all(executor)
        .await?;
    rows.iter()
        .map(|row| Sel::decode_at(row, &mut 0).map_err(Error::from))
        .collect()
}

async fn fetch_optional<'e, Scope, Idx, Sel: PgDecode<Scope, Idx>, E: sqlx::PgExecutor<'e>>(
    executor: E,
    sql: &str,
    params: Vec<Value>,
) -> Result<Option<Sel::Output>> {
    let row = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql)), params)?
        .fetch_optional(executor)
        .await?;
    row.as_ref()
        .map(|r| Sel::decode_at(r, &mut 0).map_err(Error::from))
        .transpose()
}

async fn execute_only<'e, E: sqlx::PgExecutor<'e>>(
    executor: E,
    sql: &str,
    params: Vec<Value>,
) -> Result<u64> {
    let result = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql)), params)?
        .execute(executor)
        .await?;
    Ok(result.rows_affected())
}

/// Threading an explicit `Idx` parameter (rather than hiding it in a
/// `where` clause on a single-parameter trait) is the same fix the Phase 0
/// spike already needed for `scope::Superset` itself: Rust has no
/// existential quantification over impl generics, so an index used only in
/// a `where` clause (and not the trait's own parameter list) is an
/// unconstrained-type-parameter compile error (E0207). Callers never see
/// `Idx` — it's always inferred at the call site, exactly like `Find`'s and
/// `Superset`'s own indices.
pub trait LoadExt<Idx> {
    type Output;
    fn load<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Vec<Self::Output>>>;
    fn load_one<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Option<Self::Output>>>;
}

impl<Scope, Sel: PgDecode<Scope, Idx>, Idx> LoadExt<Idx> for Select<Postgres, Scope, Sel> {
    type Output = Sel::Output;

    async fn load<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<Vec<Self::Output>> {
        let (sql, params) = self.to_sql::<Idx>();
        fetch_all::<Scope, Idx, Sel, E>(executor, &sql, params).await
    }

    async fn load_one<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> Result<Option<Self::Output>> {
        let (sql, params) = self.to_sql::<Idx>();
        fetch_optional::<Scope, Idx, Sel, E>(executor, &sql, params).await
    }
}

pub trait ExecuteExt {
    fn execute<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<u64>>;
}

impl<T: Table, R: InsertRow<Table = T>> ExecuteExt for Insert<Postgres, T, R> {
    async fn execute<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<u64> {
        let (sql, params) = self.to_sql();
        execute_only(executor, &sql, params).await
    }
}

impl<T: Table> ExecuteExt for Update<Postgres, T> {
    async fn execute<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<u64> {
        let (sql, params) = self.to_sql();
        execute_only(executor, &sql, params).await
    }
}

impl<T: Table> ExecuteExt for Delete<Postgres, T> {
    async fn execute<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<u64> {
        let (sql, params) = self.to_sql();
        execute_only(executor, &sql, params).await
    }
}

pub trait LoadReturningExt<Idx> {
    type Output;
    fn load<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Vec<Self::Output>>>;
}

impl<T: Table, R: InsertRow<Table = T>, Sel, Idx> LoadReturningExt<Idx>
    for InsertReturning<Postgres, T, R, Sel>
where
    Sel: PgDecode<
            qbrs_core::scope::Cons<
                qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
                qbrs_core::scope::Nil,
            >,
            Idx,
        >,
{
    type Output = Sel::Output;
    async fn load<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<Vec<Self::Output>> {
        let (sql, params) = self.to_sql();
        fetch_all::<
            qbrs_core::scope::Cons<
                qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
                qbrs_core::scope::Nil,
            >,
            Idx,
            Sel,
            E,
        >(executor, &sql, params)
        .await
    }
}

impl<T: Table, Sel, Idx> LoadReturningExt<Idx> for UpdateReturning<Postgres, T, Sel>
where
    Sel: PgDecode<
            qbrs_core::scope::Cons<
                qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
                qbrs_core::scope::Nil,
            >,
            Idx,
        >,
{
    type Output = Sel::Output;
    async fn load<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<Vec<Self::Output>> {
        let (sql, params) = self.to_sql();
        fetch_all::<
            qbrs_core::scope::Cons<
                qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
                qbrs_core::scope::Nil,
            >,
            Idx,
            Sel,
            E,
        >(executor, &sql, params)
        .await
    }
}

impl<T: Table, Sel, Idx> LoadReturningExt<Idx> for DeleteReturning<Postgres, T, Sel>
where
    Sel: PgDecode<
            qbrs_core::scope::Cons<
                qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
                qbrs_core::scope::Nil,
            >,
            Idx,
        >,
{
    type Output = Sel::Output;
    async fn load<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<Vec<Self::Output>> {
        let (sql, params) = self.to_sql();
        fetch_all::<
            qbrs_core::scope::Cons<
                qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
                qbrs_core::scope::Nil,
            >,
            Idx,
            Sel,
            E,
        >(executor, &sql, params)
        .await
    }
}

/// Decodes a `DynSelect`'s `Output` positionally out of a `PgRow`. A
/// separate, narrower trait from `PgDecode` rather than a reuse of it:
/// once a query is erased via `.erase()`, only the plain-Rust `Output`
/// type survives (see `DynSelect`'s doc comment) — there's no `Selection`
/// impl left to piggyback decode logic on, so this is implemented directly
/// against the same closed set of native types `Selection`/`PgDecode`
/// cover, not derived from them.
pub trait DecodeRow: Sized {
    #[doc(hidden)]
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self>;
}

macro_rules! decode_row_leaf {
    ($ty:ty) => {
        impl DecodeRow for $ty {
            fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
                let v = row.try_get::<$ty, _>(*idx)?;
                *idx += 1;
                Ok(v)
            }
        }
        impl DecodeRow for Option<$ty> {
            fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
                let v = row.try_get::<Option<$ty>, _>(*idx)?;
                *idx += 1;
                Ok(v)
            }
        }
    };
}
decode_row_leaf!(i32);
decode_row_leaf!(i64);
decode_row_leaf!(f64);
decode_row_leaf!(String);
decode_row_leaf!(bool);
decode_row_leaf!(Vec<u8>);

impl<A: DecodeRow> DecodeRow for (A,) {
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
        Ok((A::decode_at(row, idx)?,))
    }
}
impl<A: DecodeRow, B: DecodeRow> DecodeRow for (A, B) {
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
        Ok((A::decode_at(row, idx)?, B::decode_at(row, idx)?))
    }
}
impl<A: DecodeRow, B: DecodeRow, C: DecodeRow> DecodeRow for (A, B, C) {
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
        Ok((
            A::decode_at(row, idx)?,
            B::decode_at(row, idx)?,
            C::decode_at(row, idx)?,
        ))
    }
}
impl<A: DecodeRow, B: DecodeRow, C: DecodeRow, D: DecodeRow> DecodeRow for (A, B, C, D) {
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
        Ok((
            A::decode_at(row, idx)?,
            B::decode_at(row, idx)?,
            C::decode_at(row, idx)?,
            D::decode_at(row, idx)?,
        ))
    }
}
impl<A: DecodeRow, B: DecodeRow, C: DecodeRow, D: DecodeRow, E: DecodeRow> DecodeRow
    for (A, B, C, D, E)
{
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
        Ok((
            A::decode_at(row, idx)?,
            B::decode_at(row, idx)?,
            C::decode_at(row, idx)?,
            D::decode_at(row, idx)?,
            E::decode_at(row, idx)?,
        ))
    }
}

pub trait LoadDynExt {
    type Output;
    fn load<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Vec<Self::Output>>>;
}

impl<Output: DecodeRow> LoadDynExt for DynSelect<Postgres, Output> {
    type Output = Output;
    async fn load<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<Vec<Output>> {
        let (sql, params) = self.to_sql();
        let rows = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql.as_str())), params)?
            .fetch_all(executor)
            .await?;
        rows.iter()
            .map(|row| Output::decode_at(row, &mut 0).map_err(Error::from))
            .collect()
    }
}

/// Loads a `UNION`/`INTERSECT`/`EXCEPT` chain (see
/// `qbrs_core::select::SetOp`) against real Postgres. Reuses the same
/// `DecodeRow` trait `LoadDynExt` uses — once combined via a set operator, a
/// `SetOp`'s branches no longer carry their individual `Scope`s/`Selection`
/// impls (they were already rendered to text at combine time), so there is
/// nothing left to decode against but the plain `Output` type, exactly the
/// erased situation `DynSelect` is already in.
pub trait LoadSetOpExt {
    type Output;
    fn load<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Vec<Self::Output>>>;
}

impl<Output: DecodeRow> LoadSetOpExt for SetOp<Postgres, Output> {
    type Output = Output;
    async fn load<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<Vec<Output>> {
        let (sql, params) = self.to_sql();
        let rows = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql.as_str())), params)?
            .fetch_all(executor)
            .await?;
        rows.iter()
            .map(|row| Output::decode_at(row, &mut 0).map_err(Error::from))
            .collect()
    }
}

/// Executes a `prepare!{}`-built query against real Postgres, resolving its
/// named placeholders from `params` first — see `qbrs_core::select::Prepared`.
/// The same `Prepared` value is meant to be reused across many `execute`
/// calls with different `params`, since `.resolve()` only clones the
/// template, not re-render the SQL text.
pub trait PreparedExt<Params> {
    type Output;
    fn execute<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
        params: Params,
    ) -> impl std::future::Future<Output = Result<Vec<Self::Output>>>;
}

impl<Params: PreparedParams, Output: DecodeRow> PreparedExt<Params> for Prepared<Params, Output> {
    type Output = Output;
    async fn execute<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
        params: Params,
    ) -> Result<Vec<Output>> {
        let (sql, values) = self.resolve(params)?;
        let rows = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql.as_str())), values)?
            .fetch_all(executor)
            .await?;
        rows.iter()
            .map(|row| Output::decode_at(row, &mut 0).map_err(Error::from))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // No real Postgres needed: `bind_all`/`bind_value` only inspect the
    // `Value` enum before ever reaching the network, so the misuse case
    // (executing a query with an unresolved `prepare!{}` placeholder) is
    // reachable — and its error type checkable — without a live DB.
    #[test]
    fn unresolved_placeholder_is_a_typed_error_not_a_sqlx_configuration_string() {
        let query = sqlx::query(sqlx::AssertSqlSafe("SELECT $1"));
        let err = match bind_all(query, vec![Value::Placeholder("email")]) {
            Err(e) => e,
            Ok(_) => panic!("unresolved placeholder must fail to bind"),
        };

        assert!(matches!(
            err,
            Error::UnresolvedPlaceholder(qbrs_core::select::UnresolvedPlaceholder("email"))
        ));
        // `Error` is a real `std::error::Error`, not just a `Debug`/`Display`
        // pair, so callers can use it with `anyhow`/`Box<dyn Error>`/etc.
        let _: &dyn std::error::Error = &err;
        assert_eq!(err.to_string(), "no value provided for placeholder `email`");
    }

    #[test]
    fn sqlx_errors_convert_via_from() {
        let err: Error = sqlx::Error::RowNotFound.into();
        assert!(matches!(err, Error::Sqlx(sqlx::Error::RowNotFound)));
    }
}
