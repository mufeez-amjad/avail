mod commands;
mod events;
mod oauth;
mod store;
mod util;

use chrono::{prelude::*, Duration};
use clap::{Args, Parser, Subcommand};
use colored::Colorize;
use regex::Regex;

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

    /// Duration of availability window, specify with <int>(w|d|h|m) (default 1w)
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
    /// Allows specifying which calendars to use when querying, refreshes calendar cache for added accounts
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
            AccountCommands::Add(cmd) => commands::add_account(db, &cmd.email).await?,
            AccountCommands::Remove(cmd) => commands::remove_account(db, &cmd.email)?,
            AccountCommands::List(_) => commands::list_accounts(db)?,
        },
        Some(Commands::Calendars(_)) => commands::refresh_calendars(db).await?,
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

            commands::find_availability(db, start_time, end_time, duration).await?
        }
    }

    Ok(())
}
