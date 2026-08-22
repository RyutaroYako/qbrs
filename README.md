<p align="center">
  <img src="assets/logo-dark.png" alt="qbrs" width="320">
</p>

<p align="center"><b>A Drizzle-flavored, type-safe SQL query builder for Rust.</b></p>

<p align="center">
<a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License: MIT OR Apache-2.0"></a>
<img src="https://img.shields.io/badge/status-pre--release-orange.svg" alt="Status: pre-release">
<img src="https://img.shields.io/badge/rust-2024%20edition-orange.svg" alt="Rust 2024 edition">
</p>

<p align="center">
<a href="#quick-example">Quick example</a> ·
<a href="#why-qbrs">Why qbrs?</a> ·
<a href="#install">Install</a> ·
<a href="#usage">Usage</a> ·
<a href="#status">Status</a> ·
<a href="#running-the-examples--tests">Examples &amp; tests</a>
</p>

---

qbrs checks column and join references at compile time without giving up
dynamic query composition — most builders make you pick one. See [Why
qbrs?](#why-qbrs) for how.

> **Not an ORM.** qbrs builds and renders SQL with compile-time-checked
> column/join references; it doesn't do change-tracking, identity maps, or
> hide SQL behind an object graph. Want that? See
> [SeaORM](https://www.sea-ql.org/SeaORM/). Want raw SQL with compile-time
> type-checking instead of a builder? See
> [sqlx](https://github.com/launchbadge/sqlx).

## Quick example

```rust
#[derive(Table)]
#[table(name = "users")]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
    display_name: Option<String>,
    #[column(default)]
    active: bool,
}

#[derive(Table)]
#[table(name = "orders")]
struct Orders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

let rows = select((users::email, orders::total))
    .from::<Postgres, _>(users::Table)
    .left_join(orders::Table, orders::user_id.eq(users::id))
    .order_by(users::id.asc())
    .load(&pool)
    .await?;

for row in &rows {
    println!("{} {:?}", row.email(), row.total()); // &String, &Option<i64>
}
```

```sql
-- rendered by qbrs:
SELECT "users"."email", "orders"."total"
FROM "users" LEFT JOIN "orders" ON ("orders"."user_id" = "users"."id")
ORDER BY "users"."id" ASC
```

`orders::total` is declared as a plain `i64` column — not `Option<i64>` — but
because it's on the far side of a `LEFT JOIN`, `row.total()` comes back as
`&Option<i64>` automatically. A row is read by the same column value that
selected it, not by position, so adding a column to the selection doesn't
move anything (`into_tuples()` gives the positional view back where
destructuring is what's wanted). Forget the join and reference
`orders::total` anyway, and it's a compile error, not a runtime surprise:

```
error[E0277]: `orders::Table` is not available in this query's scope
  --> src/main.rs:23:22
   |
23 |       let rows = select((users::email, orders::total))
   |  ________________^
24 | |         .from::<Postgres, _>(users::Table)
   | |__________________________________________^ add `.join(<table>, ..)` (or `.from(..)`)
   |                                              for `orders::Table` before referencing
   |                                              its columns here
   |
   = note: columns can only be referenced once their table has been joined into the
           current FROM/JOIN scope
```

(rustc prints the underlying `Find`/`RowField`/`Selection` obligation chain after
that, as it does for any unsatisfied trait bound.)

## Why qbrs?

|                                                                                         |    qbrs     |                                            diesel                                            |       sea-query        |            sqlx            |
| --------------------------------------------------------------------------------------- | :---------: | :------------------------------------------------------------------------------------------: | :--------------------: | :------------------------: |
| Compile-time column/join validity checking                                              |     ✅      |                                              ✅                                              |    ❌ (runtime AST)    |    n/a — not a builder     |
| `NULL`-ability auto-derived from join kind                                              |     ✅      |                                  ❌ (manual `.nullable()`)                                   |           ❌           |            n/a             |
| Add predicates conditionally/in a loop, no escape hatch                                 |     ✅      |                                   ⚠️ needs `.into_boxed()`                                   | ✅ (dynamic by design) | ⚠️ drops to `QueryBuilder` |
| Table/column refs are plain values, not turbofish/closures                              |     ✅      |                                           partial                                            |           ✅           |            n/a             |
| Result rows keyed by column, not by position                                            |     ✅      |                                              ❌                                              |           ❌           |     ✅ (`query_as!`)      |
| Compile time at ~40+ joins (measured, see [`tests/compile-bench`](tests/compile-bench)) | linear, ~ms | [documented exponential blowup](https://github.com/diesel-rs/diesel/issues/3223) at ~7 joins |          n/a           |            n/a             |

The trick: whether a column reference makes sense given what's joined is a
compile-time question, checked once via a flat type-level list (see
[`crates/core/src/scope.rs`](crates/core/src/scope.rs)). How many predicates
you've added is a runtime question — a plain `Vec`, so `.filter()` can be
called conditionally or in a loop without changing the query's type. Most
builders conflate the two and need an escape hatch (`.$dynamic()`,
`.into_boxed()`) the moment a query gets built conditionally. qbrs needs one
too, but it's deliberately narrow — see [Known limitations](#known-limitations).

## Install

Not yet published to crates.io. Depend on it from git in the meantime:

```toml
[dependencies]
qbrs = { git = "https://github.com/RyutaroYako/qbrs" }
qbrs-sqlx = { git = "https://github.com/RyutaroYako/qbrs" }  # Postgres execution via sqlx
```

## Usage

A schema is a `#[derive(Table)]` struct, shown in the
[Quick example](#quick-example) above. Two imports cover a query —
`use qbrs::prelude::*;` for the builder and its extension traits, and
`use qbrs_sqlx::prelude::*;` for `.load()`/`.execute()`. See
[`examples/`](examples) for complete, runnable code for everything below.

- **Rows keyed by column** —
  [`16_row_access`](examples/examples/16_row_access.rs). A tuple selection
  decodes to a `Row`, read with `row.get(users::email)` or the accessor
  `#[derive(Table)]` generates for each column (`row.email()`). Selecting a
  single un-tupled column still decodes to a bare value, and `into_tuple()` /
  `into_tuples()` recover the positional tuple. A computed
  expression is keyed by the function that produced it (`row.count()`,
  `row.row_number()`); `label!(name, ..)` renames one when the same
  function is selected twice, and emits the name as the column's `AS`.
- **Whole-table selection** (`users::All`) —
  [`17_from_row`](examples/examples/17_from_row.rs). The derive already knows
  the table's columns, so `select(users::All)` doesn't restate them and a new
  column can't leave a query behind. It composes:
  `select((users::All, orders::total))` counts as one element of the tuple
  however many columns the table has, and each of them takes its
  NULL-ability from how the table was joined.
- **Naming a query or a row** — a `Select`'s `Scope` lists the most
  recently joined table first, and a `Row`'s fields are in selection order;
  both spell out long enough to want a `type` alias and an
  `#[allow(clippy::type_complexity)]`. Inference covers every use that stays
  inside a function.
- **Rows into your own structs** —
  [`17_from_row`](examples/examples/17_from_row.rs). `#[derive(FromRow)]`
  fills a plain struct by matching field *names* — or the name given by
  `#[from_row(rename = "..")]` where the two differ. The struct declares no
  column path, no table, and no join, so it can live in a domain module with
  `#[derive(Serialize)]` and be filled from any query that selects columns of
  those names and types. Selection order doesn't matter and extra columns are
  ignored. Nothing is copied — fields move out of the row, and no `Clone`
  bound exists to do otherwise. Where a name doesn't line up, `row.take(col)`
  moves one field out and hands back the rest.
- **Select / Insert / Update / Delete** —
  [`01_select_basic`](examples/examples/01_select_basic.rs),
  [`03_insert`](examples/examples/03_insert.rs),
  [`04_update`](examples/examples/04_update.rs),
  [`05_delete`](examples/examples/05_delete.rs). A NOT NULL column with a
  schema default gets a three-state `Defaultable<Option<T>>` on `*Insert`
  (omit / explicit `NULL` / explicit value); `*Update` mirrors this with
  `Option<Option<T>>` (untouched / `NULL` / value). A request struct's
  `Option<T>` converts into either. Rows arrive one at a time with
  `.values(row)` or all at once with `.values_all(rows)`; a statement with
  nothing in it — an `*Update` whose every field is untouched, an insert of
  zero rows — has no SQL form, so those hand back
  `Result<_, NothingToSet>` / `Result<_, NothingToInsert>` rather than
  panicking at render time.
- **`SELECT DISTINCT`** — `.distinct()`, the answer to a one-to-many join
  that repeats its left side. A count of such a query counts its distinct
  rows.
- **Upsert** (`ON CONFLICT`, Postgres/SQLite only) —
  [`11_upsert`](examples/examples/11_upsert.rs). No typed `EXCLUDED.column`
  yet.
- **`UNION`/`INTERSECT`/`EXCEPT`** —
  [`12_union`](examples/examples/12_union.rs). Combines `SELECT`s with
  unrelated `Scope`s as long as their output shapes match.
- **Window functions** (`row_number()`/`rank()`/`dense_rank()`) —
  [`13_window`](examples/examples/13_window.rs). Aggregate-as-window-function
  isn't supported yet.
- **CTEs** (`with!{}` + `cte::with(..)`) —
  [`14_cte`](examples/examples/14_cte.rs). The binding goes where a table
  goes — `.from(binding)` / `.inner_join(binding, on)` — attaching the `WITH`
  clause and putting the pseudo-table in scope in one act, so a CTE can't be
  selected from unbound.
  Non-recursive, single-level only.
- **Correlated subqueries** —
  [`18_correlated_exists`](examples/examples/18_correlated_exists.rs).
  `outer.correlated(table, sel)` builds a subquery whose scope is the outer
  query's plus its own table, so referencing an outer column is legal; the
  resulting `EXISTS` is tagged with those tables, so filtering it onto a
  query that doesn't have them is a compile error.
- **One query, two shapes** — `.count(&pool)` answers "how many rows would
  this return", ignoring `ORDER BY`/`LIMIT`/`OFFSET` and counting *groups*
  for a grouped query; it borrows, so a paginated endpoint needs no clone.
  `Select` is also `Clone`, and `.reselect(sel)` swaps the selection while
  keeping every clause — for when the second shape isn't a count.
- **Dynamic composition, no escape hatch** —
  [`07_dynamic_filters`](examples/examples/07_dynamic_filters.rs).
  `.filter()` doesn't change `Select`'s type, so it can be called
  conditionally or in a loop. Conditions from *different* tables don't share
  an `Expr` type, so a collection of them goes through `predicate(..)` and
  `.filter_all(..)`, which discharges the scope requirement up front.
- **Predicates** — `.eq()`/`.ne()`/`.lt()`/`.gt()`/`.like()`, plus
  `.is_null()`/`.is_not_null()` and `.is_in([..])`. A nullable column and a
  non-nullable one compare freely, so an optional foreign key joins like any
  other. Comparing to NULL with `=` is never true in SQL, so `.eq(None)`
  isn't expressible: the question is `.is_null()`.
- **Aggregates** — `count()` (`count(*)`), plus `count_of(col)`, `sum(col)`,
  `avg(col)`, `min(col)`, `max(col)`. Each takes a real column, so a missing
  join is a compile error rather than a `must appear in the GROUP BY clause`
  at run time, and each is named after its column, so it reads back as
  `row.get(sum(orders::total))` and satisfies a CTE or DTO field called
  `total`. All are nullable except the two counts: an aggregate over zero
  rows is NULL, but a count of them is `0`.
- **Raw SQL escape hatch** (`sql!{}`) —
  [`06_raw_sql`](examples/examples/06_raw_sql.rs). Values still bind as real
  parameters, never spliced as text.
- **Prepared statements** (`prepare!{}`) —
  [`10_prepared`](examples/examples/10_prepared.rs). Typed, so a
  missing/misspelled bind is a compile error, unlike Drizzle's
  `sql.placeholder()`.
- **Transactions** —
  [`15_transaction`](examples/examples/15_transaction.rs). Every
  `.load()`/`.execute()` method is generic over `sqlx::PgExecutor`, so a
  `sqlx::PgTransaction` from `pool.begin()` works everywhere a `&PgPool`
  does — no separate transactional API to learn:
  ```rust
  let mut tx = pool.begin().await?;
  insert::<Postgres, _>(users::Table)
      .values(UsersInsert::new("ada@example.com"))
      .execute(&mut *tx)
      .await?;
  tx.commit().await?;
  ```
- **Errors** — every `.load()`/`.execute()` returns `qbrs_sqlx::Result<T>`
  (`= Result<T, qbrs_sqlx::Error>`), not a raw `sqlx::Result`. `Error` has
  two variants: `Sqlx` (a real driver/database error) and
  `UnresolvedPlaceholder` (a `prepare!{}` placeholder with no matching
  value — a qbrs-level misuse, not a database error). Keeping these
  distinct means a caller can `match` on the cause instead of
  string-matching an error message.

## Known limitations

- **Conditionally joining a table (not just filtering) has no fully-static
  solution** — a single type can't mean "joined" in one branch and "not
  joined" in another. `.erase()` into `DynSelect` is the narrow way out: only
  the join skeleton is erased (not the whole query, unlike `.$dynamic()`; not
  a boxed trait object per predicate, unlike diesel's `BoxableExpression`),
  and no further `.filter()`/`.join()` is offered on it — see
  [`examples/09_dynamic_join.rs`](examples/examples/09_dynamic_join.rs).
- Selecting a computed/raw expression's `NULL`-ability isn't derived the way
  a bare column's is — `sql!(Nullable<Text>, "...")` if the expression itself
  can be `NULL`.
- A `sql!{}` fragment has no name of its own, so it is readable only
  positionally until `label!` gives it one — passing one to `row.get(..)` is
  a compile error, not a lookup of some other unnamed field. The same applies wherever two
  selections are compared by name — a CTE body and a `UNION` branch.
- Selecting the same name twice is ambiguous at the point it's read rather
  than resolving to the first; alias one of them.
- Naming a row type in a signature takes a type alias, and one long enough
  to trip `clippy::type_complexity`; inference covers every use that stays
  inside a function.
- `ORDER BY` takes an expression, not an output alias: sort by
  `sum(orders::total).desc()`, not by the `label!` it was aliased to.
- One `label!` per scope — it declares a `label` module, and a scope holds
  one. List every name that scope needs in the one invocation.
- A helper generic over rows needs one index type parameter per column it
  reads (`fn f<I1, I2, R>(..) where R: HasEmail<I1> + HasTotal<I2>`). Sharing
  one across two columns compiles and then matches no row.
- A `#[derive(FromRow)]` field's type is the column's decoded type, so a DTO
  filled from a `LEFT JOIN` declares `Option<T>` where one filled from an
  `INNER JOIN` declares `T`. It names no column and no table, but it does
  pin the join's nullability.
- `count()` is `count(*)` — rows, not non-NULL values; `count_of(col)` is
  the latter. Both return a non-nullable count; every other aggregate is
  nullable, since an aggregate over zero rows is NULL.
- `sum`/`avg` render a `CAST` back from the wider type a database picks, so
  a sum that overflows `BIGINT` fails where the raw `sum` would have
  succeeded, and `avg` is `DOUBLE PRECISION` rather than exact.
- Aggregates take a bare column: `sum(price * qty)`, `count(DISTINCT x)` and
  `sum(CASE WHEN ..)` need `sql!{}`. Nothing relates `GROUP BY` to the
  selection list, and an aggregate in `WHERE` is accepted by the builder and
  rejected by the database.
- Two selections are compared by column name, so a `UNION` of branches whose
  columns are named differently, or a CTE body with a computed column, needs
  a `label!` alias on one side. A `UNION` also needs both branches to be
  tuple selections, and to agree on nullability.
- No table aliasing. A self-join is rejected, but by an inference ambiguity
  rather than by one of this crate's own diagnostics — and declaring the same
  SQL table twice as two Rust types, the workaround that suggests itself,
  compiles and then renders `FROM "t" JOIN "t"`, which the database refuses.
  Two `#[derive(Table)]` structs must not share a `#[table(name = "..")]`.
- A correlated `EXISTS` is tagged with the outer query's tables, so it can
  only be filtered onto that query — but `prepare!{}` doesn't tie its
  `Params` struct to the query it was built from, and a mismatch surfaces at
  `.load()` as `UnresolvedPlaceholder` rather than at compile time.
- The derives expand to `::qbrs::` paths, so depend on the `qbrs` facade
  rather than on `qbrs-core` + `qbrs-macros` directly.
- `sql!{}` treats `?` as a bind slot; write `??` for a literal one. Its text
  must be a constant — a literal, a `const`, `concat!`, `include_str!` — so
  runtime-assembled text can never become SQL shape, and the placeholder
  count is checked against the value count at compile time.
- Postgres and SQLite are executed in CI — SQLite against an in-memory
  database in `tests/dialect-exec`, which runs every rendered statement shape
  rather than asserting its text. MySQL is rendered and asserted as strings
  only, so its dialect differences are caught only where someone thought to
  look. One known difference: `DEFAULT` in an `INSERT ... VALUES` is Postgres
  and MySQL only, and SQLite rejects it, which makes `Defaultable::Default`
  unusable there.
- A computed expression over a column has to say what it decodes to —
  `expr.decodes_as::<Nullable<BigInt>>()` — because its NULL-ability doesn't
  follow from any one column's join. Like any unnamed selection it is then
  read positionally, or by name once `.alias(label::x)` gives it one.

## Status

|                                                                             | Postgres |  MySQL  | SQLite  |
| --------------------------------------------------------------------------- | :------: | :-----: | :-----: |
| Query building & SQL rendering                                              |    ✅    |   ✅    |   ✅    |
| Rendered SQL executed in CI                                                 |    ✅    | not yet |   ✅    |
| Dialect capability gating (`RETURNING`, `ON CONFLICT`, `RIGHT`/`FULL JOIN`) |    ✅    |   ✅    |   ✅    |
| Execution (via `qbrs-sqlx`)                                                 |    ✅    | not yet | not yet |
| Transactions (via `qbrs-sqlx`)                                              |    ✅    | not yet | not yet |

`SELECT`/`INSERT`/`UPDATE`/`DELETE`, all JOIN kinds, `GROUP BY`/`HAVING`,
correlated subqueries (`EXISTS`/`NOT EXISTS`), the `sql!{}` escape hatch,
reusable named-placeholder prepared statements (`prepare!{}`), upsert
(`ON CONFLICT`), `UNION`/`INTERSECT`/`EXCEPT`, ranking window functions
(`row_number()`/`rank()`/`dense_rank()`), aggregates, non-recursive CTEs
(`with!{}`), column-keyed result rows, and `#[derive(FromRow)]` struct mapping
are implemented and tested against a real Postgres instance. Not yet done:
`WITH RECURSIVE`, aggregate-as-window-functions (`sum(col) OVER (..)`), table
aliasing/self-joins, and relations/eager-loading (intentionally scoped out until the core
query-building layer has been stable for a while).

## Workspace layout

- [`crates/core`](crates/core) (`qbrs-core`) — the type-level machinery: scope
  tracking (`Cons`/`Nil`/`Find`), expressions, `Select`/`Insert`/`Update`/`Delete`
  builders, SQL rendering. No I/O, no async runtime.
- [`crates/macros`](crates/macros) (`qbrs-macros`) — `#[derive(Table)]`,
  `#[derive(FromRow)]`, `label!`, and `with!`.
- [`crates/qbrs`](crates/qbrs) — the facade crate; depend on this one.
- [`crates/qbrs-sqlx`](crates/qbrs-sqlx) — execution via `sqlx` (Postgres).
- [`examples`](examples) (`qbrs-examples`) — runnable examples; see
  [its README](examples/README.md) for how to run them.
- [`tests/compile-bench`](tests/compile-bench) — synthetic-schema compile-time
  regression checks (this is what backs the "linear at 40+ joins" claim above).

## Running the examples & tests

Runnable, end-to-end examples live in [`examples`](examples) —
see [its README](examples/README.md) for the full list.

```sh
cargo test --workspace   # everything, including the real-Postgres tests
```

No setup is required: the real-DB tests start their own throwaway
PostgreSQL 17.5, embedded via [`pglite-rs`](https://crates.io/crates/pglite-rs),
so there is no Docker, no service to launch, and nothing to download at test
time. To run the same tests against an external Postgres instead, set
`DATABASE_URL`:

```sh
DATABASE_URL=postgres://postgres:postgres@localhost:5432/qbrs_test \
  cargo test -p qbrs-sqlx
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Contributions are licensed the same way unless stated otherwise.
