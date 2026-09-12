//! Execution integration between qbrs's query builder and a real Postgres
//! via `sqlx`. This crate owns only the value-binding and row-decoding glue;
//! query building, SQL rendering, and every compile-time guarantee live in
//! `qbrs-core`, which stays independent of any async runtime or driver.

/// Re-exported so a caller can name what `LoadExt::stream` returns and
/// consume it — `StreamExt::next` is how a stream is read — without taking
/// a `futures` dependency of their own.
pub use futures_util::{Stream, StreamExt};
use qbrs_core::delete::Delete;
use qbrs_core::dialect::Postgres;
use qbrs_core::expr::Value;
use qbrs_core::insert::Insert;
use qbrs_core::row::{Row, RowCons, RowNil};
use qbrs_core::select::{DynSelect, Prepared, PreparedParams, Select, Selection, SetOp, Total};
use qbrs_core::statement::{Returning, Statement, WrittenTable};
use qbrs_core::update::Update;
use sqlx::Row as _;
use sqlx::postgres::PgRow;

/// Errors from executing a qbrs query against Postgres via `sqlx`. An enum
/// rather than a bare `sqlx::Error` so a qbrs-level misuse is distinguishable
/// from a driver/database error without string-matching a message.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A real error from Postgres or the `sqlx` driver: a failed
    /// connection, constraint violation, decode failure, etc.
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),

    /// A `prepare!{}` placeholder reached execution unresolved: either
    /// `Prepared::resolve()` found no matching field in `Params`, or the
    /// query was executed directly instead of through `.prepare()`.
    #[error(transparent)]
    UnresolvedPlaceholder(#[from] qbrs_core::select::UnresolvedPlaceholder),

    /// An `*Update` describing no assignment, or an insert of no rows —
    /// caught where the request-shaped data is read (`Assignments::from_row`,
    /// `.values_all`), never at a statement. Here so a handler returning
    /// this crate's `Result` can `?` on that as readily as on a query.
    #[error(transparent)]
    NothingToSet(#[from] qbrs_core::update::NothingToSet),

    #[error(transparent)]
    NothingToInsert(#[from] qbrs_core::insert::NothingToInsert),

    /// A column type is enabled on `qbrs` but not on `qbrs-sqlx`, so the
    /// value renders and has nothing to bind it. The two crates carry the
    /// same feature names for exactly this reason — turn it on in both.
    #[error("`{0}` values need the matching feature on `qbrs-sqlx` too")]
    FeatureNotEnabled(&'static str),
}

/// This crate's `Result`: the same shape as `sqlx::Result`, with
/// `qbrs_sqlx::Error` as the fixed error type.
pub type Result<T> = std::result::Result<T, Error>;

/// Every extension trait that puts a terminal method on a builder, plus the
/// error type a caller's own signatures have to name and the `DecodeRow`
/// bound a generic helper over `RowQuery` has to spell. `Result` is
/// deliberately absent: a glob-imported alias of that name shadows
/// `std::result::Result` in every module that follows, and a service layer
/// has its own error type in most of them — write `qbrs_sqlx::Result<T>`
/// where the alias is wanted. Which trait applies
/// depends on the builder, so importing them one at a time is bookkeeping
/// with no decision in it — and `count` in particular resolves against
/// `Iterator::count` with a confusing message until `CountExt` is in scope.
pub mod prelude {
    pub use crate::Error;
    pub use crate::{
        CountExt, CountQuery, DecodeRow, ExecuteExt, LoadExt, PreparedCountExt, PreparedExt,
        PreparedQuery, PreparedTotal, RowQuery, Stream, StreamExt, WriteStatement,
    };
}

/// Binds a `Value` to a Postgres query parameter. `Value`'s typed `NullX`
/// variants carry the parameter type a NULL bind still has to declare.
///
/// Fallible only for `Value::Placeholder`: an unresolved named placeholder
/// is a misuse no compile-time check here can catch, so it surfaces as
/// `Error::UnresolvedPlaceholder` rather than a wrong bind or a panic.
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
        Value::TextArray(x) => query.bind(x),
        Value::NullTextArray => query.bind(None::<Vec<String>>),
        Value::IntegerArray(x) => query.bind(x),
        Value::NullIntegerArray => query.bind(None::<Vec<i32>>),
        Value::BigIntArray(x) => query.bind(x),
        Value::NullBigIntArray => query.bind(None::<Vec<i64>>),
        #[cfg(feature = "uuid")]
        Value::UuidArray(x) => query.bind(x),
        #[cfg(feature = "uuid")]
        Value::NullUuidArray => query.bind(None::<Vec<uuid::Uuid>>),
        #[cfg(feature = "json")]
        Value::Json(x) => query.bind(x),
        #[cfg(feature = "json")]
        Value::NullJson => query.bind(None::<serde_json::Value>),
        #[cfg(feature = "chrono")]
        Value::Timestamptz(x) => query.bind(x),
        #[cfg(feature = "chrono")]
        Value::NullTimestamptz => query.bind(None::<chrono::DateTime<chrono::Utc>>),
        #[cfg(feature = "chrono")]
        Value::Date(x) => query.bind(x),
        #[cfg(feature = "chrono")]
        Value::NullDate => query.bind(None::<chrono::NaiveDate>),
        #[cfg(feature = "uuid")]
        Value::Uuid(x) => query.bind(x),
        #[cfg(feature = "uuid")]
        Value::NullUuid => query.bind(None::<uuid::Uuid>),
        #[cfg(feature = "decimal")]
        Value::Numeric(x) => query.bind(x),
        #[cfg(feature = "decimal")]
        Value::NullNumeric => query.bind(None::<rust_decimal::Decimal>),
        Value::Placeholder(name) => {
            return Err(qbrs_core::select::UnresolvedPlaceholder(name).into());
        }
        // Reachable only when a column type is on in `qbrs-core` and off
        // here: the variant exists, the arm that binds it doesn't.
        #[allow(unreachable_patterns)]
        other => return Err(Error::FeatureNotEnabled(other.type_name())),
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

/// Generic over `E: sqlx::PgExecutor` so every `.load()`/`.execute()` works
/// against a `&PgPool` or a transaction alike. sqlx implements `Executor` for
/// `&mut PgConnection`, not `Transaction`, so callers pass `&mut *tx`.
async fn fetch_all<'e, T: DecodeRow, E: sqlx::PgExecutor<'e>>(
    executor: E,
    sql: &str,
    params: Vec<Value>,
) -> Result<Vec<T>> {
    let rows = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql)), params)?
        .fetch_all(executor)
        .await?;
    rows.iter()
        .map(|row| T::decode_at(row, &mut 0).map_err(Error::from))
        .collect()
}

async fn fetch_optional<'e, T: DecodeRow, E: sqlx::PgExecutor<'e>>(
    executor: E,
    sql: &str,
    params: Vec<Value>,
) -> Result<Option<T>> {
    let row = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql)), params)?
        .fetch_optional(executor)
        .await?;
    row.as_ref()
        .map(|r| T::decode_at(r, &mut 0).map_err(Error::from))
        .transpose()
}

/// The streaming counterpart: rows are decoded as they arrive rather than
/// collected first. The query owns its SQL and its binds, so the stream
/// borrows only the executor.
fn fetch_stream<'e, T: DecodeRow + Send + 'e, E: sqlx::PgExecutor<'e>>(
    executor: E,
    sql: String,
    params: Vec<Value>,
) -> Result<impl Stream<Item = Result<T>> + Send + Unpin + 'e> {
    let query = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql)), params)?;
    Ok(futures_util::StreamExt::map(query.fetch(executor), |row| {
        T::decode_at(&row?, &mut 0).map_err(Error::from)
    }))
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

/// What a row-producing query renders to, and what its rows decode to: a
/// `SELECT`, a `RETURNING` clause, an erased `DynSelect`, a `UNION` chain.
/// `LoadExt` is the methods over it, and the split is load-bearing
/// — with the validity bound on the impl instead, an invalid selection
/// makes `.load(..)` not exist, and the scope error the builder wanted to
/// report is replaced by a method-resolution failure that never mentions
/// the table.
///
/// `Idx` is threaded through the trait's parameter list for the reason
/// `scope::Superset` explains. Callers never see it; it's inferred.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a query this crate can run",
    label = "a `Select`, a `RETURNING`, a `DynSelect`, a set operation, or a `SELECT` with no `FROM`, in the `Postgres` dialect, whose values are all types `DecodeRow` covers"
)]
pub trait RowQuery<Idx> {
    type Output: DecodeRow;

    #[doc(hidden)]
    fn rendered(&self) -> (String, Vec<Value>);
}

/// `load` for the rows, `load_one` for the first of them, `stream` for
/// them one at a time, and `ExecuteExt::execute` where there are none to
/// decode. One trait for
/// every row-producing builder keeps the terminal vocabulary tied to what a
/// statement yields rather than to which builder happens to be in hand.
///
/// Implemented for every builder, satisfiable by the ones that produce
/// rows: what a builder can't do is then reported by `RowQuery`, which says
/// so, rather than by the method not existing — which rustc answers with a
/// list of unsatisfied bounds or, worse, by suggesting `Iterator`. Not a
/// blanket impl, since `load`/`count`/`execute` are names other traits in a
/// caller's scope have too. A builder added here needs its three empty
/// impls, or its terminal goes back to reporting nothing.
pub trait LoadExt {
    fn load<'e, Idx, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Vec<<Self as RowQuery<Idx>>::Output>>>
    where
        Self: RowQuery<Idx>,
    {
        let (sql, params) = self.rendered();
        async move { fetch_all::<<Self as RowQuery<Idx>>::Output, E>(executor, &sql, params).await }
    }

    fn load_one<'e, Idx, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Option<<Self as RowQuery<Idx>>::Output>>>
    where
        Self: RowQuery<Idx>,
    {
        let (sql, params) = self.rendered();
        async move { fetch_optional::<<Self as RowQuery<Idx>>::Output, E>(executor, &sql, params).await }
    }

    /// The rows one at a time, for a result too large to hold: an export
    /// that writes as it reads rather than collecting first. Yields the
    /// same values `.load(..)` would, decoded as each row arrives.
    ///
    /// Not a cursor: the server still produces the whole result, and the
    /// connection is held until the stream is dropped or exhausted. What
    /// this bounds is the client's memory, which is what a `Vec` of every
    /// row costs.
    ///
    /// Returns a `Result` around the stream rather than as its first item,
    /// because what can fail before a row arrives — an unresolved
    /// placeholder, a value whose feature is on in `qbrs` and off here —
    /// is a misuse of this crate rather than a row that didn't decode.
    /// Everything the database has to say arrives as an item.
    ///
    /// `Send` and `Unpin` are promised, so the stream can be spawned and
    /// polled without pinning it first — a generic caller cannot ask for
    /// either otherwise.
    fn stream<'e, Idx, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> Result<impl Stream<Item = Result<<Self as RowQuery<Idx>>::Output>> + Send + Unpin + 'e>
    where
        Self: RowQuery<Idx>,
        <Self as RowQuery<Idx>>::Output: Send + 'e,
    {
        let (sql, params) = self.rendered();
        fetch_stream::<<Self as RowQuery<Idx>>::Output, E>(executor, sql, params)
    }
}

impl<D, Scope, Sel, Outer> LoadExt for Select<D, Scope, Sel, Outer> {}
impl<Sel> LoadExt for qbrs_core::select::SelectSeed<Sel> {}
impl<S, Sel> LoadExt for Returning<S, Sel> {}
impl<D, Output> LoadExt for DynSelect<D, Output> {}
impl<D, Output> LoadExt for SetOp<D, Output> {}
impl<D, R: qbrs_core::insert::InsertRow> LoadExt for Insert<D, R> {}
impl<D, T: qbrs_core::scope::Table> LoadExt for qbrs_core::insert::InsertSelect<D, T> {}
impl<D, T: qbrs_core::scope::Table> LoadExt for Update<D, T> {}
impl<D, T: qbrs_core::scope::Table> LoadExt for Delete<D, T> {}

impl<Scope, Sel, Idx> RowQuery<Idx> for Select<Postgres, Scope, Sel>
where
    Sel: Selection<Scope, Idx>,
    Sel::Output: DecodeRow,
{
    type Output = Sel::Output;

    fn rendered(&self) -> (String, Vec<Value>) {
        self.to_sql::<Idx>(Postgres)
    }
}

/// A `SELECT` with no `FROM`: the seed is the whole statement, so it is
/// what carries the terminal.
impl<Sel, Idx> RowQuery<Idx> for qbrs_core::select::SelectSeed<Sel>
where
    Sel: Selection<qbrs_core::scope::Nil, Idx>,
    Sel::Output: DecodeRow,
{
    type Output = Sel::Output;

    fn rendered(&self) -> (String, Vec<Value>) {
        self.to_sql::<Postgres, Idx>(Postgres)
    }
}

/// `SELECT count(*)` over a query's `FROM`/`JOIN`/`WHERE`/`GROUP BY`, with
/// its `ORDER BY`/`LIMIT`/`OFFSET` dropped — a total counts the rows that
/// match, not the page being shown. Returns a number rather than an
/// `Option`, since a count query always produces exactly one row.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a query this crate can count",
    label = "a `Select`, a `DynSelect` or a set operation in the `Postgres` dialect is; a writing statement reports rows affected through `.execute(..)` instead, and a `SELECT` with no `FROM` returns one row — a query missing its `.from(..)` is what this usually means"
)]
pub trait CountQuery<Idx> {
    #[doc(hidden)]
    fn count_rendered(&self) -> (String, Vec<Value>);
}

/// The bound is on the method, and the impls are per-builder, for the two
/// reasons `LoadExt` explains.
pub trait CountExt {
    fn count<'e, Idx, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<i64>>
    where
        Self: CountQuery<Idx>,
    {
        count_rows(executor, self.count_rendered())
    }
}

impl<D, Scope, Sel, Outer> CountExt for Select<D, Scope, Sel, Outer> {}
impl<Sel> CountExt for qbrs_core::select::SelectSeed<Sel> {}
impl<S, Sel> CountExt for Returning<S, Sel> {}
impl<D, Output> CountExt for DynSelect<D, Output> {}
impl<D, Output> CountExt for SetOp<D, Output> {}
impl<D, R: qbrs_core::insert::InsertRow> CountExt for Insert<D, R> {}
impl<D, T: qbrs_core::scope::Table> CountExt for qbrs_core::insert::InsertSelect<D, T> {}
impl<D, T: qbrs_core::scope::Table> CountExt for Update<D, T> {}
impl<D, T: qbrs_core::scope::Table> CountExt for Delete<D, T> {}

impl<Scope, Sel: Selection<Scope, Idx>, Idx> CountQuery<Idx> for Select<Postgres, Scope, Sel> {
    fn count_rendered(&self) -> (String, Vec<Value>) {
        self.count_sql::<Idx>(Postgres)
    }
}

/// Erasure is for a query whose joins depend on a condition, and such a
/// query is paged like any other, so it counts like any other. The same
/// goes for a set-operation chain.
impl<Output> CountQuery<()> for DynSelect<Postgres, Output> {
    fn count_rendered(&self) -> (String, Vec<Value>) {
        self.count_sql(Postgres)
    }
}

impl<Output> CountQuery<()> for SetOp<Postgres, Output> {
    fn count_rendered(&self) -> (String, Vec<Value>) {
        self.count_sql(Postgres)
    }
}

async fn count_rows<'e, E: sqlx::PgExecutor<'e>>(
    executor: E,
    (sql, params): (String, Vec<Value>),
) -> Result<i64> {
    let row = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql.as_str())), params)?
        .fetch_one(executor)
        .await?;
    Ok(row.try_get::<i64, _>(0)?)
}

/// Every writing statement, rendered: what `execute` returns is rows
/// affected, whichever of the three it was.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a statement this crate can execute",
    label = "an `INSERT`, `UPDATE` or `DELETE` in the `Postgres` dialect is; a `SELECT` or a `RETURNING` yields rows, so it goes through `.load(..)` — and a prepared query through `.load(.., params)`"
)]
pub trait WriteStatement {
    #[doc(hidden)]
    fn write_rendered(&self) -> (String, Vec<Value>);
}

#[diagnostic::do_not_recommend]
impl<S: Statement<Dialect = Postgres>> WriteStatement for S {
    fn write_rendered(&self) -> (String, Vec<Value>) {
        self.to_sql(Postgres)
    }
}

/// The bound is on the method, and the impls are per-builder, for the two
/// reasons `LoadExt` explains.
pub trait ExecuteExt {
    fn execute<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<u64>>
    where
        Self: WriteStatement,
    {
        let (sql, params) = self.write_rendered();
        async move { execute_only(executor, &sql, params).await }
    }
}

impl<D, Scope, Sel, Outer> ExecuteExt for Select<D, Scope, Sel, Outer> {}
impl<Sel> ExecuteExt for qbrs_core::select::SelectSeed<Sel> {}
impl<S, Sel> ExecuteExt for Returning<S, Sel> {}
impl<D, Output> ExecuteExt for DynSelect<D, Output> {}
impl<D, Output> ExecuteExt for SetOp<D, Output> {}
impl<D, R: qbrs_core::insert::InsertRow> ExecuteExt for Insert<D, R> {}
impl<D, T: qbrs_core::scope::Table> ExecuteExt for qbrs_core::insert::InsertSelect<D, T> {}
impl<D, T: qbrs_core::scope::Table> ExecuteExt for Update<D, T> {}
impl<D, T: qbrs_core::scope::Table> ExecuteExt for Delete<D, T> {}

/// One impl for every `RETURNING`: what a statement returns is decided by
/// its selection, not by which statement it was.
impl<S: Statement<Dialect = Postgres>, Sel, Idx> RowQuery<Idx> for Returning<S, Sel>
where
    Sel: Selection<WrittenTable<S::Table>, Idx>,
    Sel::Output: DecodeRow,
{
    type Output = Sel::Output;
    fn rendered(&self) -> (String, Vec<Value>) {
        self.to_sql(Postgres)
    }
}

/// Decodes a query's `Output` positionally out of a `PgRow`. Keyed on the
/// plain-Rust type a selection produces rather than on the selection
/// itself: erasure leaves only `Output`, with no `Selection` impl left to
/// hang decoding off, so this is implemented directly against the closed set
/// of native types.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a value this crate can decode",
    label = "every selected column has to decode to one of the featureless natives, or to a type whose feature is on here as well as on `qbrs`",
    note = "`chrono`/`uuid`/`decimal` have to be enabled on `qbrs-sqlx` too — they are separate `cfg`s over one `Value`"
)]
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
decode_row_leaf!(Vec<String>);
decode_row_leaf!(Vec<i32>);
decode_row_leaf!(Vec<i64>);
#[cfg(feature = "uuid")]
decode_row_leaf!(Vec<uuid::Uuid>);
#[cfg(feature = "json")]
decode_row_leaf!(serde_json::Value);
#[cfg(feature = "chrono")]
decode_row_leaf!(chrono::DateTime<chrono::Utc>);
#[cfg(feature = "chrono")]
decode_row_leaf!(chrono::NaiveDate);
#[cfg(feature = "uuid")]
decode_row_leaf!(uuid::Uuid);
#[cfg(feature = "decimal")]
decode_row_leaf!(rust_decimal::Decimal);

impl DecodeRow for RowNil {
    fn decode_at(_row: &PgRow, _idx: &mut usize) -> sqlx::Result<Self> {
        Ok(RowNil)
    }
}

impl<K, V: DecodeRow, Tail: DecodeRow> DecodeRow for RowCons<K, V, Tail> {
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
        let value = V::decode_at(row, idx)?;
        Ok(RowCons::new(value, Tail::decode_at(row, idx)?))
    }
}

impl<L: DecodeRow> DecodeRow for Row<L> {
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
        Ok(Row::new(L::decode_at(row, idx)?))
    }
}

/// An erased query and a set-op chain were both rendered before their
/// selection type was gone, leaving nothing for `Idx` to index — hence
/// `RowQuery<()>`, the same trait with an empty proof.
impl<Output: DecodeRow> RowQuery<()> for DynSelect<Postgres, Output> {
    type Output = Output;
    fn rendered(&self) -> (String, Vec<Value>) {
        self.to_sql(Postgres)
    }
}

impl<Output: DecodeRow> RowQuery<()> for SetOp<Postgres, Output> {
    type Output = Output;
    fn rendered(&self) -> (String, Vec<Value>) {
        self.to_sql(Postgres)
    }
}

/// Runs a `prepare!{}`-built query, resolving its named placeholders from
/// `params` first. Separate from `LoadExt` only because the values arrive at
/// the call rather than being baked into the query: one `Prepared` is meant
/// to serve many calls, and `.resolve()` clones the template rather than
/// re-rendering it.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a prepared query this crate can run",
    label = "a `.prepare()`-built query is — `Prepared<D, Params, Output>`, params before output — and its `Params` have to be the ones it declared"
)]
pub trait PreparedQuery<Params> {
    type Output: DecodeRow;
    #[doc(hidden)]
    fn resolved(&self, params: Params) -> Result<(String, Vec<Value>)>;
}

/// The bound is on the method, and the impls are per-builder, for the two
/// reasons `LoadExt` explains.
pub trait PreparedExt {
    fn load<'e, Params, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
        params: Params,
    ) -> impl std::future::Future<Output = Result<Vec<<Self as PreparedQuery<Params>>::Output>>>
    where
        Self: PreparedQuery<Params>,
    {
        let resolved = self.resolved(params);
        async move {
            let (sql, values) = resolved?;
            fetch_all::<<Self as PreparedQuery<Params>>::Output, E>(executor, &sql, values).await
        }
    }

    fn load_one<'e, Params, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
        params: Params,
    ) -> impl std::future::Future<Output = Result<Option<<Self as PreparedQuery<Params>>::Output>>>
    where
        Self: PreparedQuery<Params>,
    {
        let resolved = self.resolved(params);
        async move {
            let (sql, values) = resolved?;
            fetch_optional::<<Self as PreparedQuery<Params>>::Output, E>(executor, &sql, values)
                .await
        }
    }

    /// The rows one at a time, as `LoadExt::stream` gives them — with the
    /// `Params` that arrive at the call, which is what a reusable export
    /// query wants.
    fn stream<'e, Params, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
        params: Params,
    ) -> Result<
        impl Stream<Item = Result<<Self as PreparedQuery<Params>>::Output>> + Send + Unpin + 'e,
    >
    where
        Self: PreparedQuery<Params>,
        <Self as PreparedQuery<Params>>::Output: Send + 'e,
    {
        let (sql, values) = self.resolved(params)?;
        fetch_stream::<<Self as PreparedQuery<Params>>::Output, E>(executor, sql, values)
    }
}

impl<D, Params, Output> PreparedExt for Prepared<D, Params, Output> {}

// A prepared query's `load`/`count` are told apart from the plain ones by
// arity, but `execute` is not — without this, it is the one terminal on the
// one builder that reports nothing.
impl<D, Params, Output> ExecuteExt for Prepared<D, Params, Output> {}

#[diagnostic::do_not_recommend]
impl<Params: PreparedParams, Output: DecodeRow> PreparedQuery<Params>
    for Prepared<Postgres, Params, Output>
{
    type Output = Output;

    fn resolved(&self, params: Params) -> Result<(String, Vec<Value>)> {
        Ok(self.resolve(params)?)
    }
}

/// A prepared total. Separate from `PreparedExt` for the reason `CountExt`
/// is separate from `LoadExt`: a count produces a number, not rows.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a prepared total this crate can run",
    label = "`.prepare_count()` builds one; `.prepare()` builds a query whose rows go through `.load(..)`"
)]
pub trait PreparedTotal<Params> {
    #[doc(hidden)]
    fn resolved_count(&self, params: Params) -> Result<(String, Vec<Value>)>;
}

impl<Params: PreparedParams> PreparedTotal<Params> for Prepared<Postgres, Params, Total> {
    fn resolved_count(&self, params: Params) -> Result<(String, Vec<Value>)> {
        Ok(self.resolve(params)?)
    }
}

/// The bound is on the method, and the impls are per-builder, for the two
/// reasons `LoadExt` explains.
pub trait PreparedCountExt {
    fn count<'e, Params, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
        params: Params,
    ) -> impl std::future::Future<Output = Result<i64>>
    where
        Self: PreparedTotal<Params>,
    {
        let resolved = self.resolved_count(params);
        async move { count_rows(executor, resolved?).await }
    }
}

impl<D, Params, Output> PreparedCountExt for Prepared<D, Params, Output> {}

#[cfg(test)]
mod tests {
    use super::*;

    // No real Postgres needed: binding inspects the `Value` enum before
    // anything reaches the network, so an unresolved placeholder is
    // reachable, and its error checkable, without a live DB.
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
        // A real `std::error::Error`, so it composes with
        // `anyhow`/`Box<dyn Error>`.
        let _: &dyn std::error::Error = &err;
        assert_eq!(err.to_string(), "no value provided for placeholder `email`");
    }

    #[test]
    fn sqlx_errors_convert_via_from() {
        let err: Error = sqlx::Error::RowNotFound.into();
        assert!(matches!(err, Error::Sqlx(sqlx::Error::RowNotFound)));
    }
}
