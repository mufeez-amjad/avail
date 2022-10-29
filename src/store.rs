use std::time::Duration;

use sea_orm::ConnectOptions;
use sea_orm::Database;
use sea_orm::DatabaseConnection;

pub struct Store {
    connection: DatabaseConnection, 
}

enum Platform {
    Microsoft,
    Google, 
}

impl Platform {
    fn as_str(&self) -> &'static str {
        match self {
            Platform::Microsoft => "Microsoft Outlook",
            Platform::Google => "Google Calendar"
        }
    }
}

use sea_orm::entity::prelude::*;

#[derive(Copy, Clone, Default, Debug, DeriveEntity)]
pub struct Entity;

impl Store {
    pub async fn new(path: &str) -> Self {
        let mut opt = ConnectOptions::new("./db.db3".to_owned());
        opt.connect_timeout(Duration::from_secs(8))
        .idle_timeout(Duration::from_secs(8))
        .max_lifetime(Duration::from_secs(8))
        .sqlx_logging(true);

        let db = Database::connect(opt).await.expect("failed to open database");
        Migrator::up(&db, None).await?;

        Self {
            connection: db,
        }
    }
}

/*
    Keyring functions
*/
const SERVICE_NAME: &str = "avail";

pub fn store_token(user: &str, token: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(SERVICE_NAME, user);
    entry.set_password(token)?;
    Ok(())
}

pub fn delete_token(user: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(SERVICE_NAME, &user);
    entry.delete_password()?;
    Ok(())
}
