use qbrs::Table;
use qbrs::dialect::Postgres;
use qbrs::expr::ExprMethods;
use qbrs::select::{OrderExt, Predicate, predicate, select};
use qbrs::statement::Statement;
use qbrs::update::Assignments;

#[derive(Table)]
#[table(name = "users")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
    display_name: Option<String>,
    /// Nullable *and* defaulted: the three-state column.
    #[column(default)]
    nickname: Option<String>,
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
fn a_nullable_defaulted_column_says_its_three_states_apart() {
    // Omitted: the schema's default. `None`: the same, since that is what a
    // request field without a value means. `_null()`: an explicit NULL.
    let omitted = qbrs::insert::insert(users::Table)
        .values(UsersInsert::builder().email("a@example.com").build())
        .to_sql(Postgres)
        .0;
    assert!(
        omitted.ends_with("VALUES ($1, $2, DEFAULT, DEFAULT)"),
        "{omitted}"
    );

    let absent: Option<String> = None;
    let from_request = qbrs::insert::insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("a@example.com")
                .nickname(absent)
                .build(),
        )
        .to_sql(Postgres)
        .0;
    assert!(
        from_request.ends_with("VALUES ($1, $2, DEFAULT, DEFAULT)"),
        "{from_request}"
    );

    let (explicit, params) = qbrs::insert::insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("a@example.com")
                .nickname_null()
                .build(),
        )
        .to_sql(Postgres);
    assert!(
        explicit.ends_with("VALUES ($1, $2, $3, DEFAULT)"),
        "{explicit}"
    );
    assert_eq!(params[2], qbrs::expr::Value::NullText);
}

#[test]
fn schema_module_and_select_builder_work_together() {
    let (sql, params) = select((users::id, users::display_name, orders::total))
        .from(users::Table)
        .left_join(orders::Table, orders::user_id.eq(users::id))
        .filter(users::active.eq(true))
        .order_by(users::id.desc())
        .limit(5)
        .to_sql(Postgres);

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
    let (sql, params) = qbrs::insert::insert(users::Table)
        .values(
            UsersInsert::builder()
                .email("a@example.com")
                .display_name("Ada")
                .build(),
        )
        .returning(users::id)
        .to_sql(Postgres);

    assert_eq!(
        sql,
        "INSERT INTO \"users\" (\"email\", \"display_name\", \"nickname\", \"active\") \
         VALUES ($1, $2, DEFAULT, DEFAULT) RETURNING \"users\".\"id\""
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
    let (sql, params) = qbrs::update::update(users::Table)
        .set(
            Assignments::from_row(UsersUpdate {
                display_name: Some(Some("New Name".into())),
                ..Default::default()
            })
            .expect("display_name is set"),
        )
        .filter(users::id.eq(1i64))
        .to_sql(Postgres);

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

#[derive(Table)]
#[table(name = "flags")]
#[allow(dead_code)]
struct Flags {
    #[column(primary_key, generated)]
    id: i64,
    opted_in: Option<bool>,
}

#[test]
fn a_nullable_boolean_column_is_a_condition_on_its_own() {
    let (sql, params) = select((flags::id,))
        .from(flags::Table)
        .filter(flags::opted_in)
        .to_sql(Postgres);

    assert_eq!(
        sql,
        "SELECT \"flags\".\"id\" FROM \"flags\" WHERE \"flags\".\"opted_in\""
    );
    assert!(params.is_empty());
}

#[test]
fn assigning_a_column_twice_keeps_the_last_assignment() {
    let (sql, params) = qbrs::update::update(users::Table)
        .set(
            Assignments::from_row(UsersUpdate {
                display_name: Some(Some("From the request".into())),
                ..Default::default()
            })
            .expect("display_name is set"),
        )
        .set_to(users::display_name, "Computed")
        .to_sql(Postgres);

    assert_eq!(sql, "UPDATE \"users\" SET \"display_name\" = $1");
    assert_eq!(params, vec![qbrs::expr::Value::Text("Computed".into())]);
}

#[test]
fn set_to_assigns_a_typed_sql_null() {
    let (sql, params) = qbrs::update::update(users::Table)
        .set_to(users::display_name, qbrs::expr::null::<qbrs::expr::Text>())
        .filter(users::id.eq(1i64))
        .to_sql(Postgres);

    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"display_name\" = $1 WHERE (\"users\".\"id\" = $2)"
    );
    assert_eq!(
        params,
        vec![qbrs::expr::Value::NullText, qbrs::expr::Value::I64(1)]
    );
}

#[test]
fn an_update_builder_leaves_an_absent_request_field_untouched() {
    // The struct literal spells a nullable column's three states as
    // `Option<Option<T>>`; the builder takes what a request holds, so an
    // absent field can't turn into `SET display_name = NULL`.
    let absent: Option<String> = None;
    let patch = UsersUpdate::builder()
        .display_name(absent)
        .nickname("Ada")
        .build();

    let (sql, params) = qbrs::update::update(users::Table)
        .set(Assignments::from_row(patch).expect("nickname is set"))
        .to_sql(Postgres);
    assert_eq!(sql, "UPDATE \"users\" SET \"nickname\" = $1");
    assert_eq!(params, vec![qbrs::expr::Value::Text("Ada".into())]);

    // The explicit NULL is its own call, as it is on the insert builder.
    let cleared = UsersUpdate::builder().display_name_null().build();
    let (sql, params) = qbrs::update::update(users::Table)
        .set(Assignments::from_row(cleared).expect("display_name is set"))
        .to_sql(Postgres);
    assert_eq!(sql, "UPDATE \"users\" SET \"display_name\" = $1");
    assert_eq!(params, vec![qbrs::expr::Value::NullText]);
}

#[test]
fn all_row_names_what_select_all_decodes_to() {
    fn take(row: users::AllRow) -> i64 {
        *row.get(users::id)
    }
    let _ = take;
}

#[test]
fn an_exists_composes_with_other_conditions_once_discharged() {
    // `Exists` is dialect-pinned, but discharging it doesn't give that up:
    // a `Predicate` carries the dialect too, so an `EXISTS` can sit in an
    // OR beside an ordinary comparison.
    let base = select((users::email,)).from(users::Table);
    let has_flag = base
        .correlated(flags::Table, (flags::id,))
        .filter(flags::id.eq(users::id));

    let (sql, params) = base
        .clone()
        .filter(Predicate::any_of([
            predicate(users::active.eq(false)),
            predicate(has_flag.exists()),
        ]))
        .to_sql(Postgres);

    assert_eq!(
        sql,
        "SELECT \"users\".\"email\" FROM \"users\" WHERE ((\"users\".\"active\" = $1) OR (EXISTS (SELECT \"flags\".\"id\" FROM \"flags\" WHERE (\"flags\".\"id\" = \"users\".\"id\"))))"
    );
    assert_eq!(params, vec![qbrs::expr::Value::Bool(false)]);
}

#[test]
fn a_write_statement_builds_its_own_correlated_exists() {
    // The same `EXISTS` a `SELECT` builds, against the one-table scope an
    // `UPDATE` has — no throwaway `Select` to hang it on.
    let statement = qbrs::update::update(users::Table).set_to(users::active, false);
    let has_flag = statement
        .correlated(flags::Table, (flags::id,))
        .filter(flags::id.eq(users::id));

    let (sql, params) = statement.filter(has_flag.exists()).to_sql(Postgres);
    assert_eq!(
        sql,
        "UPDATE \"users\" SET \"active\" = $1 WHERE (EXISTS (SELECT \"flags\".\"id\" FROM \"flags\" WHERE (\"flags\".\"id\" = \"users\".\"id\")))"
    );
    assert_eq!(params, vec![qbrs::expr::Value::Bool(false)]);
}
