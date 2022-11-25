use rusqlite::Connection;

pub struct Store {
    connection: Connection,
}

#[derive(Clone, Copy)]
pub enum Platform {
    Microsoft,
    Google,
}

pub const PLATFORMS: [Platform; 2] = [Platform::Google, Platform::Microsoft];

impl Platform {
    fn as_str(&self) -> &'static str {
        match self {
            Platform::Microsoft => "Microsoft Outlook",
            Platform::Google => "Google Calendar",
        }
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

pub trait Model<T> {
    fn get(conn: &Connection) -> anyhow::Result<Vec<T>>;
    fn insert(&self, conn: &Connection) -> anyhow::Result<()>;
    fn delete(&self, conn: &Connection) -> anyhow::Result<()>;
}

pub struct Account {
    pub id: Option<u32>,
    pub name: String,
    pub platform: Option<Platform>,
}

impl std::fmt::Display for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "- {} on {}", self.name, self.platform.unwrap())
    }
}

impl Model<Account> for Account {
    fn get(conn: &Connection) -> anyhow::Result<Vec<Account>> {
        let mut stmt = conn.prepare("SELECT id, name, platform FROM accounts")?;
        let accounts: Vec<Account> = stmt
            .query_map([], |row| {
                let id: u32 = row.get(0)?;
                let name: String = row.get(1)?;
                let platform_str: String = row.get(2)?;

                let platform = if platform_str == Platform::Microsoft.as_str() {
                    Platform::Microsoft
                } else {
                    Platform::Google
                };

                Ok(Account {
                    id: Some(id),
                    name,
                    platform: Some(platform),
                })
            })?
            .filter_map(|s| s.ok())
            .collect();
        Ok(accounts)
    }

    fn insert(&self, conn: &Connection) -> anyhow::Result<()> {
        conn.execute(
            "INSERT INTO accounts (name, platform) VALUES (?1, ?2)",
            [
                self.name.to_owned(),
                self.platform.as_ref().unwrap().as_str().to_string(),
            ],
        )?;
        Ok(())
    }

    fn delete(&self, conn: &Connection) -> anyhow::Result<()> {
        conn.execute(
            "DELETE FROM accounts where name = ?",
            [self.name.to_owned()],
        )?;
        Ok(())
    }
}

pub struct CalendarModel {
    pub account_id: Option<u32>,
    pub id: String,
    pub name: String,
    pub query: bool,
    pub can_edit: Option<bool>,
    pub use_for_hold_events: bool,
}

impl CalendarModel {
    pub fn insert_many(conn: &Connection, calendars: Vec<CalendarModel>) -> anyhow::Result<()> {
        let mut stmt = conn.prepare("INSERT INTO calendars (account_id, id, name, query, can_edit, use_for_hold_events) VALUES (?, ?, ?, ?, ?, ?)")?;
        for cal in calendars.into_iter() {
            stmt.execute((
                cal.account_id.unwrap(),
                cal.id,
                cal.name,
                cal.query,
                cal.can_edit.unwrap_or(false),
                cal.use_for_hold_events,
            ))?;
        }
        Ok(())
    }

    pub fn get_all(conn: &Connection) -> anyhow::Result<Vec<CalendarModel>> {
        let mut stmt = conn.prepare(
            "SELECT account_id, id, name, query, can_edit, use_for_hold_events FROM calendars",
        )?;
        let prev_unselected_calendars: Vec<CalendarModel> = stmt
            .query_map((), |row| {
                let account_id: u32 = row.get(0)?;
                let id: String = row.get(1)?;
                let name: String = row.get(2)?;
                let query: bool = row.get(3)?;
                let can_edit: bool = row.get(4)?;
                let use_for_hold_events: bool = row.get(5)?;

                Ok(CalendarModel {
                    account_id: Some(account_id),
                    id,
                    name,
                    query,
                    can_edit: Some(can_edit),
                    use_for_hold_events,
                })
            })?
            .filter_map(|s| s.ok())
            .collect();

        Ok(prev_unselected_calendars)
    }

    pub fn get_all_selected(
        conn: &Connection,
        account_id: &u32,
        selected: bool,
    ) -> anyhow::Result<Vec<CalendarModel>> {
        let mut stmt =
            conn.prepare("SELECT id, name FROM calendars where query = ?1 and account_id = ?2")?;
        let prev_unselected_calendars: Vec<CalendarModel> = stmt
            .query_map((selected, account_id), |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                Ok(CalendarModel {
                    account_id: Some(*account_id),
                    id: id,
                    name: name,
                    query: selected,
                    can_edit: Some(false),
                    use_for_hold_events: false,
                })
            })?
            .filter_map(|s| s.ok())
            .collect();

        Ok(prev_unselected_calendars)
    }

    pub fn get_all_editable(
        conn: &Connection,
        account_id: &u32,
        can_edit: bool,
    ) -> anyhow::Result<Vec<CalendarModel>> {
        let mut stmt =
            conn.prepare("SELECT id, name FROM calendars where can_edit = ?1 and account_id = ?2")?;
        let prev_unselected_calendars: Vec<CalendarModel> = stmt
            .query_map((can_edit, account_id), |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                Ok(CalendarModel {
                    account_id: Some(*account_id),
                    id: id,
                    name: name,
                    query: false,
                    can_edit: Some(can_edit),
                    use_for_hold_events: false,
                })
            })?
            .filter_map(|s| s.ok())
            .collect();

        Ok(prev_unselected_calendars)
    }

    pub fn delete_for_account(conn: &Connection, account_id: &u32) -> anyhow::Result<()> {
        conn.execute("DELETE FROM calendars where account_id = ?", [account_id])?;
        Ok(())
    }
}

impl Store {
    pub fn new(path: &str) -> Self {
        let conn = Connection::open(path).expect("failed to open database");
        conn.execute("PRAGMA foreign_keys = true", ())
            .expect("failed to enable foreign keys");
        conn.execute(
            "
                CREATE TABLE IF NOT EXISTS accounts (
                    id          INTEGER PRIMARY KEY,
                    name        TEXT NOT NULL UNIQUE,
                    platform    TEXT NOT NULL
                );
            ",
            (),
        )
        .expect("failed to create accounts table");
        conn.execute(
            "
                CREATE TABLE IF NOT EXISTS calendars (
                    account_id  INTEGER NOT NULL,
                    id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    query BOOLEAN,
                    can_edit BOOLEAN,
                    use_for_hold_events BOOLEAN,
                    PRIMARY KEY (account_id, id),
                    FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
                );
            ",
            (),
        )
        .expect("failed to create calendars table");

        Self { connection: conn }
    }

    pub fn execute<T>(&self, func: Box<dyn FnOnce(&Connection) -> T>) -> anyhow::Result<T> {
        Ok(func(&self.connection))
    }
}

const SERVICE_NAME: &str = "avail";

pub fn store_token(user: &str, token: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(SERVICE_NAME, user);
    entry.set_password(token)?;
    Ok(())
}

pub fn get_token(user: &str) -> anyhow::Result<String> {
    let entry = keyring::Entry::new(SERVICE_NAME, user);
    let token = entry.get_password()?;
    Ok(token)
}

pub fn delete_token(user: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(SERVICE_NAME, user);
    entry.delete_password()?;
    Ok(())
}
