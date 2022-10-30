mod events;
mod oauth;
mod store;
mod util;

use std::{thread, process::exit};

use chrono::{prelude::*, Duration};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Confirm, MultiSelect, Select, FuzzySelect};
use indicatif::{HumanDuration, MultiProgress, ProgressBar, ProgressStyle};
use clap::{Args, Parser, Subcommand};
use itertools::Itertools;
use regex::Regex;
use rusqlite::Result;
use tokio::sync::oneshot;

use events::{google, microsoft, GetResources};
use store::{Account, CalendarModel, Model};
use util::get_availability;

use crate::{store::Platform, util::Availability};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Start of search window in the form of MM/DD/YYYY (default now)
    #[arg(short, long, value_parser = parse_datetime)]
    start: Option<DateTime<Local>>,

    /// End of search window in the form of MM/DD/YYYY (default start + 7 days)
    #[arg(short, long, value_parser = parse_datetime)]
    end: Option<DateTime<Local>>,

    /// Duration for availability window (default 30 minutes)
    #[arg(short, long, value_parser = parse_duration)]
    duration: Option<Duration>,

    #[command(subcommand)]
    command: Option<Commands>,
}

fn parse_datetime(arg: &str) -> Result<DateTime<Local>, chrono::ParseError> {
    let dt_str: String = arg.to_string();
    let non_local_d = NaiveDate::parse_from_str(&dt_str, "%m/%d/%Y");
    let time = NaiveTime::from_hms(0, 0, 0);

    if non_local_d.is_ok() {
        let date = non_local_d.unwrap();
        let datetime = NaiveDateTime::new(date, time);
        Ok(Local.from_local_datetime(&datetime).unwrap())
    } else {
        Err(non_local_d.err().unwrap())
    }
}

fn parse_duration(arg: &str) -> anyhow::Result<Duration> {
    let duration_str: String = arg.to_string();

    let re = Regex::new(r"([0-9]*)(h|m)").unwrap();
    let caps = re.captures(&duration_str).unwrap();

    let group_1 = caps.get(1);
    let group_2 = caps.get(2);

    if group_1.is_none() || group_2.is_none() {
        Err(anyhow::anyhow!("Failed to parse duration."))
    } else {
        let num = group_1.unwrap().as_str().parse::<u32>()?;
        let unit = group_2.unwrap().as_str();

        match unit {
            "h" => Ok(Duration::hours(num.into())),
            "m" => Ok(Duration::minutes(num.into())),
            _ => Err(anyhow::anyhow!("Unsupported duration unit")),
        }
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let db = store::Store::new("./db.db3");

    match &cli.command {
        Some(Commands::Account(account_cmd)) => match &account_cmd.command {
            AccountCommands::Add(cmd) => {
                let selections = &[Platform::Google, Platform::Microsoft];

                let selection = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("Which platform would you like to add an account for?")
                    .items(&selections[..])
                    .interact()
                    .unwrap();

                match selections[selection] {
                    Platform::Microsoft => {
                        let (_, refresh_token) = microsoft::get_authorization_code().await;
                        store::store_token(&cmd.alias, &refresh_token)?;
                    }
                    Platform::Google => {
                        let (_, refresh_token) = google::get_authorization_code().await;
                        store::store_token(&cmd.alias, &refresh_token)?;
                    }
                }

                let account = Account {
                    name: cmd.alias.to_owned(),
                    platform: Some(selections[selection]),
                    id: None,
                };
                db.execute(Box::new(move |conn| account.insert(conn)))??;
                println!("Successfully added account.");
            }
            AccountCommands::Remove(cmd) => {
                if Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt(format!(
                        "Do you want to delete the account \"{}\"?",
                        cmd.alias
                    ))
                    .interact()
                    .unwrap()
                {
                    store::delete_token(&cmd.alias)?;
                    let account = Account {
                        name: cmd.alias.to_owned(),
                        id: None,
                        platform: None,
                    };
                    db.execute(Box::new(move |conn| account.delete(conn)))??;
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
                        println!(
                            "- {} on {}",
                            account.name.bold().blue(),
                            account.platform.unwrap()
                        );
                    }
                }
            }
        },
        Some(Commands::Calendar(_)) => {
            let accounts = db.execute(Box::new(|conn| Account::get(conn)))??;
            for account in accounts {
                let refresh_token = store::get_token(&account.name)?;

                let account_id = account.id.unwrap().to_owned();
                let mut calendars = match account.platform.unwrap() {
                    Platform::Microsoft => {
                        let (access_token, _) =
                            microsoft::refresh_access_token(refresh_token).await;
                        microsoft::MicrosoftGraph::get_calendars(access_token.to_owned()).await?
                    }
                    Platform::Google => {
                        let access_token = google::refresh_access_token(refresh_token).await;
                        google::GoogleAPI::get_calendars(access_token.to_owned()).await?
                    }
                };

                let prev_selected_calendars: Vec<String> = db
                    .execute(Box::new(move |conn| {
                        CalendarModel::get_all_selected(conn, &account_id.to_owned())
                    }))??
                    .into_iter()
                    .map(|c| c.calendar_id)
                    .collect();

                let mut defaults = vec![];
                for cal in calendars.iter() {
                    defaults.push(prev_selected_calendars.contains(&cal.id));
                }

                let calendar_names: Vec<String> =
                    calendars.iter().map(|cal| cal.name.to_owned()).collect();

                let selected_calendars_idx: Vec<usize> =
                    MultiSelect::with_theme(&ColorfulTheme::default())
                        .items(&calendar_names)
                        .defaults(&defaults)
                        .with_prompt(format!(
                            "Select the calendars you want to use for {}",
                            account.name
                        ))
                        .interact()?;

                for (i, mut cal) in calendars.iter_mut().enumerate() {
                    cal.selected = selected_calendars_idx.contains(&i);
                }

                db.execute(Box::new(move |conn| {
                    CalendarModel::delete_for_account(conn, &account_id)
                }))??;

                let insert_calendars: Vec<CalendarModel> = calendars
                    .into_iter()
                    .map(|c| CalendarModel {
                        account_id: account.id,
                        calendar_id: c.id,
                        calendar_name: c.name,
                        is_selected: c.selected,
                    })
                    .collect();

                db.execute(Box::new(|conn| {
                    CalendarModel::insert_many(conn, insert_calendars)
                }))??;
            }
        }
        _ => {
            let start_time = if let Some(start) = cli.start {
                start
            } else {
                Local::now()
            };

            let end_time = if let Some(end) = cli.end {
                end
            } else {
                start_time + Duration::days(7)
            };

            let duration = if let Some(d) = cli.duration {
                d
            } else {
                Duration::minutes(30)
            };

            println!(
                "Finding availability between {} and {}\n",
                format!("{}", start_time.format("%b %-d %Y")).bold().blue(),
                format!("{}", end_time.format("%b %-d %Y")).bold().blue()
            );

            let m = MultiProgress::new();
            let spinner_style = ProgressStyle::with_template(&"{spinner} {wide_msg}".blue())
            .unwrap()
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈✓");
            

            let pb = m.add(ProgressBar::new(1));
            pb.set_style(spinner_style.clone());
            pb.set_message("Retrieving events...");
            pb.enable_steady_tick(Duration::milliseconds(250).to_std().unwrap());

            let accounts = db.execute(Box::new(|conn| Account::get(conn)))??;
            let mut events = vec![];

            for account in accounts {
                let account_id = account.id.unwrap().to_owned();
                let selected_calendars: Vec<String> = db
                    .execute(Box::new(move |conn| {
                        CalendarModel::get_all_selected(conn, &account_id)
                    }))??
                    .into_iter()
                    .map(|c| c.calendar_id)
                    .collect();

                match account.platform.unwrap() {
                    Platform::Microsoft => {
                        let refresh_token = store::get_token(&account.name)?;
                        let (access_token, _) =
                            microsoft::refresh_access_token(refresh_token).await;

                        for cal_id in selected_calendars {
                            let mut account_events =
                                microsoft::MicrosoftGraph::get_calendar_events(
                                    access_token.to_owned(),
                                    cal_id.to_owned(),
                                    start_time,
                                    end_time,
                                )
                                .await?;
                            events.append(&mut account_events);
                        }
                    }
                    Platform::Google => {
                        let refresh_token = store::get_token(&account.name)?;
                        let access_token = google::refresh_access_token(refresh_token).await;
                        for cal_id in selected_calendars {
                            let mut account_events = google::GoogleAPI::get_calendar_events(
                                access_token.to_owned(),
                                cal_id.to_owned(),
                                start_time,
                                end_time,
                            )
                            .await?;
                            events.append(&mut account_events);
                        }
                    }
                }
            }

            pb.set_message("Computing availabilities...");

            let availability = get_availability(events, duration);
            let slots: Vec<Availability> = availability.into_iter().map(|(d, a)| a).flatten().collect();

            pb.finish_with_message("Computed availabilities.");
            
            let selection = MultiSelect::with_theme(&ColorfulTheme::default())
            .with_prompt("Select time window(s)")
            .items(&slots[..])
            .interact()
            .unwrap();
        }
    }

    Ok(())
}
