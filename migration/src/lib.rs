pub use sea_orm_migration::prelude::*;

mod m20220101_000001_add_account_table;
mod m20221027_011821_add_calendars_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_add_account_table::Migration),
            Box::new(m20221027_011821_add_calendars_table::Migration),
        ]
    }
}
