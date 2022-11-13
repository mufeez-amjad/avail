use std::sync::Arc;

use chrono::{prelude::*, Duration};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, MultiSelect, Select};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use itertools::Itertools;
use tokio::{sync::Semaphore, task::JoinHandle};

use crate::events::{google, microsoft, Calendar, Event, GetResources};
use crate::store::{Account, CalendarModel, Model, Platform, Store, PLATFORMS};
use crate::util::{
    get_availability, merge_overlapping_avails, print_availability, split_availability,
    Availability,
};

pub async fn add_account(db: Store, email: &str) -> anyhow::Result<()> {
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Which platform would you like to add an account for?")
        .items(&PLATFORMS[..])
        .interact()
        .unwrap();

    let selected_platform = PLATFORMS[selection];

    match selected_platform {
        Platform::Microsoft => {
            let (_, refresh_token) = microsoft::get_authorization_code().await;
            crate::store::store_token(email, &refresh_token)?;
        }
        Platform::Google => {
            let (_, refresh_token) = google::get_authorization_code().await;
            crate::store::store_token(email, &refresh_token)?;
        }
    }

    let account = Account {
        name: email.to_owned(),
        platform: Some(selected_platform),
        id: None,
    };
    db.execute(Box::new(move |conn| account.insert(conn)))??;
    println!("Successfully added account.");

    Ok(())
}

pub fn remove_account(db: Store, email: &str) -> anyhow::Result<()> {
    if Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Do you want to delete the account \"{}\"?", email))
        .interact()
        .unwrap()
    {
        crate::store::delete_token(email)?;
        let account = Account {
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
    let accounts = db.execute(Box::new(|conn| Account::get(conn)))??;

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

pub async fn refresh_calendars(db: Store) -> anyhow::Result<()> {
    let accounts = db.execute(Box::new(|conn| Account::get(conn)))??;
    for account in accounts {
        let refresh_token = crate::store::get_token(&account.name)?;

        let account_id = account.id.unwrap().to_owned();
        let mut calendars = match account.platform.unwrap() {
            Platform::Microsoft => {
                let (access_token, _) = microsoft::refresh_access_token(&refresh_token).await;
                microsoft::MicrosoftGraph::get_calendars(&access_token).await?
            }
            Platform::Google => {
                let access_token = google::refresh_access_token(&refresh_token).await;
                google::GoogleAPI::get_calendars(&access_token).await?
            }
        };

        let mut prev_unselected_calendars = db
            .execute(Box::new(move |conn| {
                CalendarModel::get_all_selected(conn, &account_id.to_owned(), false)
            }))??
            .into_iter()
            .map(|c| c.calendar_id);

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
    Ok(())
}

pub async fn find_availability(
    db: Store,
    start_time: DateTime<Local>,
    end_time: DateTime<Local>,
    min: NaiveTime,
    max: NaiveTime,
    duration: Duration,
    create_hold_event: bool,
) -> anyhow::Result<()> {
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
                let refresh_token = crate::store::get_token(&account.name)?;
                let (access_token, _) = microsoft::refresh_access_token(&refresh_token).await;

                for cal_id in selected_calendars {
                    let token = access_token.clone();
                    let permit = semaphore
                        .clone()
                        .acquire_owned()
                        .await
                        .expect("unable to acquire permit"); // Acquire a permit
                    tasks.push(tokio::task::spawn(async move {
                        let res = microsoft::MicrosoftGraph::get_calendar_events(
                            &token, &cal_id, start_time, end_time,
                        )
                        .await?;
                        drop(permit);
                        Ok(res)
                    }));
                }
            }
            Platform::Google => {
                let refresh_token = crate::store::get_token(&account.name)?;
                let access_token = google::refresh_access_token(&refresh_token).await;

                for cal_id in selected_calendars {
                    let token = access_token.clone();
                    tasks.push(tokio::task::spawn(async move {
                        let res = google::GoogleAPI::get_calendar_events(
                            &token, &cal_id, start_time, end_time,
                        )
                        .await?;
                        Ok(res)
                    }));
                }
            }
        }
    }

    let events: Vec<Event> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .filter_map(|r| r.ok())
        .flat_map(Result::unwrap)
        .collect();

    pb.set_message("Computing availabilities...");

    let availability = get_availability(events, start_time, end_time, min, max, duration)?;
    let slots: Vec<Availability<Local>> = availability.into_iter().flat_map(|(_d, a)| a).collect();

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

    let selected_slots = selection.into_iter().map(|i| slots.get(i).unwrap());

    // (day, day_avails)
    let days = selected_slots.group_by(|e| (e.start.date()));

    let mut iter = days.into_iter().peekable();

    let mut selected: Vec<Availability<Local>> = vec![];

    while iter.peek().is_some() {
        let i = iter.next();
        let (day, avails) = i.unwrap();

        let day_slots: Vec<&Availability<Local>> = avails.into_iter().collect();
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
            .map(|i| *windows.get(i).unwrap())
            .collect();
        selected.append(&mut selected_windows);
    }

    if selected.is_empty() {
        return Ok(());
    }

    let merged = merge_overlapping_avails(selected);

    if !create_hold_event {
        print_availability(merged);
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

    // Microsoft Graph has 4 concurrent requests limit
    let semaphore = Arc::new(Semaphore::new(4));
    let mut tasks: Vec<JoinHandle<anyhow::Result<()>>> = vec![];

    match accounts[selection].platform.unwrap() {
        Platform::Microsoft => {
            let account_id = accounts[selection].id.unwrap().to_owned();
            let editable_calendars: Vec<Calendar> = db
                .execute(Box::new(move |conn| {
                    CalendarModel::get_all_editable(conn, &account_id, true)
                }))??
                .into_iter()
                .map(|c| Calendar {
                    id: c.calendar_id,
                    name: c.calendar_name,
                    selected: c.is_selected,
                    can_edit: c.can_edit.unwrap(),
                })
                .collect_vec();

            let selected_calendar = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Which calendar would you like to add the events to?")
                .items(&editable_calendars)
                .interact()
                .unwrap();

            let pb = m.add(ProgressBar::new(1));
            pb.set_style(spinner_style.clone());
            pb.set_message("Creating hold events...");
            pb.enable_steady_tick(Duration::milliseconds(250).to_std().unwrap());

            for avail in merged.iter() {
                let refresh_token = crate::store::get_token(&accounts[selection].name)?;
                let (access_token, _) = microsoft::refresh_access_token(&refresh_token).await;
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("unable to acquire permit"); // Acquire a permit
                let calendar_id = editable_calendars[selected_calendar].id.to_owned();
                let title = format!("HOLD - {}", event_title);
                let start = avail.start.clone();
                let end = avail.end.clone();

                tasks.push(tokio::task::spawn(async move {
                    microsoft::MicrosoftGraph::create_event(
                        &access_token,
                        &calendar_id,
                        &title,
                        start,
                        end,
                    )
                    .await?;
                    drop(permit);
                    Ok(())
                }));
            }
        }
        Platform::Google => {
            let account_id = accounts[selection].id.unwrap().to_owned();
            let editable_calendars: Vec<Calendar> = db
                .execute(Box::new(move |conn| {
                    CalendarModel::get_all_editable(conn, &account_id, true)
                }))??
                .into_iter()
                .map(|c| Calendar {
                    id: c.calendar_id,
                    name: c.calendar_name,
                    selected: c.is_selected,
                    can_edit: c.can_edit.unwrap(),
                })
                .collect_vec();

            let selected_calendar = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Which calendar would you like to add the events to?")
                .items(&editable_calendars)
                .interact()
                .unwrap();

            let pb = m.add(ProgressBar::new(1));
            pb.set_style(spinner_style.clone());
            pb.set_message("Creating hold events...");
            pb.enable_steady_tick(Duration::milliseconds(250).to_std().unwrap());

            for avail in merged.iter() {
                let refresh_token = crate::store::get_token(&accounts[selection].name)?;
                let access_token = google::refresh_access_token(&refresh_token).await;

                let calendar_id = editable_calendars[selected_calendar].id.to_owned();
                let title = format!("HOLD - {}", event_title);
                let start = avail.start.clone();
                let end = avail.end.clone();

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
    }

    futures::future::join_all(tasks).await;

    pb.finish_with_message("Created hold events.");

    print_availability(merged);

    Ok(())
}
