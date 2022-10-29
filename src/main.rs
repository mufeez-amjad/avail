mod events;
mod oauth;
mod store;
mod util;

use chrono::{prelude::*, Duration};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Confirm, MultiSelect, Select};

use clap::{Args, Parser, Subcommand};
use rusqlite::Result;

use events::{google, microsoft, GetResources};
use store::{Account, CalendarModel, Model};
use util::get_availability;

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

    match &cli.command {
        Some(Commands::Account(account_cmd)) => match &account_cmd.command {
            AccountCommands::Add(cmd) => {
                let selections = &["Outlook", "GCal"];

                let selection = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("Which platform would you like to add an account for?")
                    .items(&selections[..])
                    .interact()
                    .unwrap();

                if selection == 0 {
                    let (_, refresh_token) = microsoft::get_authorization_code().await;
                    store::store_token(&cmd.alias, &refresh_token)?;
                } else if selection == 1 {
                    let (_, refresh_token) = google::get_authorization_code().await;
                    store::store_token(&cmd.alias, &refresh_token)?;
                }

                let account = Account {
                    name: cmd.alias.to_owned(),
                    platform: Some(selections[selection].to_owned()),
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
                let mut calendars = if account.platform.unwrap() == "Outlook" {
                    let (access_token, _) = microsoft::refresh_access_token(refresh_token).await;
                    microsoft::MicrosoftGraph::get_calendars(access_token.to_owned()).await?
                } else {
                    let access_token = google::refresh_access_token(refresh_token).await;
                    google::GoogleAPI::get_calendars(access_token.to_owned()).await?
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

                if account.platform.unwrap() == "Outlook" {
                    let refresh_token = store::get_token(&account.name)?;
                    let (access_token, _) = microsoft::refresh_access_token(refresh_token).await;

                    for cal_id in selected_calendars {
                        let mut account_events = microsoft::MicrosoftGraph::get_calendar_events(
                            access_token.to_owned(),
                            cal_id.to_owned(),
                            start_time,
                            end_time,
                        )
                        .await?;
                        events.append(&mut account_events);
                    }
                } else {
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

            let availability = get_availability(events);
        }
    }

    Ok(())
}
