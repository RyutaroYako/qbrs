<p align="center">
  <img src="https://raw.githubusercontent.com/RyutaroYako/qbrs/main/assets/logo-dark.png" alt="qbrs" width="320">
</p>

<p align="center"><b>A Drizzle-flavored, type-safe SQL query builder for Rust.</b></p>

<p align="center">
<a href="https://crates.io/crates/qbrs"><img src="https://img.shields.io/crates/v/qbrs.svg" alt="crates.io"></a>
<a href="https://docs.rs/qbrs"><img src="https://docs.rs/qbrs/badge.svg" alt="docs.rs"></a>
<a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License: MIT OR Apache-2.0"></a>
<img src="https://img.shields.io/badge/rust-1.88%2B-orange.svg" alt="Rust 1.88+">
</p>

<p align="center">
<a href="#quick-example">Quick example</a> ·
<a href="#why-qbrs">Why qbrs?</a> ·
<a href="#install">Install</a> ·
<a href="#whats-in-it">What's in it</a> ·
<a href="#known-limitations">Known limitations</a> ·
<a href="#status">Status</a>
</p>

---

qbrs checks column and join references at compile time without giving up
dynamic query composition — most builders make you pick one. See [Why
qbrs?](#why-qbrs) for how.

> [!WARNING]
> **Not production ready.** 0.1.0 is the first public release. The API will
> break between 0.x minors, only Postgres has an execution layer, and nothing
> here has been run against a real workload yet. Worth trying and filing
> issues against; not worth putting under something that matters.

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
    .from(users::Table)
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

`orders::total` is declared as a plain `i64` column, but because it's on the
far side of a `LEFT JOIN`, `row.total()` comes back as `&Option<i64>`
automatically. A row is read by the column value that selected it, not by
position, so adding a column to the selection moves nothing. Forget the join
and reference `orders::total` anyway, and it's a compile error:

```
error[E0277]: `orders::Table` is not available in this query's scope
  --> src/main.rs:23:22
   |
23 |       let rows = select((users::email, orders::total))
   |  ________________^
24 | |         .from(users::Table)
   | |____________________________^ add `.join(<table>, ..)` (or `.from(..)`)
   |                                for `orders::Table` before referencing
   |                                its columns here
```

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

```toml
[dependencies]
qbrs = "0.1"
qbrs-sqlx = "0.1"                                                     # Postgres execution via sqlx
sqlx = { version = "0.9", features = ["runtime-tokio", "postgres"] }  # for `PgPool`
```

`qbrs-sqlx`'s methods take any `sqlx::PgExecutor`, so the `sqlx` version has
to be the one it is built against (0.9). Column types beyond the six built in
are features — `chrono`, `uuid`, `decimal` — and each has to be enabled on
**both** `qbrs` and `qbrs-sqlx`, which are separate `cfg`s over one `Value`:
enabling only one surfaces as a `FeatureNotEnabled` at bind time, not as a
compile error.

## What's in it

`SELECT`/`INSERT`/`UPDATE`/`DELETE`, every JOIN kind, `GROUP BY`/`HAVING`,
aggregates, `DISTINCT`, upsert (`ON CONFLICT`), `UNION`/`INTERSECT`/`EXCEPT`,
ranking window functions, non-recursive CTEs, correlated `EXISTS`,
`IN (SELECT ..)`/`NOT IN (SELECT ..)`, transactions, the `sql!{}` escape
hatch, and typed prepared statements (`prepare!{}`).

A schema is a `#[derive(Table)]` struct; `use qbrs::prelude::*;` and
`use qbrs_sqlx::prelude::*;` cover a query. Everything above has a runnable,
end-to-end example against a real Postgres — see
**[`examples/README.md`](examples/README.md)** for the index, and
[docs.rs/qbrs](https://docs.rs/qbrs) for the API.

Three things that aren't obvious from a signature:

- **Rows are keyed by column, not by position.** A tuple selection decodes to
  a `Row` read with `row.get(users::email)` or the generated `row.email()`;
  `#[derive(FromRow)]` fills a plain domain struct by field name, with no
  column path or table in it. `into_tuples()` recovers the positional view.
- **A statement with nothing in it has no SQL form.** An `*Update` whose every
  field is untouched, or an insert of zero rows, hands back
  `NothingToSet`/`NothingToInsert` rather than rendering broken SQL. These are
  the only fallible builder methods; everything downstream is infallible.
- **The dialect is part of a query's type** — capability gating happens while
  the query is built, not when it renders — but it's never a turbofish:
  `.load(&pool)` infers it from the executor, `.to_sql(Postgres)` takes it as
  a value.

## Known limitations

Deferred rather than half-supported, and documented in the relevant module:
`WITH RECURSIVE`, aggregates as window functions (`sum(x) OVER (..)`), a CTE
referencing another CTE, row locking (`FOR UPDATE`/`SKIP LOCKED`), a scalar
subquery in an expression position (`col = (SELECT max(x) ..)`), and
relations/eager-loading. `sql!{}` doesn't reach the last two: it builds an
expression, not a statement suffix, and a `Select` isn't a slot value.
`IN (SELECT ..)`/`NOT IN (SELECT ..)` is covered by `Select::contains`/
`.not_contains`, which — like `EXISTS` — is dialect-pinned rather than a
plain expression.

Design constraints worth knowing before adopting:

- **Conditionally *joining* a table has no fully-static solution** — a single
  type can't mean "joined" in one branch and "not joined" in another.
  `.erase()` into `DynSelect` is the way out, and it's narrow: only the join
  skeleton is erased, and no further `.filter()`/`.join()` is offered on it.
  Conditional *filtering* needs none of this.
- **No table aliasing, so no self-join.** Two `#[derive(Table)]` structs must
  not share a `#[table(name = "..")]` — that compiles and then renders
  `FROM "t" JOIN "t"`, which the database refuses.
- **Nothing relates `GROUP BY`/`ORDER BY` to the selection list.** An
  aggregate or window function in `WHERE` or `RETURNING` is accepted by the
  builder and rejected by the database, and `.distinct()` sorted by an
  unselected column renders SQL Postgres won't take. Aggregates take a bare
  column: `sum(price * qty)` and `count(DISTINCT x)` need `sql!{}`.
- **A computed expression's nullability isn't derived** the way a column's is.
  An expression whose type the builder inferred — a comparison, an `is_null`,
  a `LIKE` — says what it decodes to once, with `.decodes_as::<Bool>()`; a
  `sql!{}` fragment states its type in the macro.
- **A selection list holds at most 32 elements** (`<table>::All` counts as
  one, whatever the column count). Naming a row type in a signature takes a
  type alias long enough to trip `clippy::type_complexity`; inference covers
  everything that stays inside a function, and `<table>::AllRow` covers a
  stored `select(All)`.
- **One `label!` per scope** — it declares a `label` module, and a scope holds
  one. List every name that scope needs in the one invocation.
- **The derives expand to `::qbrs::` paths**, so depend on the `qbrs` facade
  rather than on `qbrs-core` + `qbrs-macros` directly.
- **Every `?` in a `sql!{}` text is a slot**, with no escape for a literal one
  — MySQL and SQLite spell their bind parameters the same way. Its text must
  be a constant (a literal, a `const`, `concat!`, `include_str!`), so
  runtime-assembled text can never become SQL shape.

## Status

|                                                                             | Postgres |  MySQL  | SQLite  |
| --------------------------------------------------------------------------- | :------: | :-----: | :-----: |
| Query building & SQL rendering                                              |    ✅    |   ✅    |   ✅    |
| Rendered SQL executed in CI                                                 |    ✅    | not yet |   ✅    |
| Dialect capability gating (`RETURNING`, `ON CONFLICT`, `RIGHT`/`FULL JOIN`) |    ✅    |   ✅    |   ✅    |
| Execution (via `qbrs-sqlx`)                                                 |    ✅    | not yet | not yet |
| Transactions (via `qbrs-sqlx`)                                              |    ✅    | not yet | not yet |

MySQL is rendered and asserted as strings only, so its dialect differences are
caught only where someone thought to look. One known difference: `DEFAULT` in
an `INSERT ... VALUES` is Postgres and MySQL only — SQLite rejects it, which
makes `Defaultable::Default` unusable there.

## Development

- [`crates/core`](crates/core) — type-level machinery and SQL rendering. No
  I/O, no async, no driver.
- [`crates/macros`](crates/macros) — `#[derive(Table)]`, `#[derive(FromRow)]`,
  `label!`, `with!`.
- [`crates/qbrs`](crates/qbrs) — the facade; depend on this one.
- [`crates/qbrs-sqlx`](crates/qbrs-sqlx) — execution via `sqlx` (Postgres).
- [`examples`](examples) — runnable examples; [`tests/dialect-exec`](tests/dialect-exec)
  runs every rendered shape against SQLite; [`tests/compile-bench`](tests/compile-bench)
  backs the "linear at 40+ joins" claim.

```sh
cargo test --workspace --all-features
```

No setup required: the real-DB tests start their own throwaway PostgreSQL
17.5, embedded via [`pglite-rs`](https://crates.io/crates/pglite-rs) — no
Docker, no service to launch, nothing downloaded at test time. Set
`DATABASE_URL` to run them against an external Postgres instead.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Contributions are licensed the same way unless stated otherwise.
