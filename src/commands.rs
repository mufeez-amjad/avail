use std::sync::Arc;

use chrono::{prelude::*, Duration};
use colored::Colorize;
use copypasta::{ClipboardContext, ClipboardProvider};
use dialoguer::{theme::ColorfulTheme, Confirm, Input, MultiSelect, Select};
use indicatif::ProgressBar;
use itertools::Itertools;
use tokio::{sync::Semaphore, task::JoinHandle};

use crate::cli::ProgressIndicator;
use crate::datetime::{
    availability::{
        format_availability, merge_overlapping_avails, split_availability, Availability,
    },
    finder::AvailabilityFinder,
};
use crate::events::{google, microsoft, Calendar, Event, GetResources};
use crate::store::{AccountModel, CalendarModel, Platform, Store, PLATFORMS};
use crate::util::AvailConfig;

pub async fn add_account(
    db: Store,
    email: &str,
    cfg: &AvailConfig,
    shutdown_receiver: tokio::sync::oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Which platform would you like to add an account for?")
        .items(&PLATFORMS[..])
        .default(0)
        .interact()
        .unwrap();

    let selected_platform = PLATFORMS[selection];

    let accounts = db.execute(Box::new(|conn| AccountModel::get(conn)))??;
    if accounts
        .iter()
        .any(|a| a.name == email && a.platform.unwrap() == selected_platform)
    {
        return Err(anyhow::anyhow!("Account already exists with that email"));
    }

    match selected_platform {
        Platform::Microsoft => {
            let (_, refresh_token) = microsoft::get_authorization_code(
                &cfg.microsoft.to_owned().unwrap_or_default(),
                shutdown_receiver,
            )
            .await?;
            crate::store::store_token(email, &refresh_token)?;
        }
        Platform::Google => {
            let (_, refresh_token) = google::get_authorization_code(
                &cfg.google.to_owned().unwrap_or_default(),
                shutdown_receiver,
            )
            .await?;
            crate::store::store_token(email, &refresh_token)?;
        }
        _ => return Err(anyhow::anyhow!("Unsupported platform")),
    }

    let account = AccountModel {
        name: email.to_owned(),
        platform: Some(selected_platform),
        id: None,
    };
    db.execute(Box::new(move |conn| account.insert(conn)))??;
    println!("\nSuccessfully added account.");
    println!(
        "Run the \"{}\" command to update the calendars cache with this account's calendars.",
        "calendars".bold()
    );

    Ok(())
}

pub fn remove_account(db: Store, email: &str) -> anyhow::Result<()> {
    if Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Do you want to delete the account \"{}\"?", email))
        .interact()
        .unwrap()
    {
        crate::store::delete_token(email)?;
        let account = AccountModel {
            name: email.to_owned(),
            id: None,
            platform: None,
        };
        db.execute(Box::new(move |conn| account.delete(conn)))??;
        println!("Successfully removed account.");
    }

    Ok(())
}

pub fn list_accounts(db: Store) -> anyhow::Result<()> {
    let accounts = db.execute(Box::new(|conn| AccountModel::get(conn)))??;

    if accounts.is_empty() {
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

    Ok(())
}

pub async fn refresh_calendars(db: Store, cfg: &AvailConfig) -> anyhow::Result<()> {
    let accounts = db.execute(Box::new(|conn| AccountModel::get(conn)))??;

    if accounts.is_empty() {
        return Err(anyhow::anyhow!(format!(
            "You must link accounts using the \"{}\" command before fetching calendars.",
            "accounts add".italic().bold()
        )));
    }

    for account in accounts {
        let refresh_token = crate::store::get_token(&account.name)?;

        let account_id = account.id.unwrap().to_owned();
        let mut calendars = match account.platform.unwrap() {
            Platform::Microsoft => {
                let access_token = microsoft::refresh_access_token(
                    &cfg.microsoft.to_owned().unwrap_or_default(),
                    &refresh_token,
                )
                .await?;
                microsoft::MicrosoftGraph::get_calendars(&access_token).await?
            }
            Platform::Google => {
                let access_token = google::refresh_access_token(
                    &cfg.google.to_owned().unwrap_or_default(),
                    &refresh_token,
                )
                .await?;
                google::GoogleAPI::get_calendars(&access_token).await?
            }
            _ => return Err(anyhow::anyhow!("Unsupported platform")),
        };

        let mut prev_unselected_calendars = db
            .execute(Box::new(move |conn| {
                CalendarModel::get_all_selected(conn, &account_id.to_owned(), false)
            }))??
            .into_iter()
            .map(|c| c.id);

        let mut defaults = vec![];
        for cal in calendars.iter() {
            defaults.push(!prev_unselected_calendars.contains(&cal.id));
        }

        let selected_calendars_idx: Vec<usize> = MultiSelect::with_theme(&ColorfulTheme::default())
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
                id: c.id,
                name: c.name,
                selected: c.selected,
            })
            .collect();

        db.execute(Box::new(|conn| {
            CalendarModel::insert_many(conn, insert_calendars)
        }))??;
    }

    let mut all_calendars: Vec<Calendar> = db
        .execute(Box::new(move |conn| CalendarModel::get_all(conn)))??
        .into_iter()
        .map(|c| Calendar {
            account_id: c.account_id.unwrap(),
            id: c.id,
            name: c.name,
            selected: false,
        })
        .collect_vec();

    let previous_selected = db.execute(Box::new(move |conn| {
        CalendarModel::get_hold_event_calendar(conn)
    }))??;

    let previous_selected_idx: usize = if let Some((_, cal)) = previous_selected {
        let e = all_calendars.iter().enumerate().find(|e| e.1.id == cal.id);
        e.unwrap().0
    } else {
        0
    };

    let selected_calendar_idx: usize = Select::with_theme(&ColorfulTheme::default())
        .items(&all_calendars)
        .default(previous_selected_idx)
        .with_prompt("Which calendar would you like to use to create hold events?")
        .interact()?;

    let mut selected_calendar = all_calendars.get_mut(selected_calendar_idx).unwrap();
    selected_calendar.selected = true;

    let update_calendar = CalendarModel {
        account_id: Some(selected_calendar.account_id),
        id: selected_calendar.id.to_owned(),
        name: selected_calendar.name.to_owned(),
        selected: true,
    };

    db.execute(Box::new(move |conn| {
        CalendarModel::update_hold_event_calendar(conn, update_calendar)
    }))??;

    Ok(())
}

pub fn print_and_copy_availability(avails: &[Availability<Local>]) {
    let s = format_availability(avails);
    let mut ctx = ClipboardContext::new().unwrap();
    print!("{}", s);
    if ctx.set_contents(s).is_ok() {
        println!("\nCopied to clipboard.")
    }
}

pub(crate) async fn find_availability(
    db: &Store,
    cfg: &AvailConfig,
    finder: AvailabilityFinder,
    m: &ProgressIndicator,
) -> anyhow::Result<Vec<Availability<Local>>> {
    let accounts = db.execute(Box::new(|conn| AccountModel::get(conn)))??;

    if accounts.is_empty() {
        return Err(anyhow::anyhow!(format!(
            "You must link accounts using the \"{}\" command and configure calendars using \"{}\" command before you are able to find availabilities.",
            "accounts add".bold().italic(),
            "calendars".bold().italic()
        )));
    }

    println!(
        "Finding availability between {} and {}\n",
        format!("{}", finder.start.format("%b %-d %Y"))
            .bold()
            .blue(),
        format!("{}", finder.end.format("%b %-d %Y")).bold().blue()
    );

    let pb = m.add(ProgressBar::new(1));
    pb.set_message("Retrieving events...");
    pb.enable_steady_tick(Duration::milliseconds(250).to_std().unwrap());

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
            .map(|c| c.id)
            .collect();

        match account.platform.unwrap() {
            Platform::Microsoft => {
                let refresh_token = crate::store::get_token(&account.name)?;
                let access_token = microsoft::refresh_access_token(
                    &cfg.microsoft.to_owned().unwrap_or_default(),
                    &refresh_token,
                )
                .await?;

                for cal_id in selected_calendars {
                    let token = access_token.clone();
                    let permit = semaphore
                        .clone()
                        .acquire_owned()
                        .await
                        .expect("unable to acquire permit"); // Acquire a permit
                    tasks.push(tokio::task::spawn(async move {
                        let res = microsoft::MicrosoftGraph::get_calendar_events(
                            &token,
                            &cal_id,
                            finder.start,
                            finder.end,
                        )
                        .await?;
                        drop(permit);
                        Ok(res)
                    }));
                }
            }
            Platform::Google => {
                let refresh_token = crate::store::get_token(&account.name)?;
                let access_token = google::refresh_access_token(
                    &cfg.google.to_owned().unwrap_or_default(),
                    &refresh_token,
                )
                .await?;

                for cal_id in selected_calendars {
                    let token = access_token.clone();
                    tasks.push(tokio::task::spawn(async move {
                        let res = google::GoogleAPI::get_calendar_events(
                            &token,
                            &cal_id,
                            finder.start,
                            finder.end,
                        )
                        .await?;
                        Ok(res)
                    }));
                }
            }
            _ => return Err(anyhow::anyhow!("Unsupported platform")),
        }
    }

    let events: Vec<Event> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .filter_map(|r| r.ok())
        .flat_map(Result::unwrap)
        .collect();

    pb.finish_with_message("Retrieved events.");

    let pb = m.add(ProgressBar::new(1));
    pb.set_message("Computing availabilities...");
    pb.enable_steady_tick(Duration::milliseconds(250).to_std().unwrap());

    let availability = finder.get_availability(events)?;
    let slots: Vec<Availability<Local>> = availability.into_iter().flat_map(|(_d, a)| a).collect();

    pb.finish_with_message("Computed availabilities.");

    if slots.is_empty() {
        return Ok(vec![]);
    }

    // TODO: add multi-level multiselect
    // Right arrow goes into a time window (can select granular windows)
    // Left arrow goes back to parent
    // Needs to work with paging
    let selection = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select time window(s)")
        .items(&slots[..])
        .interact()
        .unwrap();

    let selected_slots = selection.into_iter().map(|i| slots.get(i).unwrap());

    // (day, day_avails)
    let days = selected_slots.group_by(|e| (e.start.date()));

    let mut iter = days.into_iter().peekable();

    let mut selected: Vec<Availability<Local>> = vec![];

    while iter.peek().is_some() {
        let i = iter.next();
        let (day, avails) = i.unwrap();

        let day_slots: Vec<&Availability<Local>> = avails.into_iter().collect();
        let windows = split_availability(&day_slots, finder.duration);

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
            .map(|i| *windows.get(i).unwrap())
            .collect();
        selected.append(&mut selected_windows);
    }

    if selected.is_empty() {
        return Err(anyhow::anyhow!("No availabilities selected."));
    }

    let merged = merge_overlapping_avails(selected);
    Ok(merged)
}

pub(crate) async fn create_hold_events(
    db: Store,
    cfg: &AvailConfig,
    merged: &[Availability<Local>],
    m: ProgressIndicator,
) -> anyhow::Result<()> {
    let accounts = db.execute(Box::new(|conn| AccountModel::get(conn)))??;

    let event_title: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("What's the name of your event?")
        .interact_text()?;

    let calendar = db.execute(Box::new(move |conn| {
        CalendarModel::get_hold_event_calendar(conn)
    }))??;

    if calendar.is_none() {
        return Err(anyhow::anyhow!(
            "No calendar is configured to be used for hold events."
        ));
    }

    let pb = m.add(ProgressBar::new(1));
    pb.set_message("Creating hold events...");
    pb.enable_steady_tick(Duration::milliseconds(250).to_std().unwrap());

    let (platform, cal) = calendar.unwrap();

    // Microsoft Graph has 4 concurrent requests limit
    let semaphore = Arc::new(Semaphore::new(4));
    let mut tasks: Vec<JoinHandle<anyhow::Result<()>>> = vec![];

    let account_name = accounts
        .iter()
        .find(|a| a.id == cal.account_id)
        .unwrap()
        .name
        .to_owned();

    match Platform::from(&platform) {
        Platform::Microsoft => {
            for avail in merged.iter() {
                let refresh_token = crate::store::get_token(&account_name)?;
                let access_token = microsoft::refresh_access_token(
                    &cfg.microsoft.to_owned().unwrap_or_default(),
                    &refresh_token,
                )
                .await?;
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("unable to acquire permit"); // Acquire a permit
                let calendar_id = cal.id.to_owned();
                let title = format!("HOLD - {}", event_title);
                let start = avail.start;
                let end = avail.end;

                tasks.push(tokio::task::spawn(async move {
                    let res = microsoft::MicrosoftGraph::create_event(
                        &access_token,
                        &calendar_id,
                        &title,
                        start,
                        end,
                    )
                    .await;
                    drop(permit);
                    res?;
                    Ok(())
                }));
            }
        }
        Platform::Google => {
            for avail in merged.iter() {
                let refresh_token = crate::store::get_token(&account_name)?;
                let access_token = google::refresh_access_token(
                    &cfg.google.to_owned().unwrap_or_default(),
                    &refresh_token,
                )
                .await?;

                let calendar_id = cal.id.to_owned();
                let title = format!("HOLD - {}", event_title);
                let start = avail.start;
                let end = avail.end;

                tasks.push(tokio::task::spawn(async move {
                    google::GoogleAPI::create_event(
                        &access_token,
                        &calendar_id,
                        &title,
                        start,
                        end,
                    )
                    .await?;
                    Ok(())
                }));
            }
        }
        _ => return Err(anyhow::anyhow!("Unsupported platform")),
    }

    let res = futures::future::join_all(tasks).await;
    if res.iter().any(|r| r.is_err()) {
        return Err(anyhow::anyhow!("Failed to create hold events."));
    }

    pb.finish_with_message("Created hold events.");

    Ok(())
}
