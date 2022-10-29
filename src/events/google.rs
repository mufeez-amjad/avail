use async_trait::async_trait;
use chrono::prelude::*;
use serde::Deserialize;
use serde_json;

use super::{Calendar, Event, GetResources};
use crate::oauth::{google::GoogleOauthClient, microsoft::MicrosoftOauthClient};

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
    name: String,

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
    let datetime = DateTime::parse_from_rfc3339(time_str).expect(&format!("failed to parse datetime {}", time_str));

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

pub async fn get_authorization_code() -> (String, String) {
    let client = GoogleOauthClient::new(
        "174899155202-ijgr4acsm2til0nhcac2lhq9c2dh1ie8.apps.googleusercontent.com",
        "",
        "",
        "",
    );
    let token = client.get_authorization_code().await;
    token
}

pub async fn refresh_access_token(refresh_token: String) -> String {
    let client = GoogleOauthClient::new(
        "174899155202-ijgr4acsm2til0nhcac2lhq9c2dh1ie8.apps.googleusercontent.com",
        "",
        "",
        "",
    );
    let token = client.refresh_access_token(refresh_token).await;
    token
}

pub struct GoogleAPI {}

#[async_trait]
impl GetResources for GoogleAPI {
    async fn get_calendars(token: String) -> anyhow::Result<Vec<Calendar>> {
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

        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{}/events?singleEvents=true&orderBy=startTime&timeMin={}&timeMax={}", 
            calendar_id, start_time_str, end_time_str
        );

        let resp: GoogleResponse<GoogleEvent> = reqwest::Client::new()
            .get(url)
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

        let events = resp.items.unwrap().into_iter().map(|e| Event { id: e.id, name: e.name, start: e.start, end: e.end }).collect();

        Ok(events)
    }
}
