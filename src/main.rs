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
    Account(Account),
}

#[derive(Args)]
struct Account {
    #[command(subcommand)]
    command: AccountCommands,
}

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

    #[serde(default)]
    selected: bool,
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

    let calendar_names: Vec<String> = calendars.iter().map(|cal| cal.name.to_owned()).collect(); 
    
    let selected_calendars_idx : Vec<usize> = MultiSelect::new()
    .items(&calendar_names)
    .with_prompt("Select the calendars you want to use")
    .interact()?;

    Ok(selected_calendars_idx.iter().map(|idx| calendars[*idx].clone()).collect())
}

async fn get_calendar_events(token: String, calendar: Calendar, start_time: DateTime<Local>, end_time: DateTime<Local>) -> Result<Vec<Event>, Box<dyn std::error::Error + Send + Sync>> {
    let start_time_str = str::replace(&start_time.format("%+").to_string(), "+", "-");
    let end_time_str = str::replace(&end_time.format("%+").to_string(), "+", "-");

    let url = format!("https://graph.microsoft.com/v1.0/me/calendars/{}/calendarView?startDateTime={}&endDateTime={}", calendar.id, start_time_str, end_time_str);

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

async fn get_availability(token: String) -> Result<(), Box<dyn std::error::Error>> {
    let start_time = Local::now();
    let end_time = start_time + Duration::days(7);
    let min = NaiveTime::from_hms(9, 0, 0);
    let max = NaiveTime::from_hms(17, 0, 0);

    let calendars = get_calendars(token.to_owned()).await?;
    let semaphore = Arc::new(Semaphore::new(4));

    let mut tasks = vec![];
    for cal in calendars.into_iter() {
        let token = token.to_owned();
        let permit = semaphore.clone().acquire_owned().await.expect("unable to acquire permit"); // Acquire a permit
        tasks.push(tokio::task::spawn(async move {
            let res = get_calendar_events(token.to_owned(), cal.clone(), start_time, end_time).await;
            drop(permit);
            res
        }));
    }

    let results: Vec<Vec<Event>> = futures::future::join_all(tasks)
    .await
    .into_iter()
    .filter_map(|r| r.ok())
    .map(Result::unwrap)
    .collect();

    let events: Vec<Event> = results.into_iter().flatten().collect();
    
    let avails = get_free_time(events, start_time, end_time, min, max);

    let margin = 20;

    for (day, avail) in avails {
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
    Ok(())
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
        );",
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
                    }
                }
            }
        },
        _ => {
            // get_availability(token);
        },
    }
    
    Ok(())
}

