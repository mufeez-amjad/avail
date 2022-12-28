use chrono::{prelude::*, Duration};
use clap::{Args, Parser, Subcommand};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use regex::Regex;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub(crate) struct Cli {
    /// Start of search window in the form of MM/DD/YYYY (default now)
    #[arg(long, value_parser = parse_datetime)]
    pub start: Option<DateTime<Local>>,

    /// End of search window in the form of MM/DD/YYYY (default start + 7 days)
    #[arg(long, value_parser = parse_datetime)]
    pub end: Option<DateTime<Local>>,

    /// Minimum time for availability in the form of <int>:<int>am/pm (default 9:00am)
    #[arg(long, value_parser = parse_naivetime)]
    pub min: Option<NaiveTime>,

    /// Maximum time for availability in the form of <int>:<int>am/pm (default 5:00pm)
    #[arg(long, value_parser = parse_naivetime)]
    pub max: Option<NaiveTime>,

    /// Duration of search window, specify with <int>(w|d|h|m) (default 1w)
    #[arg(short, long, value_parser = parse_duration)]
    pub window: Option<Duration>,

    /// Option to include weekends in availability search (default false)
    #[arg(long, default_value_t = false)]
    pub include_weekends: bool,

    /// Duration of availability window, specify with <int>(w|d|h|m) (default 30m)
    #[arg(short, long, value_parser = parse_duration)]
    pub duration: Option<Duration>,

    /// Create a hold event (default false)
    #[arg(short, long, default_value_t = false)]
    pub create_hold_event: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

fn parse_datetime(arg: &str) -> Result<DateTime<Local>, chrono::ParseError> {
    let dt_str: String = arg.to_string();
    let non_local_d = NaiveDate::parse_from_str(&dt_str, "%m/%d/%Y");
    let time = NaiveTime::from_hms(0, 0, 0);

    let datetime = NaiveDateTime::new(non_local_d?, time);
    Ok(Local.from_local_datetime(&datetime).unwrap())
}

fn parse_naivetime(arg: &str) -> Result<NaiveTime, chrono::ParseError> {
    let time_str: String = arg.to_string();
    NaiveTime::parse_from_str(&time_str, "%l:%M%P")
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
pub(crate) enum Commands {
    /// Manages OAuth accounts (Microsoft Outlook and Google Calendar)
    Accounts(AccountsCmd),
    /// Allows specifying which calendars to use when querying, refreshes calendar cache for added accounts
    Calendars(CalendarsCmd),
}

#[derive(Args)]
pub(crate) struct AccountsCmd {
    #[command(subcommand)]
    pub command: AccountCommands,
}

#[derive(Args)]
pub(crate) struct CalendarsCmd {}

#[derive(Subcommand)]
pub(crate) enum AccountCommands {
    /// Adds an OAuth account
    Add(AccountAdd),
    /// Removes an OAuth account
    Remove(AccountRemove),
    /// Lists all OAuth accounts
    List(AccountList),
}

#[derive(Args)]
pub(crate) struct AccountAdd {
    /// The email of the account to add
    pub email: String,
}

#[derive(Args)]
pub(crate) struct AccountRemove {
    /// The email of the account to remove
    pub email: String,
}

#[derive(Args)]
pub(crate) struct AccountList {}

pub(crate) struct ProgressIndicator {
    multi: MultiProgress,
    style: ProgressStyle,
}

impl Default for ProgressIndicator {
    fn default() -> Self {
        ProgressIndicator {
            multi: MultiProgress::new(),
            style: ProgressStyle::with_template("{spinner:.green} {wide_msg}")
                .unwrap()
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈✔"),
        }
    }
}

impl ProgressIndicator {
    pub fn add(&self, p: ProgressBar) -> ProgressBar {
        self.multi.add(p).with_style(self.style.clone())
    }

    pub fn clear(&self) {
        self.multi.clear().unwrap();
    }
}
