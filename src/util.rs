use chrono::{prelude::*, Duration};
use itertools::Itertools;

use crate::events::Event;

pub struct Availability {
    start: DateTime<Local>,
    end: DateTime<Local>,
}

impl std::fmt::Display for Availability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let duration = self.end - self.start;

        let mut duration_str: String = "".to_owned();

        if duration.num_hours() >= 1 {
            duration_str.push_str(&format!("{}h", duration.num_hours()));
            if duration.num_minutes() % 60 >= 1 {
                duration_str.push_str(&format!("{}m", duration.num_minutes() % 60));
            }
        } else if duration.num_minutes() >= 1 {
            duration_str.push_str(&format!("{}m", duration.num_minutes()));
        }

        let day = self.start.format("%b %d %Y");
        write!(f, "{} - {} to {} ({})", day, self.start.format("%I:%M %p"), self.end.format("%I:%M %p"), duration_str)
    }
}

pub fn get_free_time(
    mut events: Vec<Event>,
    start: DateTime<Local>,
    end: DateTime<Local>,
    min: NaiveTime,
    max: NaiveTime,
) -> Vec<(Date<Local>, Vec<Availability>)> {
    let mut avail: Vec<(Date<Local>, Vec<Availability>)> = vec![];
    let duration = 30;

    events.sort_by_key(|e| e.start);

    let days = events.into_iter().group_by(|e| (e.start.date()));

    let mut iter = days.into_iter();

    let mut dt = start;
    while dt <= end {
        let day = iter.next();

        if let Some((date, events)) = day {
            // Add days that are entirely free
            while dt.date() < date {
                // Whole day
                let end = (dt + Duration::days(1)).with_hour(0).unwrap().with_minute(0).unwrap();
                avail.push((dt.date(), vec![Availability { start: dt, end }]));

                dt = end;
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
                        let start_time = DateTime::from_local(
                            NaiveDateTime::new(start.date_naive(), curr_time),
                            *Local.timestamp(0, 0).offset(),
                        );
                        let end_time = start;
                        day_avail.push(Availability {
                            start: start_time,
                            end: end_time,
                        });
                    }

                    // Not available until end of this event
                    curr_time = end.time()
                } else {
                    curr_time = std::cmp::max(end.time(), curr_time);
                }
            }

            if curr_time < max {
                let start_time = DateTime::from_local(
                    NaiveDateTime::new(start.date_naive(), curr_time),
                    *Local.timestamp(0, 0).offset(),
                );
                let end_time = DateTime::from_local(
                    NaiveDateTime::new(start.date_naive(), max),
                    *Local.timestamp(0, 0).offset(),
                );
                day_avail.push(Availability {
                    start: start_time,
                    end: end_time,
                });
            }

            avail.push((dt.date(), day_avail));

            // 12AM next day
            dt = (dt + Duration::days(1)).date().and_hms(0, 0, 0);
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

pub fn get_availability(
    events: Vec<Event>,
    duration: Duration,
) -> Vec<(Date<Local>, Vec<Availability>)> {
    let start_time = Local::now();
    let end_time = start_time + Duration::days(7);
    let min = NaiveTime::from_hms(9, 0, 0);
    let max = NaiveTime::from_hms(17, 0, 0);

    let free = get_free_time(events, start_time, end_time, min, max);

    // Filter out all windows < duration
    let avails = free
        .into_iter()
        .map(|(d, a)| {
            (
                d,
                a.into_iter()
                    .filter(|a| a.end - a.start >= duration)
                    .collect(),
            )
        })
        .collect();

    avails
}
