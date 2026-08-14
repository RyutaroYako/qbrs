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

let rows: Vec<(String, Option<i64>)> = select((users::email, orders::total))
    .from::<Postgres, _>(users::Table)
    .left_join(orders::Table, orders::user_id.eq(users::id))
    .order_by(users::id.asc())
    .load(&pool)
    .await?;
```

```sql
-- rendered by qbrs:
SELECT "users"."email", "orders"."total"
FROM "users" LEFT JOIN "orders" ON ("orders"."user_id" = "users"."id")
ORDER BY "users"."id" ASC
```

`orders::total` is declared as a plain `i64` column — not `Option<i64>` — but
because it's on the far side of a `LEFT JOIN`, `rows`'s type is inferred as
`Vec<(String, Option<i64>)>` automatically. Forget the join and reference
`orders::total` anyway, and it's a compile error, not a runtime surprise:

```
error[E0277]: `Orders` is not available in this query's scope
  = help: add `.join(<table>, ..)` (or `.from(..)`) for `Orders`
          before referencing its columns here
```

## Why qbrs?

|                                                                                         |    qbrs     |                                            diesel                                            |       sea-query        |            sqlx            |
| --------------------------------------------------------------------------------------- | :---------: | :------------------------------------------------------------------------------------------: | :--------------------: | :------------------------: |
| Compile-time column/join validity checking                                              |     ✅      |                                              ✅                                              |    ❌ (runtime AST)    |    n/a — not a builder     |
| `NULL`-ability auto-derived from join kind                                              |     ✅      |                                  ❌ (manual `.nullable()`)                                   |           ❌           |            n/a             |
| Add predicates conditionally/in a loop, no escape hatch                                 |     ✅      |                                   ⚠️ needs `.into_boxed()`                                   | ✅ (dynamic by design) | ⚠️ drops to `QueryBuilder` |
| Table/column refs are plain values, not turbofish/closures                              |     ✅      |                                           partial                                            |           ✅           |            n/a             |
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
[Quick example](#quick-example) above. See [`examples/`](examples) for
complete, runnable code for everything below.

- **Select / Insert / Update / Delete** —
  [`01_select_basic`](examples/examples/01_select_basic.rs),
  [`03_insert`](examples/examples/03_insert.rs),
  [`04_update`](examples/examples/04_update.rs),
  [`05_delete`](examples/examples/05_delete.rs). A NOT NULL column with a
  schema default gets a three-state `Defaultable<Option<T>>` on `*Insert`
  (omit / explicit `NULL` / explicit value); `*Update` mirrors this with
  `Option<Option<T>>` (untouched / `NULL` / value).
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
  [`14_cte`](examples/examples/14_cte.rs). Non-recursive, single-level only.
- **Dynamic composition, no escape hatch** —
  [`07_dynamic_filters`](examples/examples/07_dynamic_filters.rs).
  `.filter()` doesn't change `Select`'s type, so it can be called
  conditionally or in a loop.
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

## Status

|                                                                             | Postgres |  MySQL  | SQLite  |
| --------------------------------------------------------------------------- | :------: | :-----: | :-----: |
| Query building & SQL rendering                                              |    ✅    |   ✅    |   ✅    |
| Dialect capability gating (`RETURNING`, `ON CONFLICT`, `RIGHT`/`FULL JOIN`) |    ✅    |   ✅    |   ✅    |
| Execution (via `qbrs-sqlx`)                                                 |    ✅    | not yet | not yet |
| Transactions (via `qbrs-sqlx`)                                              |    ✅    | not yet | not yet |

`SELECT`/`INSERT`/`UPDATE`/`DELETE`, all JOIN kinds, `GROUP BY`/`HAVING`,
correlated subqueries (`EXISTS`/`NOT EXISTS`), the `sql!{}` escape hatch,
reusable named-placeholder prepared statements (`prepare!{}`), upsert
(`ON CONFLICT`), `UNION`/`INTERSECT`/`EXCEPT`, ranking window functions
(`row_number()`/`rank()`/`dense_rank()`), and non-recursive CTEs (`with!{}`)
are implemented and tested against a real Postgres instance. Not yet done:
`WITH RECURSIVE`, aggregate-as-window-functions (`sum(col) OVER (..)`), and
relations/eager-loading (intentionally scoped out until the core
query-building layer has been stable for a while).

## Workspace layout

- [`crates/core`](crates/core) (`qbrs-core`) — the type-level machinery: scope
  tracking (`Cons`/`Nil`/`Find`), expressions, `Select`/`Insert`/`Update`/`Delete`
  builders, SQL rendering. No I/O, no async runtime.
- [`crates/macros`](crates/macros) (`qbrs-macros`) — `#[derive(Table)]`.
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
