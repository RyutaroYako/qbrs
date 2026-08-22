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
#[allow(dead_code)]
struct Orders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

#[test]
fn schema_module_and_select_builder_work_together() {
    let (sql, params) = select((users::id, users::display_name, orders::total))
        .from::<Postgres, _>(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::active.eq(true))
        .order_by(users::id.desc())
        .limit(5)
        .to_sql();

    assert_eq!(
        sql,
        "SELECT \"users\".\"id\", \"users\".\"display_name\", \"orders\".\"total\" \
         FROM \"users\" LEFT JOIN \"orders\" ON (\"orders\".\"user_id\" = \"users\".\"id\") \
         WHERE (\"users\".\"active\" = $1) ORDER BY \"users\".\"id\" DESC LIMIT 5"
    );
    assert_eq!(params, vec![qbrs::expr::Value::Bool(true)]);
}

#[test]
fn insert_uses_generated_new_and_setters() {
    let (sql, params) = qbrs::insert::insert::<Postgres, _>(users::Table)
        .values(UsersInsert::new("a@example.com").display_name("Ada"))
        .returning(users::id)
        .to_sql();

    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\", \"display_name\", \"active\") VALUES ($1, $2, DEFAULT) RETURNING \"users\".\"id\""
    );
    assert_eq!(
        params,
        vec![
            qbrs::expr::Value::Text("a@example.com".into()),
            qbrs::expr::Value::Text("Ada".into()),
        ]
    );
}

#[test]
fn update_only_sends_touched_fields() {
    let (sql, params) = qbrs::update::update::<Postgres, _>(users::Table)
        .set(UsersUpdate {
            display_name: Some(Some("New Name".into())),
            ..Default::default()
        })
        .expect("display_name is set")
        .filter(users::id.eq(1i64))
        .to_sql();

    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"display_name\" = $1 WHERE (\"users\".\"id\" = $2)"
    );
    assert_eq!(
        params,
        vec![
            qbrs::expr::Value::Text("New Name".into()),
            qbrs::expr::Value::I64(1),
        ]
    );
}
