use chrono::{prelude::*, Duration};
use itertools::Itertools;

use crate::events::Event;

#[derive(Clone, Copy)]
pub struct Availability<T: TimeZone>
where
    <T as chrono::TimeZone>::Offset: Copy,
{
    pub start: DateTime<T>,
    pub end: DateTime<T>,
}

impl Availability<Utc> {
    pub fn to_local(&self) -> Availability<Local> {
        Availability {
            start: DateTime::from(self.start),
            end: DateTime::from(self.end),
        }
    }
}

impl std::fmt::Display for Availability<Local> {
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
        write!(
            f,
            "{} - {} to {} ({})",
            day,
            self.start.format("%I:%M %p"),
            self.end.format("%I:%M %p"),
            duration_str
        )
    }
}

impl<T: TimeZone> Availability<T>
where
    <T as chrono::TimeZone>::Offset: Copy,
{
    fn overlaps(&self, other: &Availability<T>) -> bool {
        (other.start >= self.start && other.start <= self.end)
            || (other.end >= self.start && other.end <= self.end)
    }

    fn merge(&mut self, other: &Availability<T>) {
        self.start = DateTime::min(self.start, other.start);
        self.end = DateTime::max(self.end, other.end);
    }
}

pub fn get_free_time(
    mut events: Vec<Event>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    min: NaiveTime,
    max: NaiveTime,
) -> Vec<(Date<Utc>, Vec<Availability<Utc>>)> {
    let mut avail: Vec<(Date<Utc>, Vec<Availability<Utc>>)> = vec![];
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
                let end = (dt + Duration::days(1)).date().and_hms(0, 0, 0);
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
                        let start_date_time = NaiveDateTime::new(start.date_naive(), curr_time);
                        let start_time = Utc.from_utc_datetime(&start_date_time);

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
                let start_date_time = NaiveDateTime::new(start.date_naive(), curr_time);
                let start_time = Utc.from_utc_datetime(&start_date_time);

                let end_date_time = NaiveDateTime::new(start.date_naive(), max);
                let end_time = Utc.from_utc_datetime(&end_date_time);
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
) -> Vec<(Date<Utc>, Vec<Availability<Utc>>)> {
    let start_time = Utc::now();
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

pub fn split_availability<T: TimeZone>(
    avails: &Vec<&Availability<T>>,
    duration: Duration,
) -> Vec<Availability<T>>
where
    <T as chrono::TimeZone>::Offset: Copy,
{
    let mut res = vec![];

    for avail in avails {
        let mut curr = avail.start;
        while curr + duration <= avail.end {
            res.push(Availability {
                start: curr,
                end: curr + duration,
            });
            curr += duration;
        }
    }

    res
}

pub fn merge_overlapping_avails<T: TimeZone>(avails: Vec<Availability<T>>) -> Vec<Availability<T>>
where
    <T as chrono::TimeZone>::Offset: Copy,
{
    let mut res: Vec<Availability<T>> = vec![];

    for avail in avails {
        if let Some(last) = res.last_mut() {
            if last.overlaps(&avail) {
                last.merge(&avail);
            } else {
                res.push(avail);
            }
        } else {
            res.push(avail);
        }
    }

    res
}
