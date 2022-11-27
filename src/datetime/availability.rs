use chrono::{prelude::*, Duration};
use itertools::Itertools;

use std::fmt::Write as _;

#[derive(Clone, Copy, Debug)]
pub struct Availability<T: TimeZone>
where
    <T as TimeZone>::Offset: Copy,
{
    pub start: DateTime<T>,
    pub end: DateTime<T>,
}

impl PartialEq for Availability<Local> {
    fn eq(&self, other: &Self) -> bool {
        self.start.eq(&other.start) && self.end.eq(&other.end)
    }
}

impl std::fmt::Display for Availability<Local> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let duration = self.end - self.start;

        let mut duration_str = String::new();

        if duration.num_hours() >= 1 {
            let _ = write!(duration_str, "{}h", duration.num_hours());
            if duration.num_minutes() % 60 >= 1 {
                let _ = write!(duration_str, "{}m", duration.num_minutes() % 60);
            }
        } else if duration.num_minutes() >= 1 {
            let _ = write!(duration_str, "{}m", duration.num_minutes());
        }

        let day = self.start.format("%a %b %d");
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
    <T as TimeZone>::Offset: Copy,
{
    pub(crate) fn overlaps(&self, other: &Availability<T>) -> bool {
        (self.start <= other.end) && (other.start <= self.end)
    }

    pub(crate) fn merge(&mut self, other: &Availability<T>) {
        self.start = DateTime::min(self.start, other.start);
        self.end = DateTime::max(self.end, other.end);
    }
}

pub fn merge_overlapping_avails<T: TimeZone>(avails: Vec<Availability<T>>) -> Vec<Availability<T>>
where
    <T as TimeZone>::Offset: Copy,
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

pub fn split_availability<T: TimeZone>(
    avails: &Vec<&Availability<T>>,
    duration: Duration,
) -> Vec<Availability<T>>
where
    <T as TimeZone>::Offset: Copy,
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

pub fn format_availability(avails: &[Availability<Local>]) -> String {
    let avail_days = avails.iter().group_by(|e| (e.start.date()));

    let mut iter = avail_days.into_iter().peekable();

    let mut s = String::new();

    while iter.peek().is_some() {
        let i = iter.next();
        let (day, avails) = i.unwrap();

        let _ = writeln!(s, "{}", day.format("%a %b %d %Y"));

        for avail in avails {
            let _ = writeln!(
                s,
                "- {} to {}",
                avail.start.format("%I:%M %p"),
                avail.end.format("%I:%M %p")
            );
        }
    }

    s
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;

    use super::*;

    #[test]
    fn test_overlapping_avail() {
        // Nov 4 12pm - 2pm
        let avail1 = Availability {
            start: DateTime::parse_from_rfc3339("2022-11-04T12:00:00-04:00").unwrap(),
            end: DateTime::parse_from_rfc3339("2022-11-04T14:00:00-04:00").unwrap(),
        };
        // Nov 4 2:30pm - 4pm
        let avail2 = Availability {
            start: DateTime::parse_from_rfc3339("2022-11-04T14:30:00-04:00").unwrap(),
            end: DateTime::parse_from_rfc3339("2022-11-04T16:00:00-04:00").unwrap(),
        };
        // Nov 4 2:00pm - 4pm
        let avail3 = Availability {
            start: DateTime::parse_from_rfc3339("2022-11-04T14:00:00-04:00").unwrap(),
            end: DateTime::parse_from_rfc3339("2022-11-04T16:00:00-04:00").unwrap(),
        };
        assert!(!avail1.overlaps(&avail2));
        assert!(avail1.overlaps(&avail3));
    }

    #[test]
    fn test_merge_overlapping_avails() {
        let avails = vec![
            // Nov 4 12pm - 2pm
            Availability {
                start: DateTime::parse_from_rfc3339("2022-11-04T12:00:00-04:00").unwrap(),
                end: DateTime::parse_from_rfc3339("2022-11-04T14:00:00-04:00").unwrap(),
            },
            // Nov 4 2:30pm - 4pm
            Availability {
                start: DateTime::parse_from_rfc3339("2022-11-04T14:30:00-04:00").unwrap(),
                end: DateTime::parse_from_rfc3339("2022-11-04T16:00:00-04:00").unwrap(),
            },
            // Nov 4 4pm - 5pm
            Availability {
                start: DateTime::parse_from_rfc3339("2022-11-04T16:00:00-04:00").unwrap(),
                end: DateTime::parse_from_rfc3339("2022-11-04T17:00:00-04:00").unwrap(),
            },
            // Nov 4 4:30pm - 6pm
            Availability {
                start: DateTime::parse_from_rfc3339("2022-11-04T16:30:00-04:00").unwrap(),
                end: DateTime::parse_from_rfc3339("2022-11-04T18:00:00-04:00").unwrap(),
            },
        ];

        let merged_avails = merge_overlapping_avails(avails);
        assert_eq!(merged_avails.len(), 2);
    }
}
