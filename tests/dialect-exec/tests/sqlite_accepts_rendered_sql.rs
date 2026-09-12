//! Every statement qbrs renders for `Sqlite` is handed to a real SQLite and
//! executed. String assertions elsewhere say the SQL looks right; only this
//! says SQLite agrees.

use qbrs::cte::with;
use qbrs::expr::{Expr, Value, all_of, any_of, avg, count, count_of, max, min, string_agg, sum};
use qbrs::insert::partial_index;
use qbrs::prelude::*;
use qbrs::sql;
use sqlx::{Row, SqlitePool};

#[derive(Table)]
#[table(name = "users")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
    display_name: Option<String>,
}

#[derive(Table)]
#[table(name = "archived_orders")]
#[allow(dead_code)]
struct ArchivedOrders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

#[derive(Table)]
#[table(name = "orders")]
#[allow(dead_code)]
struct Orders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

with! {
    struct big_orders { user_id: BigInt, total: Nullable<BigInt> }
}

async fn schema() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite");
    for ddl in [
        "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, email TEXT NOT NULL UNIQUE, display_name TEXT)",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, total INTEGER NOT NULL)",
        "CREATE TABLE archived_orders (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, total INTEGER NOT NULL)",
        // A partial unique index, so a conflict target carrying an index
        // predicate has something to be inferred against.
        "CREATE UNIQUE INDEX users_named ON users (display_name) WHERE display_name IS NOT NULL",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(ddl))
            .execute(&pool)
            .await
            .expect("create schema");
    }
    pool
}

async fn run(
    pool: &SqlitePool,
    (sql, params): (String, Vec<Value>),
) -> Vec<sqlx::sqlite::SqliteRow> {
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.clone()));
    for p in params {
        q = match p {
            Value::I32(v) => q.bind(v),
            Value::I64(v) => q.bind(v),
            Value::F64(v) => q.bind(v),
            Value::Text(v) => q.bind(v),
            Value::Bool(v) => q.bind(v),
            Value::Bytes(v) => q.bind(v),
            Value::NullI32 => q.bind(None::<i32>),
            Value::NullI64 => q.bind(None::<i64>),
            Value::NullF64 => q.bind(None::<f64>),
            Value::NullText => q.bind(None::<String>),
            Value::NullBool => q.bind(None::<bool>),
            Value::NullBytes => q.bind(None::<Vec<u8>>),
            Value::Placeholder(name) => panic!("unresolved placeholder {name}"),
            // The feature-gated column types decode through their own
            // crates, which this SQLite battery doesn't pull in — and which
            // aren't there at all unless those features are on.
            #[allow(unreachable_patterns)]
            other => panic!("no SQLite binding for {other:?}"),
        };
    }
    q.fetch_all(pool)
        .await
        .unwrap_or_else(|e| panic!("sqlite rejected `{sql}`: {e}"))
}

#[tokio::test]
async fn sqlite_executes_every_rendered_statement_shape() {
    let pool = schema().await;

    run(
        &pool,
        insert(users::Table)
            .values(
                UsersInsert::builder()
                    .email("ada@example.com")
                    .display_name("Ada")
                    .build(),
            )
            .values(UsersInsert::builder().email("dan@example.com").build())
            .to_sql(Sqlite),
    )
    .await;

    let ids = run(
        &pool,
        insert(orders::Table)
            .values(OrdersInsert::builder().user_id(1).total(100).build())
            .values(OrdersInsert::builder().user_id(1).total(2000).build())
            .values(OrdersInsert::builder().user_id(2).total(30).build())
            .returning(orders::id)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(ids.len(), 3);

    run(
        &pool,
        insert(users::Table)
            .values(
                UsersInsert::builder()
                    .email("ada@example.com")
                    .display_name("Ada L.")
                    .build(),
            )
            .on_conflict_do_update(
                users::email,
                Assignments::from_row(UsersUpdate {
                    display_name: Some(Some("Ada Lovelace".into())),
                    ..Default::default()
                })
                .expect("display_name is set"),
            )
            .to_sql(Sqlite),
    )
    .await;

    run(
        &pool,
        update(users::Table)
            .set(
                Assignments::from_row(UsersUpdate {
                    display_name: Some(None),
                    ..Default::default()
                })
                .expect("display_name is set"),
            )
            .filter(users::email.eq("dan@example.com"))
            .to_sql(Sqlite),
    )
    .await;

    let joined = run(
        &pool,
        select((users::email, orders::total))
            .from(users::Table)
            .left_join(orders::Table, orders::user_id.eq(users::id))
            .filter(users::email.eq("ada@example.com"))
            .order_by(orders::total.desc())
            .limit(10)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(joined.len(), 2);

    let grouped = run(
        &pool,
        select((orders::user_id, sum(orders::total), count()))
            .from(orders::Table)
            .group_by(orders::user_id)
            .having(sum(orders::total).gt(50i64))
            .to_sql(Sqlite),
    )
    .await;
    // Only user 1 clears the HAVING threshold.
    assert_eq!(grouped.len(), 1);

    // A bare OFFSET: SQLite has no `OFFSET` without a `LIMIT`, so the
    // dialect fills one in.
    let offset_only = run(
        &pool,
        select(users::email)
            .from(users::Table)
            .order_by(users::id.asc())
            .offset(1)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(offset_only.len(), 1);

    // Branches that carry their own `ORDER BY`/`LIMIT`, or a `WITH`: each
    // binds to the branch, not to the compound, which is what the derived
    // table around every SQLite branch is for.
    let paged_union = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .order_by(users::id.asc())
            .limit(1)
            .union(&select((users::email,)).from(users::Table))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(paged_union.len(), 2);

    let ordered_union = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .order_by(users::id.asc())
            .union(&select((users::email,)).from(users::Table))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(ordered_union.len(), 2);

    let cte_branch = run(
        &pool,
        select((big_orders::user_id,))
            .from(users::Table)
            .inner_join(
                with(
                    big_orders::Table,
                    &select((orders::user_id, sum(orders::total)))
                        .from(orders::Table)
                        .group_by(orders::user_id),
                ),
                big_orders::user_id.eq(users::id),
            )
            .union(&select((orders::user_id,)).from(orders::Table))
            .to_sql(Sqlite),
    )
    .await;
    assert!(!cte_branch.is_empty());

    let union = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .filter(users::email.eq("ada@example.com"))
            .union(
                &select((users::email,))
                    .from(users::Table)
                    .filter(users::email.eq("dan@example.com")),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(union.len(), 2);

    let totals = select((orders::user_id, sum(orders::total)))
        .from(orders::Table)
        .group_by(orders::user_id);
    let cte = run(
        &pool,
        select((users::email, big_orders::total))
            .from(users::Table)
            .inner_join(
                with(big_orders::Table, &totals),
                big_orders::user_id.eq(users::id),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(cte.len(), 2);

    let exists_q = select(users::email).from(users::Table);
    let correlated = exists_q
        .correlated(orders::Table, orders::id)
        .filter(orders::user_id.eq(users::id))
        .exists();
    let rows = run(&pool, exists_q.filter(correlated).to_sql(Sqlite)).await;
    assert_eq!(rows.len(), 2);

    // `IN (<subquery>)`: only user 1 (ada) has an order over 50.
    let big_spenders = select(orders::user_id)
        .from(orders::Table)
        .filter(orders::total.gt(50i64));
    let contains_rows = run(
        &pool,
        select(users::email)
            .from(users::Table)
            .filter(big_spenders.contains(users::id))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(contains_rows.len(), 1);

    let not_contains_rows = run(
        &pool,
        select(users::email)
            .from(users::Table)
            .filter(big_spenders.not_contains(users::id))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(not_contains_rows.len(), 1);

    qbrs::label!(rank_in_user);

    let ranked = run(
        &pool,
        select((
            orders::user_id,
            row_number()
                .over(
                    window()
                        .partition_by(orders::user_id)
                        .order_by(orders::total.desc()),
                )
                .label(label::rank_in_user),
        ))
        .from(orders::Table)
        .to_sql(Sqlite),
    )
    .await;
    assert_eq!(ranked.len(), 3);

    let right = run(
        &pool,
        select((users::email, orders::total))
            .from(orders::Table)
            .right_join(users::Table, orders::user_id.eq(users::id))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(right.len(), 3);

    let raw = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .filter(sql!(Bool, "length(email) > ?", 3i64))
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(raw.len(), 2);

    let distinct = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .inner_join(orders::Table, orders::user_id.eq(users::id))
            .distinct()
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(distinct.len(), 2);

    // `.order_by_selected(..)` is what a `SELECT DISTINCT` needs its sort
    // key to satisfy — checked against the selection at compile time,
    // rather than only discovered when the database rejects it.
    let distinct_sorted = run(
        &pool,
        select((users::email, users::display_name))
            .from(users::Table)
            .distinct()
            .order_by_selected(users::email, SortDir::Asc)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(distinct_sorted.len(), 2);

    let bulk = run(
        &pool,
        insert(orders::Table)
            .values_all((10..13).map(|n| OrdersInsert::builder().user_id(1).total(n).build()))
            .expect("three rows")
            .returning(orders::id)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(bulk.len(), 3);

    let deleted = run(
        &pool,
        delete(orders::Table)
            .filter(orders::total.lt(50i64))
            .returning(orders::id)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(deleted.len(), 4);

    let remaining = run(&pool, select(count()).from(orders::Table).to_sql(Sqlite)).await;
    assert_eq!(remaining[0].get::<i64, _>(0), 2);

    // `count_sql` is its own rendering — the query wrapped in a total, with
    // its paging dropped — so it is executed here rather than only asserted
    // as a string.
    let total = run(
        &pool,
        select((orders::id,))
            .from(orders::Table)
            .filter(orders::total.gt(0i64))
            .order_by(orders::id.desc())
            .limit(1)
            .count_sql(Sqlite),
    )
    .await;
    assert_eq!(total[0].get::<i64, _>(0), 2);

    let grouped_total = run(
        &pool,
        select((orders::user_id, count()))
            .from(orders::Table)
            .group_by(orders::user_id)
            .having_all(vec![predicate(count().gte(1i64))])
            .count_sql(Sqlite),
    )
    .await;
    assert_eq!(grouped_total[0].get::<i64, _>(0), 1);

    let full = run(
        &pool,
        select((users::email, orders::total))
            .from(users::Table)
            .full_join(orders::Table, orders::user_id.eq(users::id))
            .to_sql(Sqlite),
    )
    .await;
    assert!(!full.is_empty());

    let intersected = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .intersect(
                &select((users::email,))
                    .from(users::Table)
                    .filter(users::email.like("ada%")),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(intersected.len(), 1);

    let excepted = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .except(
                &select((users::email,))
                    .from(users::Table)
                    .filter(users::email.like("ada%")),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(excepted.len(), 1);

    let erased = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .erase()
            .limit(1)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(erased.len(), 1);

    // The expression arms a single `.filter()` never reaches: `AND`/`OR`
    // between two conditions, `NOT`, `IS NULL`, `!=`, `<=`, and the
    // `TRUE`/`FALSE` an empty `all_of`/`any_of` folds to.
    let combined = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .filter(
                users::display_name
                    .is_not_null()
                    .and(users::email.ne("nobody@example.com"))
                    .or(users::id.lte(0i64)),
            )
            .filter(!users::display_name.is_null())
            .filter(all_of(Vec::<Expr<Nil, Bool>>::new()))
            .filter(any_of([users::id.gte(1i64)]))
            .to_sql(Sqlite),
    )
    .await;
    assert!(!combined.is_empty());

    // `UPDATE .. SET <column> = <expression>` and `ON CONFLICT DO NOTHING`.
    run(
        &pool,
        qbrs::update::update(users::Table)
            .set_to(users::email, sql!(Text, "lower(?)", users::email))
            .filter(users::id.eq(1i64))
            .to_sql(Sqlite),
    )
    .await;

    run(
        &pool,
        insert(users::Table)
            .values(UsersInsert::builder().email("ada@example.com").build())
            .on_conflict_do_nothing(users::email)
            .to_sql(Sqlite),
    )
    .await;

    // A conflict target that names a partial unique index. SQLite rejects
    // a target it cannot infer an index from, so reaching this one at all
    // is what says the predicate rendered where SQLite reads it — and the
    // row it conflicts with is inserted here rather than borrowed from
    // 400 lines up, so what the `RETURNING` says is about this shape.
    let seeded = run(
        &pool,
        insert(users::Table)
            .values(
                UsersInsert::builder()
                    .email("partial-seed@example.com")
                    .display_name("Partial Seed")
                    .build(),
            )
            .returning(users::id)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(seeded.len(), 1);

    let dropped = run(
        &pool,
        insert(users::Table)
            .values(
                UsersInsert::builder()
                    .email("partial-again@example.com")
                    .display_name("Partial Seed")
                    .build(),
            )
            .on_conflict_do_nothing(partial_index(
                users::display_name,
                users::display_name.is_not_null(),
            ))
            .returning(users::id)
            .to_sql(Sqlite),
    )
    .await;
    assert!(
        dropped.is_empty(),
        "the partial index should have dropped the row"
    );

    // `count(<column>)` counts non-NULLs, unlike `count(*)`.
    let counted = run(
        &pool,
        select((count(), count_of(users::display_name)))
            .from(users::Table)
            .to_sql(Sqlite),
    )
    .await;
    assert!(counted[0].get::<i64, _>(0) >= counted[0].get::<i64, _>(1));

    // A set operation's own total.
    let union_total = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .union(
                &select((users::email,))
                    .from(users::Table)
                    .filter(users::email.like("ada%")),
            )
            .count_sql(Sqlite),
    )
    .await;
    assert!(union_total[0].get::<i64, _>(0) >= 1);

    // `ORDER BY <ordinal>` + paging on a set operation is the one place
    // SQLite's derived-table branch wrapping and its ordinal ordering meet.
    let paged_union = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .union_all(
                &select((users::email,))
                    .from(users::Table)
                    .filter(users::email.like("ada%")),
            )
            .order_by_column(users::email, SortDir::Desc)
            .limit(2)
            .offset(1)
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(paged_union.len(), 2);

    // A separator SQLite would read as syntax if it were written out
    // instead of bound, so the returned string says which of the two it is.
    let names = run(
        &pool,
        select((string_agg(users::email, r"')--"),))
            .from(users::Table)
            .filter(users::email.like("%@example.com"))
            .to_sql(Sqlite),
    )
    .await;
    let emails = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .filter(users::email.like("%@example.com"))
            .order_by(users::email.asc())
            .to_sql(Sqlite),
    )
    .await;
    let expected: Vec<String> = emails.iter().map(|row| row.get(0)).collect();
    let joined: Option<String> = names[0].get(0);
    let joined = joined.expect("the users run together");
    let mut parts: Vec<&str> = joined.split("')--").collect();
    parts.sort_unstable();
    assert_eq!(
        parts,
        expected.iter().map(String::as_str).collect::<Vec<_>>()
    );

    // The aggregates that render a `CAST`, and a `NOT EXISTS`.
    let stats = run(
        &pool,
        select((avg(orders::total), min(orders::total), max(orders::total)))
            .from(orders::Table)
            .filter(
                select((orders::id,))
                    .from(orders::Table)
                    .filter(orders::total.lt(0i64))
                    .not_exists(),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(stats.len(), 1);

    // A mixed chain: SQLite reads compound operators left to right and
    // Postgres binds `INTERSECT` tighter, so the rendered nesting is what
    // makes the two agree with the builder.
    let mixed = run(
        &pool,
        select((users::email,))
            .from::<Sqlite, _>(users::Table)
            .union(
                &select((users::email,))
                    .from::<Sqlite, _>(users::Table)
                    .filter(users::email.like("dan%")),
            )
            .intersect(
                &select((users::email,))
                    .from::<Sqlite, _>(users::Table)
                    .filter(users::email.like("ada%")),
            )
            .to_sql(Sqlite),
    )
    .await;
    assert_eq!(mixed.len(), 1);

    // `INSERT INTO t (..) SELECT ..` into a table of its own, so the rows
    // it copies can be counted rather than the statement merely parsed.
    // The source query's bind is numbered by the statement it lands in.
    run(
        &pool,
        insert(archived_orders::Table)
            .select(
                &select((orders::user_id, orders::total))
                    .from(orders::Table)
                    .filter(orders::total.gt(50i64)),
            )
            .to_sql(Sqlite),
    )
    .await;
    let archived = run(
        &pool,
        select((archived_orders::total,))
            .from(archived_orders::Table)
            .to_sql(Sqlite),
    )
    .await;
    assert!(
        archived.iter().all(|row| row.get::<i64, _>(0) > 50),
        "only the orders over 50 should have been copied"
    );

    // A `SELECT` with no `FROM`. SQLite has it too, and its own bind is
    // numbered the same way.
    let computed = run(
        &pool,
        select((sql!(BigInt, "(? + ?)", 2i64, 3i64),)).to_sql(Sqlite),
    )
    .await;
    assert_eq!(computed[0].get::<i64, _>(0), 5);

    let in_list = run(
        &pool,
        select((users::email,))
            .from(users::Table)
            .filter(users::email.is_in(["ada@example.com".to_string()]))
            .filter(users::id.is_in(Vec::<i64>::new()))
            .to_sql(Sqlite),
    )
    .await;
    assert!(in_list.is_empty());

    // One bind-carrying fragment in three clauses. Under Sqlite each `?` is
    // its own parameter, so the statement carries the bind three times and
    // it is SQLite, not a string assertion, that says the three still line
    // up with the values handed to it.
    let bucket = sql!(BigInt, "((? / ?) * ?)", orders::total, 1000i64, 1000i64);
    let buckets = run(
        &pool,
        select((bucket.clone(), count()))
            .from(orders::Table)
            .group_by(bucket.clone())
            .order_by(bucket.asc())
            .to_sql(Sqlite),
    )
    .await;
    assert!(!buckets.is_empty());
}
