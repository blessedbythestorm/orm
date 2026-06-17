/// Maps a Rust type to its Postgres column type. Implemented for scalars here,
/// and for `#[enum_type]` / `#[json_type]` definitions by their macros, so the
/// `#[table_type]` macro can resolve every column without knowing the type.
pub trait SqlType {
    const SQL_TYPE: &'static str;
    const NULLABLE: bool;
}

macro_rules! scalar_sql_types {
    ($($rust_type:ty => $sql_type:literal),* $(,)?) => {
        $(
            impl SqlType for $rust_type {
                const SQL_TYPE: &'static str = $sql_type;
                const NULLABLE: bool = false;
            }
        )*
    };
}

scalar_sql_types! {
    bool => "boolean",
    i16 => "int2",
    i32 => "int4",
    i64 => "int8",
    f32 => "real",
    f64 => "double precision",
    String => "text",
    Vec<u8> => "bytea",
    uuid::Uuid => "uuid",
    chrono::NaiveDate => "date",
    chrono::NaiveDateTime => "timestamp",
    chrono::DateTime<chrono::Utc> => "timestamptz",
    serde_json::Value => "jsonb",
}

impl<T: SqlType> SqlType for Option<T> {
    const SQL_TYPE: &'static str = T::SQL_TYPE;
    const NULLABLE: bool = true;
}
