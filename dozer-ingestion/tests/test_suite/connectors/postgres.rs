use dozer_ingestion::connectors::postgres::connector::{PostgresConfig, PostgresConnector};
use dozer_types::types::Field;
use dozer_utils::{process::run_docker_compose, Cleanup};
use tempdir::TempDir;

use crate::test_suite::{
    records::Operation, CudConnectorTest, DataReadyConnectorTest, FieldsAndPk,
    InsertOnlyConnectorTest,
};

use super::sql::{
    create_schema, create_table, create_table_with_all_supported_data_types, insert_record,
    operation_to_sql, schema_to_sql,
};

pub struct PostgresConnectorTest {
    config: postgres::Config,
    schema_name: Option<String>,
    table_name: String,
    schema: FieldsAndPk,
    _cleanup: Cleanup,
    _temp_dir: TempDir,
}

impl DataReadyConnectorTest for PostgresConnectorTest {
    type Connector = PostgresConnector;

    fn new() -> (Self, Self::Connector) {
        let (mut client, connector_test, connector) = create_postgres_server();
        client
            .batch_execute(&create_table_with_all_supported_data_types("test_table"))
            .unwrap();

        (connector_test, connector)
    }
}

impl InsertOnlyConnectorTest for PostgresConnectorTest {
    type Connector = PostgresConnector;

    fn new(
        schema_name: Option<String>,
        table_name: String,
        schema: FieldsAndPk,
        records: Vec<Vec<Field>>,
    ) -> Option<(Self, Self::Connector, FieldsAndPk)> {
        let (mut client, mut connector_test, connector) = create_postgres_server();

        let (actual_schema, _) = schema_to_sql(schema.clone());

        if let Some(schema_name) = &schema_name {
            client
                .batch_execute(&create_schema(schema_name))
                .expect("Failed to create schema");
        }

        let query = create_table(schema_name.as_deref(), &table_name, &actual_schema);
        client
            .batch_execute(&query)
            .expect("Failed to create table");

        for record in records {
            let query = insert_record(schema_name.as_deref(), &table_name, &record, &schema.0);
            client
                .batch_execute(&query)
                .expect("Failed to insert record");
        }

        connector_test.schema_name = schema_name;
        connector_test.table_name = table_name;
        connector_test.schema = schema;

        Some((connector_test, connector, actual_schema))
    }
}

impl CudConnectorTest for PostgresConnectorTest {
    fn start_cud(&self, operations: Vec<Operation>) {
        let mut client = self.config.connect(postgres::NoTls).unwrap();
        let schema_name = self.schema_name.clone();
        let table_name = self.table_name.clone();
        let schema = self.schema.clone();
        std::thread::spawn(move || {
            for operation in operations {
                client
                    .batch_execute(&operation_to_sql(
                        schema_name.as_deref(),
                        &table_name,
                        &operation,
                        &schema,
                    ))
                    .unwrap();
            }
        });
    }
}

fn create_postgres_server() -> (postgres::Client, PostgresConnectorTest, PostgresConnector) {
    let host = "localhost";
    let port = 5432;
    let user = "postgres";
    let password = "postgres";
    let dbname = "dozer-test";

    let temp_dir = TempDir::new("postgres").expect("Failed to create temp dir");
    let docker_compose_path = temp_dir.path().join("docker-compose.yaml");
    std::fs::write(&docker_compose_path, DOCKER_COMPOSE_YAML)
        .expect("Failed to write docker compose file");
    let cleanup = run_docker_compose(&docker_compose_path, "dozer-wait-for-connections-healthy");

    let mut config = tokio_postgres::Config::default();
    config
        .host(host)
        .port(port)
        .user(user)
        .password(password)
        .dbname(dbname);

    let connector = PostgresConnector::new(PostgresConfig {
        name: "postgres_connector_test".to_string(),
        config: config.clone(),
    });

    let config = postgres::Config::from(config);

    let client = config.connect(postgres::NoTls).unwrap();

    (
        client,
        PostgresConnectorTest {
            config,
            schema_name: Default::default(),
            table_name: Default::default(),
            schema: Default::default(),
            _cleanup: cleanup,
            _temp_dir: temp_dir,
        },
        connector,
    )
}

const DOCKER_COMPOSE_YAML: &str = r#"version: '2.4'
services:
  postgres:
    container_name: postgres
    image: debezium/postgres:13
    ports:
    - target: 5432
      published: 5432
    environment:
    - POSTGRES_DB=dozer-test
    - POSTGRES_USER=postgres
    - POSTGRES_PASSWORD=postgres
    - ALLOW_IP_RANGE=0.0.0.0/0
    healthcheck:
      test:
      - CMD-SHELL
      - pg_isready -U postgres -h 0.0.0.0 -d dozer-test
      interval: 5s
      timeout: 5s
      retries: 5
  dozer-wait-for-connections-healthy:
    image: alpine
    command: echo 'All connections are healthy'
    depends_on:
      postgres:
        condition: service_healthy
"#;
