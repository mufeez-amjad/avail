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
    pub calendar_id: String,
    pub calendar_name: String,
    pub is_selected: bool,
    pub can_edit: Option<bool>,
}

impl CalendarModel {
    pub fn insert_many(conn: &Connection, calendars: Vec<CalendarModel>) -> anyhow::Result<()> {
        let mut stmt = conn.prepare("INSERT INTO calendars (account_id, calendar_id, calendar_name, is_selected, can_edit) VALUES (?, ?, ?, ?, ?)")?;
        for cal in calendars.into_iter() {
            stmt.execute((
                cal.account_id.unwrap(),
                cal.calendar_id,
                cal.calendar_name,
                cal.is_selected,
                cal.can_edit.unwrap_or(false),
            ))?;
        }
        Ok(())
    }

    pub fn get_all_selected(
        conn: &Connection,
        account_id: &u32,
        selected: bool,
    ) -> anyhow::Result<Vec<CalendarModel>> {
        let mut stmt = conn.prepare("SELECT calendar_id, calendar_name FROM calendars where is_selected = ?1 and account_id = ?2")?;
        let prev_unselected_calendars: Vec<CalendarModel> = stmt
            .query_map((selected, account_id), |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                Ok(CalendarModel {
                    account_id: Some(*account_id),
                    calendar_id: id,
                    calendar_name: name,
                    is_selected: selected,
                    can_edit: Some(false),
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
        let mut stmt = conn.prepare("SELECT calendar_id, calendar_name FROM calendars where can_edit = ?1 and account_id = ?2")?;
        let prev_unselected_calendars: Vec<CalendarModel> = stmt
            .query_map((can_edit, account_id), |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                Ok(CalendarModel {
                    account_id: Some(*account_id),
                    calendar_id: id,
                    calendar_name: name,
                    is_selected: false,
                    can_edit: Some(can_edit),
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
                    calendar_id TEXT NOT NULL,
                    calendar_name TEXT NOT NULL,
                    is_selected BOOLEAN,
                    can_edit BOOLEAN,
                    PRIMARY KEY (account_id, calendar_id),
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
