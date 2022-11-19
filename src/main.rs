mod commands;
mod events;
mod oauth;
mod store;
mod util;

use chrono::{prelude::*, Duration, DurationRound};
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

    /// Minimum time for availability in the form of <int>:<int>am/pm (default 9:00am)
    #[arg(long, value_parser = parse_naivetime)]
    min: Option<NaiveTime>,

    /// Maximum time for availability in the form of <int>:<int>am/pm (default 5:00pm)
    #[arg(long, value_parser = parse_naivetime)]
    max: Option<NaiveTime>,

    /// Duration of search window, specify with <int>(w|d|h|m) (default 1w)
    #[arg(short, long, value_parser = parse_duration)]
    window: Option<Duration>,

    /// Option to include weekends in availability search (default false)
    #[arg(long, default_value_t = false)]
    include_weekends: bool,

    /// Duration of availability window, specify with <int>(w|d|h|m) (default 1w)
    #[arg(short, long, value_parser = parse_duration)]
    duration: Option<Duration>,

    /// Create a hold event (default false)
    #[arg(long, default_value_t = false)]
    hold_event: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

fn parse_datetime(arg: &str) -> Result<DateTime<Local>, chrono::ParseError> {
    let dt_str: String = arg.to_string();
    let non_local_d = NaiveDate::parse_from_str(&dt_str, "%m/%d/%Y");
    let time = NaiveTime::from_hms(0, 0, 0);

    if let Ok(date) = non_local_d {
        let datetime = NaiveDateTime::new(date, time);
        Ok(Local.from_local_datetime(&datetime).unwrap())
    } else {
        Err(non_local_d.err().unwrap())
    }
}

fn parse_naivetime(arg: &str) -> Result<NaiveTime, chrono::ParseError> {
    let time_str: String = arg.to_string();
    let naivetime = NaiveTime::parse_from_str(&time_str, "%l:%M%P");
    naivetime
}

fn parse_duration(arg: &str) -> anyhow::Result<Duration> {
    let duration_str: String = arg.to_string();

    let re = Regex::new(r"([0-9]*)(w|d|h|m)").unwrap();
    let caps = re.captures(&duration_str).unwrap();

    let group_1 = caps.get(1);
    let group_2 = caps.get(2);

    if let (Some(match_1), Some(match_2)) = (group_1, group_2) {
        let num = match_1.as_str().parse::<i64>()?;
        let unit = match_2.as_str();

        match unit {
            "w" => Ok(Duration::weeks(num)),
            "d" => Ok(Duration::days(num)),
            "h" => Ok(Duration::hours(num)),
            "m" => Ok(Duration::minutes(num)),
            _ => Err(anyhow::anyhow!("Unsupported duration unit")),
        }
    } else {
        Err(anyhow::anyhow!("Failed to parse duration."))
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

    // Needed to restore cursor if program exits during dialoguer prompt.
    ctrlc::set_handler(move || {
        let term = console::Term::stdout();
        let _ = term.show_cursor();
    })?;

    match &cli.command {
        Some(Commands::Account(account_cmd)) => match &account_cmd.command {
            AccountCommands::Add(cmd) => commands::add_account(db, &cmd.email).await?,
            AccountCommands::Remove(cmd) => commands::remove_account(db, &cmd.email)?,
            AccountCommands::List(_) => commands::list_accounts(db)?,
        },
        Some(Commands::Calendars(_)) => commands::refresh_calendars(db).await?,
        _ => {
            let start_time = cli
                .start
                .unwrap_or_else(Local::now)
                .duration_round(Duration::minutes(30))?;

            let end_time = if let Some(end) = cli.end {
                end
            } else {
                let window = cli.window.unwrap_or_else(|| Duration::days(7));
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

            let min_time = cli.min.unwrap_or(NaiveTime::from_hms(9, 0, 0));
            let max_time = cli.max.unwrap_or(NaiveTime::from_hms(17, 0, 0));

            let duration = cli.duration.unwrap_or_else(|| Duration::minutes(30));

            println!(
                "Finding availability between {} and {}\n",
                format!("{}", start_time.format("%b %-d %Y")).bold().blue(),
                format!("{}", end_time.format("%b %-d %Y")).bold().blue()
            );

            commands::find_availability(
                db,
                start_time,
                end_time,
                min_time,
                max_time,
                duration,
                cli.hold_event,
                cli.include_weekends,
            )
            .await?
        }
    }

    Ok(())
}
