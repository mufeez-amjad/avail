use rusqlite::{Connection};

pub struct Store {
    connection: Connection, 
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

trait Model<T> {
    fn get(&self, conn: Connection) -> anyhow::Result<Vec<T>>;
    fn insert(&self, conn: Connection) -> anyhow::Result<()>;
    fn delete(&self, conn: Connection) -> anyhow::Result<()>;
}

pub struct Account {
    id: u32,
    name: String,
    platform: String,
}

impl Model<Account> for Account {
    fn get(&self, conn: Connection) -> anyhow::Result<Vec<Account>> {
        let mut stmt = conn.prepare("SELECT id, name, platform FROM accounts")?;
        let accounts: Vec<Account> = stmt.query_map([], |row| {
            let id: u32 = row.get(0)?;
            let name: String = row.get(1)?;
            let platform: String = row.get(2)?;
            Ok(Account {
                id,
                name,
                platform
            })
        })?.filter_map(|s| s.ok()).collect();
        Ok(accounts)
    }

    fn insert(&self, conn: Connection) -> anyhow::Result<()> {
        conn.execute(
            "INSERT INTO accounts (name, platform) VALUES (?1, ?2)",
            [self.name, self.platform],
        )?;
        Ok(())
    }

    fn delete(&self, conn: Connection) -> anyhow::Result<()> {
        conn.execute(
            "DELETE FROM accounts where name = ?",
            [self.name],
        )?;
        Ok(())
    }
}

impl Store {
    pub fn new(path: &str) -> Self {
        let conn = Connection::open(path).expect("failed to open database");
        conn.execute("PRAGMA foreign_keys = true", ()).expect("failed to enable foreign keys");
        conn.execute(
            "
                CREATE TABLE IF NOT EXISTS accounts (
                    id          INTEGER PRIMARY KEY,
                    name        TEXT NOT NULL,
                    platform    TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS calendars (
                    account_id  INTEGER NOT NULL,
                    calendar_id TEXT NOT NULL,
                    is_selected BOOLEAN,
                    PRIMARY KEY (account_id, calendar_id),
                    FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
                );
            ",
            (),
        ).expect("failed to create tables");

        Self {
            connection: conn,
        }
    }

    pub fn get<S, T: Model<S>>(&self, d: T) -> anyhow::Result<Vec<S>> {
        d.get(self.connection)
    }

    pub fn insert<S, T: Model<S>>(&self, d: T) -> anyhow::Result<()> {
        d.insert(self.connection)
    }

    pub fn delete<S, T: Model<S>>(&self, d: T) -> anyhow::Result<()> {
        d.delete(self.connection)
    }
}

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
