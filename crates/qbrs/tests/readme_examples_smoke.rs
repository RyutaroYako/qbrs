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
fn the_readme_quick_example_renders_the_sql_its_comment_shows() {
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
