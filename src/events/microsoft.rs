use async_trait::async_trait;
use chrono::prelude::*;
use reqwest::Response;
use serde::Deserialize;
use serde_json;

use super::{Calendar, Event, GetResources};
use crate::oauth::microsoft::MicrosoftOauthClient;

#[derive(serde::Deserialize, Clone)]
struct GraphCalendar {
    id: String,
    name: String,
}

#[derive(serde::Deserialize, Clone)]
struct GraphEvent {
    id: String,
    #[serde(rename(deserialize = "subject"))]
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
    let tz_str = json.get("timeZone").expect("timeZone").as_str().unwrap();

    // 2022-10-22T20:30:00.0000000
    let naive_time = NaiveDateTime::parse_from_str(time_str, "%Y-%m-%dT%H:%M:%S.%f").unwrap();

    let utc_datetime = match tz_str {
        "UTC" => DateTime::from_utc(naive_time, Utc),
        _ => DateTime::<Utc>::from_utc(naive_time, Utc),
    };

    Ok(utc_datetime.with_timezone(&Local))
}

#[derive(serde::Deserialize)]
struct GraphResponse<T> {
    value: Option<Vec<T>>,
    error: Option<GraphError>,
}

#[derive(serde::Deserialize)]
struct GraphError {
    code: String,
    message: String,
}

pub async fn get_authorization_code() -> (String, String) {
    let client = MicrosoftOauthClient::new("345ac594-c15f-4904-b9c5-49a29016a8d2", "", "", "");
    let token = client.get_authorization_code().await;
    token
}

pub async fn refresh_access_token(refresh_token: String) -> (String, String) {
    let client = MicrosoftOauthClient::new("345ac594-c15f-4904-b9c5-49a29016a8d2", "", "", "");
    let token = client.refresh_access_token(refresh_token).await;
    token
}

pub struct MicrosoftGraph {}

#[async_trait]
impl GetResources for MicrosoftGraph {
    async fn get_calendars(token: String) -> anyhow::Result<Vec<Calendar>> {
        let resp: GraphResponse<GraphCalendar> = reqwest::Client::new()
            .get("https://graph.microsoft.com/v1.0/me/calendars")
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
            .value
            .unwrap()
            .into_iter()
            .map(|c| Calendar {
                id: c.id,
                name: c.name,
                selected: false,
            })
            .collect();
        Ok(calendars)
    }

    async fn get_calendar_events(
        token: String,
        calendar_id: String,
        start_time: DateTime<Local>,
        end_time: DateTime<Local>,
    ) -> anyhow::Result<Vec<Event>> {
        let start_time_str = str::replace(&start_time.format("%+").to_string(), "+", "-");
        let end_time_str = str::replace(&end_time.format("%+").to_string(), "+", "-");

        let url = format!("https://graph.microsoft.com/v1.0/me/calendars/{}/calendarView?startDateTime={}&endDateTime={}", calendar_id, start_time_str, end_time_str);

        let resp: Response = reqwest::Client::new()
            .get(url)
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .send()
            .await
            .unwrap();

        let data: reqwest::Result<GraphResponse<GraphEvent>> = resp.json().await;

        match data {
            Ok(v) => {
                if let Some(err) = v.error {
                    return Err(anyhow::anyhow!("{}: {}", err.code, err.message));
                }

                let events = v
                    .value
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
}
