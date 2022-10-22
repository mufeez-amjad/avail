mod oauth;

use oauth::client::MicrosoftOauthClient;
use dialoguer::{Input, MultiSelect};
use chrono::{prelude::*, Duration};
use itertools::Itertools;


use serde::Deserialize;
use serde_json;

#[derive(serde::Deserialize, Clone)]
struct Calendar {
    id: String,
    name: String,

    #[serde(default)]
    selected: bool,
}

#[derive(serde::Deserialize, Clone)]
struct Event {
    id: String,
    #[serde(rename(deserialize = "subject"))]
    name: String,

    #[serde(deserialize_with = "deserialize_json_time")]
    start: DateTime<Utc>,
    #[serde(deserialize_with = "deserialize_json_time")]
    end: DateTime<Utc>,

    #[serde(default)]
    selected: bool,
}

struct Availability {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

fn deserialize_json_time<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
	D: serde::de::Deserializer<'de>,
{
    let json: serde_json::value::Value = serde_json::value::Value::deserialize(deserializer)?;
    let time_str = json.get("dateTime").expect("datetime").as_str().unwrap();
    let tz_str = json.get("timeZone").expect("timeZone").as_str().unwrap();

    // 2022-10-22T20:30:00.0000000
    let datetime = NaiveDateTime::parse_from_str(time_str, "%Y-%m-%dT%H:%M:%S.%f").unwrap();
    
    match tz_str {
        "UTC" => Ok(DateTime::<Utc>::from_utc(datetime, Utc)),
        _ =>  Ok(DateTime::<Utc>::from_utc(datetime, Utc)),
    }
}

impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}, {}, {}, {}", self.id, self.name, self.start, self.end)
    }
}

#[derive(serde::Deserialize)]
struct GraphResponse<T> {
    value: Vec<T>
}

async fn get_calendars(token: String) -> Result<Vec<Calendar>, Box<dyn std::error::Error>> {
    let resp: GraphResponse<Calendar> = reqwest::Client::new()
        .get("https://graph.microsoft.com/v1.0/me/calendars")
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .send()
        .await
        .unwrap()
        .json()
        .await?;

    let calendars = resp.value;

    let calendar_names: Vec<String> = calendars.iter().map(|cal| cal.name.to_owned()).collect(); 
    
    let selected_calendars_idx : Vec<usize> = MultiSelect::new()
    .items(&calendar_names)
    .with_prompt("Select the calendars you want to use")
    .interact()?;

    Ok(selected_calendars_idx.iter().map(|idx| calendars[*idx].clone()).collect())
}

async fn get_calendar_events(token: String, calendar: Calendar, start_time: DateTime<Utc>, end_time: DateTime<Utc>) -> Result<Vec<Event>, Box<dyn std::error::Error + Send + Sync>> {
    let start_time_str = str::replace(&start_time.format("%+").to_string(), "+", "-");
    let end_time_str = str::replace(&end_time.format("%+").to_string(), "+", "-");

    let url = format!("https://graph.microsoft.com/v1.0/me/calendars/{}/calendarView?startDateTime={}&endDateTime={}", calendar.id, start_time_str, end_time_str);

    let resp: GraphResponse<Event> = reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .send()
        .await
        .unwrap()
        .json()
        .await?;

    Ok(resp.value)
}

fn get_free_time(mut events: Vec<Event>, start: DateTime<Utc>, end: DateTime<Utc>, min: NaiveTime, max: NaiveTime) -> Vec<(Date<Utc>, Vec<Availability>)> {
    let mut avail: Vec<(Date<Utc>, Vec<Availability>)> = vec![];
    let duration = 30;

    events.sort_by_key(|e| e.start);
    
    let days = events.iter().group_by(|e| (e.start.date()));

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
                        let start_time = Utc::from_local_datetime(&Utc, &NaiveDateTime::new(start.date_naive(), curr_time)).unwrap();
                        let end_time = start;
                        day_avail.push(Availability { start: start_time, end: end_time });
                    }

                    // Not available until end of this event
                    curr_time = end.time()
                } else {
                    curr_time = std::cmp::max(end.time(), curr_time);
                }
            }
            avail.push((dt.date(), day_avail))
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = MicrosoftOauthClient::new("345ac594-c15f-4904-b9c5-49a29016a8d2", "", "", "");
    let token = client.get_authorization_code().await.secret().to_owned();

    let start_time = Utc::now();
    let end_time = start_time + Duration::days(7);
    let min = NaiveTime::from_hms(9, 0, 0);
    let max = NaiveTime::from_hms(17, 0, 0);

    let calendars = get_calendars(token.to_owned()).await?;

    let mut tasks = vec![];
    for cal in calendars.into_iter() {
        let token = token.to_owned();

        tasks.push(tokio::task::spawn(async move {
            get_calendar_events(token.to_owned(), cal.clone(), start_time, end_time).await
        }));
    }

    let results: Vec<Vec<Event>> = futures::future::join_all(tasks)
    .await
    .into_iter()
    .filter_map(|r| r.ok())
    .map(Result::unwrap)
    .collect();

    let events: Vec<Event> = results.into_iter().flatten().collect();
    
    let avails = get_free_time(events, start_time, end_time, min, max);

    let margin = 20;

    for (day, avail) in avails {
        println!("{:-^margin$}", day.format("%a %B %e"));
        for a in avail {
            if a.end - a.start == Duration::days(1) {
                println!("Whole day!");
            } else {
                println!("{} to {}", a.start.format("%H:%M"), a.end.format("%H:%M"));
            }
        }
        println!()
    }
    
    Ok(())
}

