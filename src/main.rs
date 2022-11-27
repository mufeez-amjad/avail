mod cli;
mod commands;
mod datetime;
mod events;
mod oauth;
mod store;
mod util;

use chrono::{prelude::*, Duration};
use clap::Parser;
use colored::Colorize;

use crate::{cli::ProgressIndicator, datetime::finder::AvailabilityFinder};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();

    let db = store::Store::new(&format!("{}/db.db3", util::get_avail_directory()?));

    // Needed to restore cursor if program exits during dialoguer prompt.
    ctrlc::set_handler(move || {
        let term = console::Term::stdout();
        let _ = term.show_cursor();
    })?;

    match &cli.command {
        Some(cli::Commands::Accounts(account_cmd)) => match &account_cmd.command {
            cli::AccountCommands::Add(cmd) => commands::add_account(db, &cmd.email).await?,
            cli::AccountCommands::Remove(cmd) => commands::remove_account(db, &cmd.email)?,
            cli::AccountCommands::List(_) => commands::list_accounts(db)?,
        },
        Some(cli::Commands::Calendars(_)) => commands::refresh_calendars(db).await?,
        _ => {
            let start_time = cli
                .start
                .unwrap_or_else(|| datetime::finder::Round::ceil(&Local::now()));

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

            let min_time = cli.min.unwrap_or_else(|| NaiveTime::from_hms(9, 0, 0));
            let max_time = cli.max.unwrap_or_else(|| NaiveTime::from_hms(17, 0, 0));

            let duration = cli.duration.unwrap_or_else(|| Duration::minutes(30));

            println!(
                "Finding availability between {} and {}\n",
                format!("{}", start_time.format("%b %-d %Y")).bold().blue(),
                format!("{}", end_time.format("%b %-d %Y")).bold().blue()
            );

            let finder = AvailabilityFinder {
                start: start_time,
                end: end_time,
                min: min_time,
                max: max_time,
                duration,
                include_weekends: cli.include_weekends,
            };

            let progress = ProgressIndicator::default();

            let merged = commands::find_availability(&db, finder, &progress).await?;

            if !cli.hold_event {
                commands::print_and_copy_availability(&merged);
                return Ok(());
            }

            commands::create_hold_events(db, &merged, progress).await?;
            commands::print_and_copy_availability(&merged);
        }
    }

    Ok(())
}
