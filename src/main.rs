mod events;
mod oauth;
mod store;
mod util;

use chrono::{prelude::*, Duration};
use clap::{Args, Parser, Subcommand};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, MultiSelect, Select};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use itertools::Itertools;
use regex::Regex;
use rusqlite::Result;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use crate::{events::Event, store::PLATFORMS, util::print_availability};
use events::{google, microsoft, GetResources};
use store::{Account, CalendarModel, Model, Platform};
use util::{get_availability, split_availability, Availability};

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

    /// Duration of search window, specify with <int>(w|d|h|m) (default 1w)
    #[arg(short, long, value_parser = parse_duration)]
    window: Option<Duration>,

    /// Duration for availability window (default 30m)
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

    let re = Regex::new(r"([0-9]*)(w|d|h|m)").unwrap();
    let caps = re.captures(&duration_str).unwrap();

    let group_1 = caps.get(1);
    let group_2 = caps.get(2);

    if group_1.is_none() || group_2.is_none() {
        Err(anyhow::anyhow!("Failed to parse duration."))
    } else {
        let num = group_1.unwrap().as_str().parse::<i64>()?;
        let unit = group_2.unwrap().as_str();

        match unit {
            "w" => Ok(Duration::weeks(num)),
            "d" => Ok(Duration::days(num)),
            "h" => Ok(Duration::hours(num)),
            "m" => Ok(Duration::minutes(num)),
            _ => Err(anyhow::anyhow!("Unsupported duration unit")),
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Manages OAuth accounts (Microsoft Outlook and Google Calendar)
    Account(AccountCmd),
    /// Allows specifying which calendars to use when querying, also refreshes calendar cache for added accounts
    Calendars(CalendarsCmd),
}

#[derive(Args)]
struct AccountCmd {
    #[command(subcommand)]
    command: AccountCommands,
}

#[derive(Args)]
struct CalendarsCmd {}

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
    /// The email of the account
    email: String,
}

#[derive(Args)]
struct AccountRemove {
    /// The email of the account to remove
    email: String,
}

#[derive(Args)]
struct AccountList {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let db = store::Store::new("./db.db3");

    match &cli.command {
        Some(Commands::Account(account_cmd)) => match &account_cmd.command {
            AccountCommands::Add(cmd) => {
                let selection = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("Which platform would you like to add an account for?")
                    .items(&PLATFORMS[..])
                    .interact()
                    .unwrap();

                let selected_platform = PLATFORMS[selection];

                match selected_platform {
                    Platform::Microsoft => {
                        let (_, refresh_token) = microsoft::get_authorization_code().await;
                        store::store_token(&cmd.email, &refresh_token)?;
                    }
                    Platform::Google => {
                        let (_, refresh_token) = google::get_authorization_code().await;
                        store::store_token(&cmd.email, &refresh_token)?;
                    }
                }

                let account = Account {
                    name: cmd.email.to_owned(),
                    platform: Some(selected_platform),
                    id: None,
                };
                db.execute(Box::new(move |conn| account.insert(conn)))??;
                println!("Successfully added account.");
            }
            AccountCommands::Remove(cmd) => {
                if Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt(format!(
                        "Do you want to delete the account \"{}\"?",
                        cmd.email
                    ))
                    .interact()
                    .unwrap()
                {
                    store::delete_token(&cmd.email)?;
                    let account = Account {
                        name: cmd.email.to_owned(),
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
        Some(Commands::Calendars(_)) => {
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

                let prev_unselected_calendars: Vec<String> = db
                    .execute(Box::new(move |conn| {
                        CalendarModel::get_all_selected(conn, &account_id.to_owned(), false)
                    }))??
                    .into_iter()
                    .map(|c| c.calendar_id)
                    .collect();

                let mut defaults = vec![];
                for cal in calendars.iter() {
                    defaults.push(!prev_unselected_calendars.contains(&cal.id));
                }

                let selected_calendars_idx: Vec<usize> =
                    MultiSelect::with_theme(&ColorfulTheme::default())
                        .items(&calendars)
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
                        can_edit: Some(c.can_edit),
                    })
                    .collect();

                db.execute(Box::new(|conn| {
                    CalendarModel::insert_many(conn, insert_calendars)
                }))??;
            }
        }
        _ => {
            let start_time = cli.start.unwrap_or(Local::now());

            let end_time = if let Some(end) = cli.end {
                end
            } else {
                let window = cli.window.unwrap_or(Duration::days(7));
                start_time + window
            };

            if end_time < start_time {
                return Err(anyhow::anyhow!("end time cannot be before start time"));
            }

            if cli.end.is_some() && cli.window.is_some() {
                println!(
                    "{}",
                    "Specified both end and window options, using end.\n"
                        .bold()
                        .red()
                );
            }

            let duration = cli.duration.unwrap_or(Duration::minutes(30));

            println!(
                "Finding availability between {} and {}\n",
                format!("{}", start_time.format("%b %-d %Y")).bold().blue(),
                format!("{}", end_time.format("%b %-d %Y")).bold().blue()
            );

            let m = MultiProgress::new();
            let spinner_style = ProgressStyle::with_template(&"{spinner} {wide_msg}".blue())
                .unwrap()
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈✔");

            let pb = m.add(ProgressBar::new(1));
            pb.set_style(spinner_style.clone());
            pb.set_message("Retrieving events...");
            pb.enable_steady_tick(Duration::milliseconds(250).to_std().unwrap());

            let accounts = db.execute(Box::new(|conn| Account::get(conn)))??;

            // Microsoft Graph has 4 concurrent requests limit
            let semaphore = Arc::new(Semaphore::new(4));
            let mut tasks: Vec<JoinHandle<anyhow::Result<Vec<Event>>>> = vec![];

            for account in accounts {
                let account_id = account.id.unwrap().to_owned();
                let selected_calendars: Vec<String> = db
                    .execute(Box::new(move |conn| {
                        CalendarModel::get_all_selected(conn, &account_id, true)
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
                            let token = access_token.clone();
                            let permit = semaphore
                                .clone()
                                .acquire_owned()
                                .await
                                .expect("unable to acquire permit"); // Acquire a permit
                            tasks.push(tokio::task::spawn(async move {
                                let res = microsoft::MicrosoftGraph::get_calendar_events(
                                    token,
                                    cal_id.to_owned(),
                                    start_time,
                                    end_time,
                                )
                                .await?;
                                drop(permit);
                                Ok(res)
                            }));
                        }
                    }
                    Platform::Google => {
                        let refresh_token = store::get_token(&account.name)?;
                        let access_token = google::refresh_access_token(refresh_token).await;

                        for cal_id in selected_calendars {
                            let token = access_token.clone();
                            tasks.push(tokio::task::spawn(async move {
                                let res = google::GoogleAPI::get_calendar_events(
                                    token,
                                    cal_id.to_owned(),
                                    start_time,
                                    end_time,
                                )
                                .await?;
                                Ok(res)
                            }));
                        }
                    }
                }
            }

            let results: Vec<Vec<Event>> = futures::future::join_all(tasks)
                .await
                .into_iter()
                .filter_map(|r| r.ok())
                .map(Result::unwrap)
                .collect();

            let events: Vec<Event> = results.into_iter().flatten().collect();

            pb.set_message("Computing availabilities...");

            let availability = get_availability(events, start_time, end_time, duration)?;
            let slots: Vec<Availability<Local>> = availability
                .into_iter()
                .map(|(_d, a)| a)
                .flatten()
                .collect();

            pb.finish_with_message("Computed availabilities.");

            // TODO: add multi-level multiselect
            // Right arrow goes into a time window (can select granular windows)
            // Left arrow goes back to root
            // Needs to work with paging
            let selection = MultiSelect::with_theme(&ColorfulTheme::default())
                .with_prompt("Select time window(s)")
                .items(&slots[..])
                .interact()
                .unwrap();

            let selected_slots: Vec<&Availability<Local>> = selection
                .into_iter()
                .map(|i| slots.get(i).unwrap())
                .collect();

            // (day, day_avails)
            let days = selected_slots.into_iter().group_by(|e| (e.start.date()));

            let mut iter = days.into_iter().peekable();

            let mut selected: Vec<Availability<Local>> = vec![];

            while iter.peek().is_some() {
                let i = iter.next();
                let (day, avails) = i.unwrap();

                let day_slots: Vec<&Availability<Local>> = avails.into_iter().map(|a| a).collect();
                let windows = split_availability(&day_slots, duration);

                let selection = MultiSelect::with_theme(&ColorfulTheme::default())
                    .with_prompt(format!(
                        "Select time window(s) for {}",
                        day.format("%b %d %Y")
                    ))
                    .items(&windows[..])
                    .interact()
                    .unwrap();

                let mut selected_windows: Vec<Availability<Local>> = selection
                    .into_iter()
                    .map(|i| windows.get(i).unwrap().clone())
                    .collect();
                selected.append(&mut selected_windows);
            }

            if selected.len() == 0 {
                return Ok(());
            }

            if !Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt("Do you want to add a hold event for your availabilities?")
                .interact()
                .unwrap()
            {
                print_availability(selected);
                return Ok(());
            }

            let event_title: String = Input::with_theme(&ColorfulTheme::default())
                .with_prompt("What's the name of your event?")
                .interact_text()?;

            let accounts = db.execute(Box::new(|conn| Account::get(conn)))??;

            let selection = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Which account would you like to create the events under?")
                .items(&accounts)
                .interact()
                .unwrap();

            match accounts[selection].platform.unwrap() {
                Platform::Microsoft => {
                    let account_id = accounts[selection].id.unwrap().to_owned();
                    let editable_calendars: Vec<String> = db
                        .execute(Box::new(move |conn| {
                            CalendarModel::get_all_editable(conn, &account_id, true)
                        }))??
                        .into_iter()
                        .map(|c| c.calendar_id)
                        .collect();

                    let selected_calendar = Select::with_theme(&ColorfulTheme::default())
                        .with_prompt("Which calendar would you like to add the events to?")
                        .items(&editable_calendars)
                        .interact()
                        .unwrap();

                    for avail in selected.iter() {
                        let refresh_token = store::get_token(&accounts[selection].name)?;
                        let (access_token, _) =
                            microsoft::refresh_access_token(refresh_token).await;
                        microsoft::MicrosoftGraph::create_event(
                            access_token.to_owned(),
                            editable_calendars[selected_calendar].to_owned(),
                            &event_title,
                            avail.start,
                            avail.end,
                        )
                        .await?;
                    }
                }
                Platform::Google => todo!(),
            }

            print_availability(selected);
        }
    }

    Ok(())
}
