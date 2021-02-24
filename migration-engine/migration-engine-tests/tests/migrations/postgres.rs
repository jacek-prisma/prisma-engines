use migration_engine_tests::*;
use quaint::prelude::Queryable;
use sql_schema_describer::{ColumnArity, ColumnTypeFamily};
use std::fmt::Write;

#[test_each_connector(tags("postgres"))]
async fn enums_can_be_dropped_on_postgres(api: &TestApi) -> TestResult {
    let dm1 = r#"
        model Cat {
            id String @id
            name String
            mood CatMood
        }

        enum CatMood {
            ANGRY
            HUNGRY
            CUDDLY
        }
    "#;

    api.schema_push(dm1).send().await?.assert_green()?;
    api.assert_schema()
        .await?
        .assert_enum("CatMood", |r#enum| r#enum.assert_values(&["ANGRY", "HUNGRY", "CUDDLY"]))?;

    let dm2 = r#"
        model Cat {
            id String @id
            name String
        }
    "#;

    api.schema_push(dm2).send().await?.assert_green()?;
    api.assert_schema().await?.assert_has_no_enum("CatMood")?;

    Ok(())
}

#[test_each_connector(capabilities("scalar_lists"))]
async fn adding_a_scalar_list_for_a_model_with_id_type_int_must_work(api: &TestApi) -> TestResult {
    let dm1 = r#"
        datasource pg {
            provider = "postgres"
            url = "postgres://localhost:5432"
        }

        model A {
            id Int @id
            strings String[]
            enums Status[]
        }

        enum Status {
            OK
            ERROR
        }
    "#;

    api.schema_push(dm1).send().await?.assert_green()?;

    let result = api.describe_database().await?;

    let table_for_a = result.table_bang("A");
    let string_column = table_for_a.column_bang("strings");
    assert_eq!(string_column.tpe.family, ColumnTypeFamily::String);
    assert_eq!(string_column.tpe.arity, ColumnArity::List);

    let enum_column = table_for_a.column_bang("enums");
    assert_eq!(enum_column.tpe.family, ColumnTypeFamily::Enum("Status".to_owned()));
    assert_eq!(enum_column.tpe.arity, ColumnArity::List);

    Ok(())
}

// Reference for the tables created by PostGIS: https://postgis.net/docs/manual-1.4/ch04.html#id418599
#[test_each_connector(tags("postgres"))]
async fn existing_postgis_tables_must_not_be_migrated(api: &TestApi) -> TestResult {
    let create_spatial_ref_sys_table = "CREATE TABLE IF NOT EXISTS \"spatial_ref_sys\" ( id SERIAL PRIMARY KEY )";
    // The capitalized Geometry is intentional here, because we want the matching to be case-insensitive.
    let create_geometry_columns_table = "CREATE TABLE IF NOT EXiSTS \"Geometry_columns\" ( id SERIAL PRIMARY KEY )";

    api.database().execute_raw(create_spatial_ref_sys_table, &[]).await?;
    api.database().execute_raw(create_geometry_columns_table, &[]).await?;

    api.assert_schema()
        .await?
        .assert_has_table("spatial_ref_sys")?
        .assert_has_table("Geometry_columns")?;

    let schema = "";

    api.schema_push(schema)
        .send()
        .await?
        .assert_green()?
        .assert_no_steps()?;

    api.assert_schema()
        .await?
        .assert_has_table("spatial_ref_sys")?
        .assert_has_table("Geometry_columns")?;

    Ok(())
}

#[test_each_connector(tags("postgres"), log = "debug")]
async fn native_type_columns_can_be_created(api: &TestApi) -> TestResult {
    let types = &[
        ("smallint", "Int", "SmallInt", "int2"),
        ("int", "Int", "Integer", "int4"),
        ("bigint", "BigInt", "BigInt", "int8"),
        ("decimal", "Decimal", "Decimal(4, 2)", "numeric"),
        ("decimaldefault", "Decimal", "Decimal", "numeric"),
        ("real", "Float", "Real", "float4"),
        ("doublePrecision", "Float", "DoublePrecision", "float8"),
        ("varChar", "String", "VarChar(200)", "varchar"),
        ("char", "String", "Char(200)", "bpchar"),
        ("text", "String", "Text", "text"),
        ("bytea", "Bytes", "ByteA", "bytea"),
        ("ts", "DateTime", "Timestamp(0)", "timestamp"),
        ("tsdefault", "DateTime", "Timestamp", "timestamp"),
        ("tstz", "DateTime", "Timestamptz", "timestamptz"),
        ("date", "DateTime", "Date", "date"),
        ("time", "DateTime", "Time(2)", "time"),
        ("timedefault", "DateTime", "Time", "time"),
        ("timetz", "DateTime", "Timetz(2)", "timetz"),
        ("timetzdefault", "DateTime", "Timetz", "timetz"),
        ("bool", "Boolean", "Boolean", "bool"),
        ("bit", "String", "Bit(1)", "bit"),
        ("varbit", "String", "VarBit(1)", "varbit"),
        ("uuid", "String", "Uuid", "uuid"),
        ("xml", "String", "Xml", "xml"),
        ("json", "Json", "Json", "json"),
        ("jsonb", "Json", "JsonB", "jsonb"),
        ("money", "Decimal", "Money", "money"),
        ("inet", "String", "Inet", "inet"),
        ("oid", "Int", "Oid", "oid"),
    ];

    let mut dm = r#"
        datasource pg {
            provider = "postgres"
            url = "postgresql://localhost/test"
        }

        generator client {
          provider = "prisma-client-js"
          previewFeatures = ["nativeTypes"]
        }

        model A {
            id Int @id
    "#
    .to_owned();

    for (field_name, prisma_type, native_type, _) in types {
        writeln!(&mut dm, "    {} {} @pg.{}", field_name, prisma_type, native_type)?;
    }

    dm.push_str("}\n");

    api.schema_push(&dm).send().await?.assert_green()?;

    api.assert_schema().await?.assert_table("A", |table| {
        types.iter().fold(
            Ok(table),
            |table, (field_name, _prisma_type, _native_type, database_type)| {
                table.and_then(|table| table.assert_column(field_name, |col| col.assert_full_data_type(database_type)))
            },
        )
    })?;

    api.schema_push(dm).send().await?.assert_green()?.assert_no_steps()?;

    Ok(())
}

#[test_each_connector(tags("postgres"))]
async fn uuids_do_not_generate_drift_issue_5282(api: &TestApi) -> TestResult {
    api.database().raw_cmd(
        r#"
        CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
        CREATE TABLE a (id uuid DEFAULT uuid_generate_v4() primary key);
        CREATE TABLE b (id uuid DEFAULT uuid_generate_v4() primary key, a_id uuid, CONSTRAINT aaa FOREIGN KEY (a_id) REFERENCES a(id));
        "#
    ).await?;

    let dm = api.native_types_datamodel(
        r#"
        model a {
            id String @id @default(dbgenerated("uuid_generate_v4()")) @test_db.Uuid
            b  b[]
        }

        model b {
            id   String  @id @default(dbgenerated("uuid_generate_v4()")) @test_db.Uuid
            a_id String? @test_db.Uuid
            a    a?      @relation(fields: [a_id], references: [id])
        }
        "#,
    );

    api.schema_push(&dm)
        .migration_id(Some("first"))
        .send()
        .await?
        .assert_green()?
        .assert_no_steps()?;

    Ok(())
}
