pub mod google;
pub mod microsoft;

use async_trait::async_trait;
use chrono::prelude::*;

pub struct Calendar {
    pub account_id: u32,
    pub id: String,
    pub name: String,
    pub selected: bool,
    pub can_edit: bool,
    pub use_for_hold_events: bool,
}

impl std::fmt::Display for Calendar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

pub struct Event {
    pub id: String,
    pub name: Option<String>,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
}

#[async_trait]
pub trait GetResources {
    async fn get_calendars(token: &str) -> anyhow::Result<Vec<Calendar>>;
    async fn get_calendar_events(
        token: &str,
        calendar_id: &str,
        start_time: DateTime<Local>,
        end_time: DateTime<Local>,
    ) -> anyhow::Result<Vec<Event>>;
    async fn create_event(
        token: &str,
        calendar_id: &str,
        title: &str,
        start_time: DateTime<Local>,
        end_time: DateTime<Local>,
    ) -> anyhow::Result<()>;
}
