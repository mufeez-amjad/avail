use sea_orm_migration::prelude::*;
use super::m20220101_000001_add_account_table::Account;


#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Calendar::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Calendar::Id)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Calendar::AccountId).integer().not_null())
                    .col(ColumnDef::new(Calendar::IsSelected).boolean().not_null())
                    .primary_key(
                        Index::create()
                            .name("pk-account-calendar")
                            .col(Calendar::Id)
                            .col(Calendar::AccountId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-calendar-account_id")
                            .from(Calendar::Table, Calendar::AccountId)
                            .to(Account::Table, Account::Id),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Calendar::Table).to_owned())
            .await
    }
}

/// Learn more at https://docs.rs/sea-query#iden
#[derive(Iden)]
enum Calendar {
    Table,
    Id,
    AccountId,
    IsSelected,
}
