//! Guards against README doc-drift: the generated-SQL comment shown in
//! README.md's "Quick example" section is asserted here against the actual
//! renderer, so a future change to rendering that silently breaks the
//! README's claim fails CI instead of just looking wrong to whoever reads
//! the docs next. Every other feature's example code lives in `examples/`
//! instead of the README, where it's compiled and run against real Postgres.

use qbrs::Table;
use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs::select::{OrderExt, select};

#[derive(Table)]
#[table(name = "users")]
#[allow(dead_code)]
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
#[allow(dead_code)]
struct Orders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

#[test]
fn readme_quick_example_matches_actual_output() {
    let (sql, _params) = select((users::email, orders::total))
        .from(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .order_by(users::id.asc())
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"email\", \"orders\".\"total\" FROM \"users\" LEFT JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\") ORDER BY \"users\".\"id\" ASC"
    );
}
