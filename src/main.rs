extern crate keyring;

mod oauth;

use std::{sync::Arc, str::FromStr};

use oauth::client::MicrosoftOauthClient;
use dialoguer::{Select, MultiSelect, theme::ColorfulTheme, Confirm};
use chrono::{prelude::*, Duration};
use itertools::Itertools;
use tokio::sync::Semaphore;

use serde::Deserialize;
use serde_json;

use clap::{Args, Parser, Subcommand};
use rusqlite::{Connection, Result};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    // #[arg(value_parser = parse_datetime)]
    // start: DateTime<Local>,

    #[arg(value_parser = parse_datetime)]
    end: Option<DateTime<Local>>,

    #[command(subcommand)]
    command: Option<Commands>,
}

fn parse_datetime(arg: &str) -> Result<DateTime<Local>, chrono::ParseError> {
    let dt_str: String = arg.to_string();
    let non_local_d = NaiveDate::parse_from_str(&dt_str, "%b %d %Y");
    let time = NaiveTime::from_hms(0, 0, 0);

    if non_local_d.is_ok() {
        let date = non_local_d.unwrap();
        let datetime = NaiveDateTime::new(date, time);
        Ok(Local.from_local_datetime(&datetime).unwrap())
    } else {
        Err(non_local_d.err().unwrap())
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Manages OAuth accounts (Microsoft Outlook and Google Calendar)
    Account(AccountCmd),
    Calendar(CalendarCmd),
}

#[derive(Args)]
struct AccountCmd {
    #[command(subcommand)]
    command: AccountCommands,
}

#[derive(Args)]
struct CalendarCmd {}

#[derive(Subcommand)]
enum AccountCommands {
    /// Adds an OAuth account
    add(AccountAdd),
    /// Removes an OAuth account
    remove(AccountRemove),
}

#[derive(Args)]
struct AccountAdd {
    /// The name of the account (should be unique)
    alias: String,
}

#[derive(Args)]
struct AccountRemove {
    /// The name of the account to remove
    alias: String,
}

#[derive(serde::Deserialize, Clone)]
struct Calendar {
    id: String,
    name: String,

    // TODO: use this field for default selection
    #[serde(default)]
    selected: bool,
}

#[derive(serde::Deserialize, Clone)]
struct Event {
    id: String,
    #[serde(rename(deserialize = "subject"))]
    name: String,

    #[serde(deserialize_with = "deserialize_json_time")]
    start: DateTime<Local>,
    #[serde(deserialize_with = "deserialize_json_time")]
    end: DateTime<Local>,
}

struct Availability {
    start: DateTime<Local>,
    end: DateTime<Local>,
}

fn deserialize_json_time<'de, D>(deserializer: D) -> Result<DateTime<Local>, D::Error>
where
	D: serde::de::Deserializer<'de>,
{
    let json: serde_json::value::Value = serde_json::value::Value::deserialize(deserializer)?;
    let time_str = json.get("dateTime").expect("datetime").as_str().unwrap();
    let tz_str = json.get("timeZone").expect("timeZone").as_str().unwrap();

    // 2022-10-22T20:30:00.0000000
    let naive_time = NaiveDateTime::parse_from_str(time_str, "%Y-%m-%dT%H:%M:%S.%f").unwrap();
    
    Local.timestamp(0, 0).offset();

    let datetime = match tz_str {
        "UTC" => DateTime::<Utc>::from_utc(naive_time, Utc),
        _ =>  DateTime::<Utc>::from_utc(naive_time, Utc),
    };
    Ok(datetime.with_timezone(&Local))
}

impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}, {}, {}, {}", self.id, self.name, self.start, self.end)
    }
}

#[derive(serde::Deserialize)]
struct GraphResponse<T> {
    value: Vec<T>
}

async fn get_calendars(token: String) -> Result<Vec<Calendar>, Box<dyn std::error::Error>> {
    let resp: GraphResponse<Calendar> = reqwest::Client::new()
        .get("https://graph.microsoft.com/v1.0/me/calendars")
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .send()
        .await
        .unwrap()
        .json()
        .await?;

    let calendars = resp.value;
    Ok(calendars)
}

async fn get_calendar_events(token: String, calendar_id: String, start_time: DateTime<Local>, end_time: DateTime<Local>) -> Result<Vec<Event>, Box<dyn std::error::Error>> {
    let start_time_str = str::replace(&start_time.format("%+").to_string(), "+", "-");
    let end_time_str = str::replace(&end_time.format("%+").to_string(), "+", "-");

    let url = format!("https://graph.microsoft.com/v1.0/me/calendars/{}/calendarView?startDateTime={}&endDateTime={}", calendar_id, start_time_str, end_time_str);

    let resp: GraphResponse<Event> = reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .send()
        .await
        .unwrap()
        .json()
        .await?;

    Ok(resp.value)
}

fn get_free_time(mut events: Vec<Event>, start: DateTime<Local>, end: DateTime<Local>, min: NaiveTime, max: NaiveTime) -> Vec<(Date<Local>, Vec<Availability>)> {
    let mut avail: Vec<(Date<Local>, Vec<Availability>)> = vec![];
    let duration = 30;

    events.sort_by_key(|e| e.start);
    
    let days = events.into_iter().group_by(|e| (e.start.date()));

    let mut iter = days.into_iter();

    let mut dt = start;
    while dt <= end {
        let day = iter.next();

        if let Some((date, events)) = day {
            // Add days that are entirely free
            while dt.date() < date {
                // Whole day
                let end = dt + Duration::days(1);
                avail.push((dt.date(), vec![Availability { start: dt, end }]));

                dt += Duration::days(1);
            }
            
            // events is guaranteed to be non-empty because of the GroupBy
            
            // Check for availabilities within the day

            let mut day_avail = vec![];
            let mut curr_time = chrono::NaiveTime::from_hms(9, 0, 0);

            for event in events {
                let start = event.start;
                let end = event.end;

                // Have time before event
                if curr_time < start.time() {
                    // Meets requirement of minimum duration
                    if start.time() - curr_time >= Duration::minutes(duration) {
                        let start_time = DateTime::from_local(NaiveDateTime::new(start.date_naive(), curr_time), *Local.timestamp(0, 0).offset());
                        let end_time = start;
                        day_avail.push(Availability { start: start_time, end: end_time });
                    }

                    // Not available until end of this event
                    curr_time = end.time()
                } else {
                    curr_time = std::cmp::max(end.time(), curr_time);
                }
            }

            if curr_time < max {
                let start_time = DateTime::from_local(NaiveDateTime::new(start.date_naive(), curr_time), *Local.timestamp(0, 0).offset());
                let end_time = DateTime::from_local(NaiveDateTime::new(start.date_naive(), max), *Local.timestamp(0, 0).offset());
                day_avail.push(Availability { start: start_time, end: end_time });
            }

            avail.push((dt.date(), day_avail));

            // 12AM next day
            dt = (dt + Duration::days(1)).date().and_hms(0, 0, 0);
        } else {
            // Add days that are entirely free
            while dt <= end {
                // Whole day
                let end = dt + Duration::days(1);
                avail.push((dt.date(), vec![Availability { start: dt, end }]));

                dt += Duration::days(1);
            }
        }
    }

    avail
}

async fn get_authorization_code() -> String {
    let client = MicrosoftOauthClient::new("345ac594-c15f-4904-b9c5-49a29016a8d2", "", "", "");
    let token = client.get_authorization_code().await.secret().to_owned();
    token
}

fn get_availability(events: Vec<Event>) -> Vec<(Date<Local>, Vec<Availability>)> {
    let start_time = Local::now();
    let end_time = start_time + Duration::days(7);
    let min = NaiveTime::from_hms(9, 0, 0);
    let max = NaiveTime::from_hms(17, 0, 0);

    let avails = get_free_time(events, start_time, end_time, min, max);

    let margin = 20;

    for (day, avail) in avails.iter() {
        println!("{:-^margin$}", day.format("%a %B %e"));
        for a in avail {
            if a.end - a.start == Duration::days(1) {
                println!("Whole day!");
            } else {
                let duration = a.end  - a.start;
                print!("{} to {}", a.start.format("%H:%M"), a.end.format("%H:%M"));

                print!(" (");
                if duration.num_hours() >= 1 {
                    print!("{}h", duration.num_hours());
                    if duration.num_minutes() >= 1 {
                        print!("{}m", duration.num_minutes() % 60);
                    }
                } else if duration.num_minutes() >= 1 {
                    print!("{}m", duration.num_minutes())
                }
                println!(")");
            }
        }
        println!()
    }
    avails
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let path = "./db.db3";
    let db = Connection::open(path)?;

    db.execute(
        "CREATE TABLE IF NOT EXISTS accounts (
            id          INTEGER PRIMARY KEY,
            name        TEXT NOT NULL,
            platform    TEXT NOT NULL
        )",
        (),
    )?;

    db.execute(
        "CREATE TABLE IF NOT EXISTS calendars (
            account_id  INTEGER NOT NULL,
            calendar_id TEXT NOT NULL,
            is_selected BOOLEAN,
            FOREIGN KEY(account_id) REFERENCES accounts(id),
            PRIMARY KEY (account_id, calendar_id)
        )",
        (),
    )?;
    
    match &cli.command {
        Some(Commands::Account(account_cmd)) => {
            match &account_cmd.command {
                AccountCommands::add(cmd) => {
                    let selections = &[
                        "Outlook",
                        "GCal",
                    ];

                    let selection = Select::with_theme(&ColorfulTheme::default())
                        .with_prompt("Which platform would you like to add an account for?")
                        .items(&selections[..])
                        .interact()
                        .unwrap();

                    let token = get_authorization_code().await;
                    let entry = keyring::Entry::new("avail", &cmd.alias);
                    entry.set_password(&token)?;

                    db.execute(
                        "INSERT INTO accounts (name, platform) VALUES (?1, ?2)",
                        [cmd.alias.to_string(), selections[selection].to_string()],
                    )?;
                },
                AccountCommands::remove(cmd) => {
                    if Confirm::with_theme(&ColorfulTheme::default())
                        .with_prompt(format!("Do you want to delete the account \"{}\"?", cmd.alias))
                        .interact()
                        .unwrap()
                    {
                        let service = "avail";
                        let entry = keyring::Entry::new(&service, &cmd.alias);
                        entry.delete_password()?;
                        println!("Account removed.");

                        //TODO: remove from database.
                    }
                }
            }
        },
        Some(Commands::Calendar(_)) => {
            let mut stmt = db.prepare("SELECT id, name, platform FROM accounts")?;
            let accounts = stmt.query_map([], |row| {
                let id: u32 = row.get(0)?;
                let name: String = row.get(1)?;
                let platform: String = row.get(2)?;
                Ok((id, name, platform))
            })?;

            for account in accounts {
                match account {
                    // Destructure the second and third elements
                    Ok((id, name, platform)) => {
                        let entry = keyring::Entry::new("avail", &name);
                        let token = entry.get_password()?;

                        if platform == "Outlook" {
                            let mut calendars = get_calendars(token.to_owned()).await?;

                            // Pull from database.
                            let mut stmt = db.prepare("SELECT calendar_id FROM calendars where is_selected = true")?;
                            let prev_selected_calendars: Vec<String> = stmt.query_map([], |row| {
                                let id: String = row.get(0)?;
                                Ok(id)
                            })?.filter_map(|s| s.ok()).collect();

                            let mut defaults = vec![];
                            for cal in calendars.iter() {
                                defaults.push(prev_selected_calendars.contains(&cal.id));
                            }

                            let calendar_names: Vec<String> = calendars.iter().map(|cal| cal.name.to_owned()).collect();
    
                            let selected_calendars_idx : Vec<usize> = MultiSelect::new()
                            .items(&calendar_names)
                            .defaults(&defaults)
                            .with_prompt("Select the calendars you want to use")
                            .interact()?;

                            for (i, mut cal) in calendars.iter_mut().enumerate() {
                                cal.selected = selected_calendars_idx.contains(&i);
                            }

                            db.execute("DELETE FROM calendars where account_id = ?", [id])?;

                            let mut stmt = db.prepare("INSERT INTO calendars (account_id, calendar_id, is_selected) VALUES (?, ?, ?)")?;
                            for cal in calendars.into_iter() {
                                stmt.execute((id, cal.id, cal.selected))?;
                            }
                        }
                    },
                    _ => println!("It doesn't matter what they are")
                }
            }
        },
        _ => {
            let mut stmt = db.prepare("SELECT id, name, platform FROM accounts")?;
            let accounts = stmt.query_map([], |row| {
                let id: u32 = row.get(0)?;
                let name: String = row.get(1)?;
                let platform: String = row.get(2)?;
                Ok((id, name, platform))
            })?;

            let mut events = vec![];

            for account in accounts {
                match account {
                    Ok((id, name, platform)) => {
                        let mut stmt = db.prepare("SELECT calendar_id FROM calendars where is_selected = true and account_id = ?")?;
                        
                        let selected_calendars: Vec<String> = stmt.query_map([id], |row| {
                            let calendar_id: String = row.get(0)?;
                            Ok(calendar_id)
                        })?.filter_map(|s| s.ok()).collect();

                        let entry = keyring::Entry::new("avail", &name);
                        let token = entry.get_password()?;

                        if platform == "Outlook" {
                            for cal_id in selected_calendars {
                                let start_time = Local::now();
                                let end_time = start_time + Duration::days(7);
                                let mut account_events = get_calendar_events(token.to_owned(), cal_id.to_owned(), start_time, end_time).await?;
                                events.append(&mut account_events);
                            }
                        }
                    },
                    _ => println!("It doesn't matter what they are")
                }
            }

            get_availability(events);
        },
    }
    
    Ok(())
}

