mod oauth;
mod store;

use std::sync::Arc;

use dialoguer::{Select, MultiSelect, theme::ColorfulTheme, Confirm};
use chrono::{prelude::*, Duration};
use itertools::Itertools;
use colored::Colorize;

use serde::Deserialize;
use serde_json;

use clap::{Args, Parser, Subcommand};
use rusqlite::{Result};

use oauth::client::MicrosoftOauthClient;
use store::{Account, Model, CalendarModel};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Start of search window (default now)
    #[arg(short, long, value_parser = parse_datetime)]
    start: Option<DateTime<Local>>,

    /// End of search window (default start + 7 days)
    #[arg(short, long, value_parser = parse_datetime)]
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
    Add(AccountAdd),
    /// Removes an OAuth account
    Remove(AccountRemove),
    /// Lists all OAuth accounts
    List(AccountList),
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

#[derive(Args)]
struct AccountList {}

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
    value: Option<Vec<T>>,
    error: Option<GraphError>
}

// {"error":{"code":"InvalidAuthenticationToken","message":"CompactToken validation failed with reason code: 80049228.","innerError":{"date":"2022-10-23T22:59:31","request-id":"878f5037-d852-446f-8913-04fb1a0401d5","client-request-id":"878f5037-d852-446f-8913-04fb1a0401d5"}}}
#[derive(serde::Deserialize)]
struct GraphError {
    code: String,
    message: String,
}

async fn get_calendars(token: String) -> anyhow::Result<Vec<Calendar>> {
    let resp: GraphResponse<Calendar> = reqwest::Client::new()
        .get("https://graph.microsoft.com/v1.0/me/calendars")
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .send()
        .await
        .unwrap()
        .json()
        .await?;

    if let Some(err) = resp.error {
        return Err(anyhow::anyhow!("{}: {}", err.code, err.message));
    }

    Ok(resp.value.unwrap())
}

async fn get_calendar_events(token: String, calendar_id: String, start_time: DateTime<Local>, end_time: DateTime<Local>) -> anyhow::Result<Vec<Event>> {
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

    if let Some(err) = resp.error {
        return Err(anyhow::anyhow!("{}: {}", err.code, err.message));
    }

    Ok(resp.value.unwrap())
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

async fn get_authorization_code() -> (String, String) {
    let client = MicrosoftOauthClient::new("345ac594-c15f-4904-b9c5-49a29016a8d2", "", "", "");
    let token = client.get_authorization_code().await;
    token
}

async fn refresh_access_token(refresh_token: String) -> (String, String) {
    let client = MicrosoftOauthClient::new("345ac594-c15f-4904-b9c5-49a29016a8d2", "", "", "");
    let token = client.refresh_access_token(refresh_token).await;
    println!("refreshed token: {}, {}", token.0, token.1);
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
    let db = store::Store::new("./db.db3");
    
    match &cli.command {
        Some(Commands::Account(account_cmd)) => {
            match &account_cmd.command {
                AccountCommands::Add(cmd) => {
                    let selections = &[
                        "Outlook",
                        "GCal",
                    ];

                    let selection = Select::with_theme(&ColorfulTheme::default())
                        .with_prompt("Which platform would you like to add an account for?")
                        .items(&selections[..])
                        .interact()
                        .unwrap();

                    let (_, refresh_token) = get_authorization_code().await;
                    store::store_token(&cmd.alias, &refresh_token)?;
                    let account = Account {name: cmd.alias.to_owned(), platform: Some(selections[selection].to_owned()), id: None };
                    db.execute(Box::new(move |conn| account.insert(conn)));
                    println!("Successfully added account.");
                },
                AccountCommands::Remove(cmd) => {
                    if Confirm::with_theme(&ColorfulTheme::default())
                        .with_prompt(format!("Do you want to delete the account \"{}\"?", cmd.alias))
                        .interact()
                        .unwrap()
                    {
                        store::delete_token(&cmd.alias)?;
                        let account = Account {name: cmd.alias.to_owned(), id: None, platform: None };
                        db.execute(Box::new(move |conn| account.delete(conn)));
                        println!("Successfully removed account.");
                    }
                }
                AccountCommands::List(_) => {
                    let accounts = db.execute(Box::new(|conn| Account::get(conn)))??;

                    if accounts.len() == 0 {
                        println!("Configured accounts: None");
                    } else {
                        println!("Configured accounts:");
                        for account in accounts {
                            println!("- {} on {}", account.name.bold().blue(), account.platform.unwrap());
                        }
                    }

                },
            }
        },
        Some(Commands::Calendar(_)) => {
            let accounts = db.execute(Box::new(|conn| Account::get(conn)))??;
            for account in accounts {
                let entry = keyring::Entry::new("avail", &account.name);
                let refresh_token = entry.get_password()?;

                let (access_token, _) = refresh_access_token(refresh_token).await;

                let account_id = account.id.unwrap().to_owned();
                if account.platform.unwrap() == "Outlook" {
                    let mut calendars = get_calendars(access_token.to_owned()).await?;

                    let prev_selected_calendars: Vec<String> = db.execute(Box::new(move |conn| CalendarModel::get_all_selected(conn, &account_id.to_owned())))??
                    .into_iter()
                    .map(|c| c.calendar_id).collect();

                    let mut defaults = vec![];
                    for cal in calendars.iter() {
                        defaults.push(prev_selected_calendars.contains(&cal.id));
                    }

                    let calendar_names: Vec<String> = calendars.iter().map(|cal| cal.name.to_owned()).collect();

                    let selected_calendars_idx : Vec<usize> = MultiSelect::with_theme(&ColorfulTheme::default()) 
                    .items(&calendar_names)
                    .defaults(&defaults)
                    .with_prompt(format!("Select the calendars you want to use for {}", account.name))
                    .interact()?;

                    for (i, mut cal) in calendars.iter_mut().enumerate() {
                        cal.selected = selected_calendars_idx.contains(&i);
                    }

                    db.execute(Box::new(move |conn| CalendarModel::delete_for_account(conn, &account_id)))??;
                    let insert_calendars: Vec<CalendarModel> = calendars.into_iter()
                    .map(|c| CalendarModel { account_id: account.id, calendar_id: c.id, calendar_name: c.name, is_selected: c.selected })
                    .collect();

                    db.execute(Box::new(|conn| CalendarModel::insert_many(conn, insert_calendars)));
                }
            }
        },
        _ => {
            let accounts = db.execute(Box::new(|conn| Account::get(conn)))??;
            let mut events = vec![];

            for account in accounts {
                let account_id = account.id.unwrap().to_owned();
                let selected_calendars: Vec<String> = db.execute(Box::new(move |conn| CalendarModel::get_all_selected(conn, &account_id)))??
                    .into_iter()
                    .map(|c| c.calendar_id).collect();

                let entry = keyring::Entry::new("avail", &account.name);
                let refresh_token = entry.get_password()?;

                let (access_token, _) = refresh_access_token(refresh_token).await;

                if account.platform.unwrap() == "Outlook" {
                    for cal_id in selected_calendars {
                        let start_time = Local::now();
                        let end_time = start_time + Duration::days(7);
                        let mut account_events = get_calendar_events(access_token.to_owned(), cal_id.to_owned(), start_time, end_time).await?;
                        events.append(&mut account_events);
                    }
                }
            }

            get_availability(events);
        },
    }
    
    Ok(())
}

