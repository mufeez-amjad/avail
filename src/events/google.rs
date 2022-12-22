use async_trait::async_trait;
use chrono::prelude::*;
use reqwest::Response;
use serde::Deserialize;
use serde_json;

use super::{Calendar, Event, GetResources};
use crate::{oauth::google, util::AvailConfig};

#[derive(serde::Deserialize, Clone)]
struct GoogleCalendar {
    id: String,
    #[serde(rename(deserialize = "summary"))]
    name: String,
}

#[derive(serde::Deserialize, Clone)]
struct GoogleEvent {
    id: String,
    #[serde(rename(deserialize = "summary"))]
    name: Option<String>,

    #[serde(deserialize_with = "deserialize_json_time")]
    start: DateTime<Local>,
    #[serde(deserialize_with = "deserialize_json_time")]
    end: DateTime<Local>,
}

fn deserialize_json_time<'de, D>(deserializer: D) -> Result<DateTime<Local>, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    let json: serde_json::value::Value = serde_json::value::Value::deserialize(deserializer)?;
    let time_str = json.get("dateTime").expect("datetime").as_str().unwrap();
    let _tz_str = json.get("timeZone").expect("timeZone").as_str().unwrap();

    // 2022-10-22T20:30:00.0000000
    let datetime = DateTime::parse_from_rfc3339(time_str)
        .unwrap_or_else(|_| panic!("failed to parse datetime {}", time_str));

    Ok(datetime.with_timezone(&Local))
}

#[derive(serde::Deserialize)]
struct GoogleResponse<T> {
    items: Option<Vec<T>>,
    error: Option<GoogleError>,
}

#[derive(serde::Deserialize)]
struct GoogleError {
    code: String,
    message: String,
}

pub async fn get_authorization_code(
    cfg: &AvailConfig,
    shutdown_receiver: tokio::sync::oneshot::Receiver<()>,
) -> (String, String) {
    let client = google::new_client(&cfg.google.client_id, &cfg.google.client_secret);
    client.get_authorization_code(shutdown_receiver).await
}

pub async fn refresh_access_token(cfg: &AvailConfig, refresh_token: &str) -> (String, String) {
    let client = google::new_client(&cfg.google.client_id, &cfg.google.client_secret);
    client.refresh_access_token(refresh_token.to_owned()).await
}

pub struct GoogleAPI {}

#[async_trait]
impl GetResources for GoogleAPI {
    async fn get_calendars(token: &str) -> anyhow::Result<Vec<Calendar>> {
        let resp: GoogleResponse<GoogleCalendar> = reqwest::Client::new()
            .get("https://www.googleapis.com/calendar/v3/users/me/calendarList")
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .send()
            .await
            .unwrap()
            .json()
            .await?;

        if let Some(err) = resp.error {
            return Err(anyhow::anyhow!("{}: {}", err.code, err.message));
        }

        let calendars = resp
            .items
            .unwrap()
            .into_iter()
            .map(|c| Calendar {
                id: c.id,
                name: c.name,
                selected: false,
                account_id: 0,
            })
            .collect();
        Ok(calendars)
    }

    async fn get_calendar_events(
        token: &str,
        calendar_id: &str,
        start_time: DateTime<Local>,
        end_time: DateTime<Local>,
    ) -> anyhow::Result<Vec<Event>> {
        let start_time_str = str::replace(&start_time.format("%+").to_string(), "+", "-");
        let end_time_str = str::replace(&end_time.format("%+").to_string(), "+", "-");

        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{}/events?singleEvents=true&orderBy=startTime&timeMin={}&timeMax={}",
            calendar_id, start_time_str, end_time_str
        );

        let resp: Response = reqwest::Client::new()
            .get(url)
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .send()
            .await
            .unwrap();

        let data: reqwest::Result<GoogleResponse<GoogleEvent>> = resp.json().await;

        match data {
            Ok(v) => {
                if let Some(err) = v.error {
                    return Err(anyhow::anyhow!("{}: {}", err.code, err.message));
                }

                let events = v
                    .items
                    .unwrap()
                    .into_iter()
                    .map(|e| Event {
                        id: e.id,
                        name: e.name,
                        start: e.start,
                        end: e.end,
                    })
                    .collect();

                Ok(events)
            }
            Err(e) => {
                println!(
                    "Failed to parse JSON response of calendar events for {}, {}",
                    calendar_id, e
                );
                return Ok(vec![]);
            }
        }
    }

    async fn create_event(
        token: &str,
        calendar_id: &str,
        title: &str,
        start_time: DateTime<Local>,
        end_time: DateTime<Local>,
    ) -> anyhow::Result<()> {
        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{}/events",
            calendar_id
        );

        let body = CreateEventBody {
            summary: title.to_owned(),
            start: GoogleDateTime {
                date_time: start_time.to_rfc3339(),
            },
            end: GoogleDateTime {
                date_time: end_time.to_rfc3339(),
            },
        };

        let client = reqwest::Client::new();
        let _event: GoogleEvent = client
            .post(url)
            .body(serde_json::to_string(&body).unwrap())
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await?;

        Ok(())
    }
}

#[derive(serde::Serialize)]
struct CreateEventBody {
    summary: String,
    start: GoogleDateTime,
    end: GoogleDateTime,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GoogleDateTime {
    date_time: String,
}
