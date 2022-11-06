use chrono::{prelude::*, Duration};
use itertools::Itertools;

use crate::events::Event;

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
    <T as TimeZone>::Offset: Copy,
{
    fn overlaps(&self, other: &Availability<T>) -> bool {
        (self.start <= other.end) && (other.start <= self.end)
    }

    fn merge(&mut self, other: &Availability<T>) {
        self.start = DateTime::min(self.start, other.start);
        self.end = DateTime::max(self.end, other.end);
    }
}

pub fn get_free_time(
    mut events: Vec<Event>,
    start: DateTime<Local>,
    end: DateTime<Local>,
    min: NaiveTime,
    max: NaiveTime,
) -> Vec<(Date<Local>, Vec<Availability<Local>>)> {
    let mut avail: Vec<(Date<Local>, Vec<Availability<Local>>)> = vec![];
    let duration = 30;

    events.sort_by_key(|e| e.start);

    let days = events.into_iter().group_by(|e| (e.start.date()));

    let mut iter = days.into_iter();

    // Start at start day and min time
    let mut curr = start.date().and_hms(min.hour(), min.minute(), 0);
    curr = DateTime::max(start, curr);

    while curr < end {
        let day = iter.next();

        // Have another day of events to process
        if let Some((date, events)) = day {
            // Add days that are entirely free
            while curr.date() < date {
                // Whole day till max
                let end = curr.date().and_hms(max.hour(), max.minute(), 0);
                avail.push((curr.date(), vec![Availability { start: curr, end }]));

                // min next day
                curr = (curr + Duration::days(1))
                    .date()
                    .and_hms(min.hour(), min.minute(), 0);
            }

            // events is guaranteed to be non-empty because of the GroupBy

            // Check for availabilities within the day

            let mut day_avail = vec![];
            let mut curr_time = min;

            for event in events {
                let start = event.start;
                let end = event.end;

                // Have time before event
                if curr_time < start.time() {
                    // Meets requirement of minimum duration
                    if start.time() - curr_time >= Duration::minutes(duration) && curr_time < max {
                        let avail_start =
                            start
                                .date()
                                .and_hms(curr_time.hour(), curr_time.minute(), 0);
                        let avail_end = start;
                        day_avail.push(Availability {
                            start: avail_start,
                            end: avail_end,
                        });
                    }
                }
                // Not available until end of this event
                // Only go forwards
                curr_time = NaiveTime::max(end.time(), curr_time);
            }

            // Still have time left over today.
            // TODO: combine with logic in the else below
            if curr_time < max {
                let avail_start = curr.date().and_hms(curr_time.hour(), curr_time.minute(), 0);
                let avail_end = curr.date().and_hms(max.hour(), max.minute(), 0);
                day_avail.push(Availability {
                    start: avail_start,
                    end: avail_end,
                });
            }

            avail.push((curr.date(), day_avail));

            // 12AM next day
            curr = (curr + Duration::days(1))
                .date()
                .and_hms(min.hour(), min.minute(), 0);
        } else {
            // Add days that are entirely free
            // Either before end date or on the end date but before the max time
            while curr.date() < end.date()
                || (curr.date() == end.date() && curr.time() < max && curr < end)
            {
                // Whole day
                let end = curr + (max - curr.time());
                avail.push((curr.date(), vec![Availability { start: curr, end }]));

                // min next day
                curr = (curr + Duration::days(1))
                    .date()
                    .and_hms(min.hour(), min.minute(), 0);
            }
        }
    }

    avail
}

pub fn get_availability(
    events: Vec<Event>,
    start_time: DateTime<Local>,
    end_time: DateTime<Local>,
    duration: Duration,
) -> Vec<(Date<Local>, Vec<Availability<Local>>)> {
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

    fn create_local_datetime(dt_str: &str) -> DateTime<Local> {
        let datetime_fmt = "%m-%d-%Y %H:%M";
        let ndt = NaiveDateTime::parse_from_str(dt_str, datetime_fmt).unwrap();
        Local.from_local_datetime(&ndt).unwrap()
    }

    fn create_event(start: &str, end: &str) -> Event {
        let event_id = "id";
        let event_name = "name";
        Event {
            id: event_id.to_string(),
            name: Some(event_name.to_string()),
            // 12 PM
            start: create_local_datetime(start),
            // 2 PM
            end: create_local_datetime(end),
        }
    }

    #[test]
    fn test_get_availability() {
        let events = vec![
            // 12pm - 2pm
            create_event("10-05-2022 12:00", "10-05-2022 14:00"),
            // 3:30pm - 4pm
            create_event("10-05-2022 15:30", "10-05-2022 16:00"),
            // 4pm - 6pm
            create_event("10-05-2022 16:00", "10-05-2022 18:00"),
            // 7pm - 9pm (outside min-max window)
            create_event("10-05-2022 19:00", "10-05-2022 21:00"),
            // Next day, 5:30am to 7am (outside min-max window)
            create_event("10-06-2022 05:30", "10-06-2022 07:00"),
            // Next day, 8:30am to 12pm
            create_event("10-06-2022 08:30", "10-06-2022 12:00"),
        ];
        let start = create_local_datetime("10-05-2022 00:00");
        let end = create_local_datetime("10-07-2022 00:00");
        let min = NaiveTime::from_hms(9, 0, 0);
        let max = NaiveTime::from_hms(17, 0, 0);

        let avails = get_free_time(events, start, end, min, max);

        assert_eq!(avails.len(), 2);
        let mut day_avails = &avails.get(0).unwrap().1;
        assert_eq!(day_avails.len(), 2);

        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 09:00"),
                end: create_local_datetime("10-05-2022 12:00"),
            }
        );
        assert_eq!(
            *day_avails.get(1).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 14:00"),
                end: create_local_datetime("10-05-2022 15:30"),
            }
        );

        day_avails = &avails.get(1).unwrap().1;
        assert_eq!(day_avails.len(), 1);
        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("10-06-2022 12:00"),
                end: create_local_datetime("10-06-2022 17:00"),
            }
        );
    }

    #[test]
    fn test_get_availability_no_events() {
        let events = vec![];
        let start = create_local_datetime("10-05-2022 00:00");
        let end = create_local_datetime("10-07-2022 00:00");
        let min = NaiveTime::from_hms(9, 0, 0);
        let max = NaiveTime::from_hms(17, 0, 0);

        let avails = get_free_time(events, start, end, min, max);

        assert_eq!(avails.len(), 2);
        let mut day_avails = &avails.get(0).unwrap().1;
        assert_eq!(day_avails.len(), 1);
        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 09:00"),
                end: create_local_datetime("10-05-2022 17:00"),
            }
        );

        day_avails = &avails.get(1).unwrap().1;
        assert_eq!(day_avails.len(), 1);
        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("10-06-2022 09:00"),
                end: create_local_datetime("10-06-2022 17:00"),
            }
        );
    }

    #[test]
    fn test_get_availability_start_with_full_day() {
        let events = vec![
            // No events on start day

            // 12pm - 2pm
            create_event("10-06-2022 12:00", "10-06-2022 14:00"),
            // 3:30pm - 4pm
            create_event("10-06-2022 15:30", "10-06-2022 16:00"),
        ];
        let start = create_local_datetime("10-05-2022 00:00");
        let end = create_local_datetime("10-07-2022 00:00");
        let min = NaiveTime::from_hms(9, 0, 0);
        let max = NaiveTime::from_hms(17, 0, 0);

        let avails = get_free_time(events, start, end, min, max);

        assert_eq!(avails.len(), 2);
        let mut day_avails = &avails.get(0).unwrap().1;
        assert_eq!(day_avails.len(), 1);
        assert_eq!(
            *day_avails.get(0).unwrap(),
            // Full day
            Availability {
                start: create_local_datetime("10-05-2022 09:00"),
                end: create_local_datetime("10-05-2022 17:00"),
            }
        );

        day_avails = &avails.get(1).unwrap().1;
        assert_eq!(day_avails.len(), 3);
        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("10-06-2022 09:00"),
                end: create_local_datetime("10-06-2022 12:00"),
            }
        );
        assert_eq!(
            *day_avails.get(1).unwrap(),
            Availability {
                start: create_local_datetime("10-06-2022 14:00"),
                end: create_local_datetime("10-06-2022 15:30"),
            }
        );
        assert_eq!(
            *day_avails.get(2).unwrap(),
            Availability {
                start: create_local_datetime("10-06-2022 16:00"),
                end: create_local_datetime("10-06-2022 17:00"),
            }
        );
    }
}
