use chrono::{prelude::*, Duration};
use itertools::Itertools;

use crate::events::Event;
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
    fn overlaps(&self, other: &Availability<T>) -> bool {
        (self.start <= other.end) && (other.start <= self.end)
    }

    fn merge(&mut self, other: &Availability<T>) {
        self.start = DateTime::min(self.start, other.start);
        self.end = DateTime::max(self.end, other.end);
    }
}

pub struct AvailabilityFinder {
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub min: NaiveTime,
    pub max: NaiveTime,
    pub duration: Duration,
    pub include_weekends: bool,
}

fn is_weekend(weekday: Weekday) -> bool {
    return weekday == Weekday::Sat || weekday == Weekday::Sun;
}

impl AvailabilityFinder {
    pub fn get_availability(
        &self,
        mut events: Vec<Event>,
    ) -> anyhow::Result<Vec<(Date<Local>, Vec<Availability<Local>>)>> {
        let mut avail: Vec<(Date<Local>, Vec<Availability<Local>>)> = vec![];

        events.sort_by_key(|e| e.start);

        let days = events.into_iter().group_by(|e| (e.start.date()));

        let mut iter = days.into_iter();

        // Start at start day and min time
        let mut curr = self
            .start
            .date()
            .and_hms(self.min.hour(), self.min.minute(), 0);
        if curr.minute() % 15 != 0 {
            curr += Duration::minutes((curr.minute() % 15) as i64);
        }
        curr = DateTime::max(self.start, curr);

        while curr < self.end {
            let day = iter.next();

            // Have another day of events to process
            if let Some((date, events)) = day {
                // Add days that are entirely free
                while curr.date() < date && curr.time() < self.max {
                    // Whole day till max
                    let end = curr.date().and_hms(self.max.hour(), self.max.minute(), 0);

                    if self.include_weekends || !is_weekend(curr.weekday()) {
                        avail.push((curr.date(), vec![Availability { start: curr, end }]));
                    }

                    // min next day
                    curr = (curr + Duration::days(1)).date().and_hms(
                        self.min.hour(),
                        self.min.minute(),
                        0,
                    );
                }

                // events is guaranteed to be non-empty because of the GroupBy

                // Check for availabilities within the day

                if !self.include_weekends && is_weekend(date.weekday()) {
                    // Advance date if we haven't already
                    if curr.date() == date {
                        // min next day
                        curr = (curr + Duration::days(1)).date().and_hms(
                            self.min.hour(),
                            self.min.minute(),
                            0,
                        );
                    }

                    continue;
                }

                let mut day_avail = vec![];
                let mut curr_time = self.min;

                for event in events {
                    let start = event.start;
                    let end = event.end;

                    // Have time before event
                    if curr_time < start.time() {
                        // Round datetime here so that the availability doesn't start at an awkward time
                        let avail_start = start
                            .date()
                            .and_hms(curr_time.hour(), curr_time.minute(), 0)
                            .ceil();

                        let avail_end = DateTime::min(
                            start,
                            curr.date().and_hms(self.max.hour(), self.max.minute(), 0),
                        )
                        .floor();

                        // Meets requirement of minimum duration
                        if avail_end.time() - avail_start.time() >= self.duration
                            && avail_start.time() < self.max
                        {
                            day_avail.push(Availability {
                                start: avail_start,
                                end: avail_end,
                            });
                        }
                    }
                    // Not available until end of this event
                    // max to only go forwards
                    curr_time = NaiveTime::max(end.time(), curr_time);
                }

                // Still have time left over today.
                // TODO: combine with logic in the else below
                if curr_time < self.max {
                    let avail_start = curr
                        .date()
                        .and_hms(curr_time.hour(), curr_time.minute(), 0)
                        .ceil();
                    let avail_end = curr.date().and_hms(self.max.hour(), self.max.minute(), 0);

                    if avail_end - avail_start > self.duration {
                        day_avail.push(Availability {
                            start: avail_start,
                            end: avail_end,
                        });
                    }
                }

                avail.push((curr.date(), day_avail));

                // 12AM next day
                curr = (curr + Duration::days(1)).date().and_hms(
                    self.min.hour(),
                    self.min.minute(),
                    0,
                );
            } else {
                // Add days that are entirely free
                // Either before end date or on the end date but before the max time
                while curr.date() < self.end.date()
                    || (curr.date() == self.end.date() && curr < self.end)
                {
                    let start = curr.ceil();

                    // Whole day
                    let end = curr + (self.max - start.time());

                    if start.time() >= self.max || end - start < self.duration {
                        break;
                    }

                    avail.push((curr.date(), vec![Availability { start, end }]));

                    // min next day
                    curr = (curr + Duration::days(1)).date().and_hms(
                        self.min.hour(),
                        self.min.minute(),
                        0,
                    );
                }
            }
        }

        Ok(avail)
    }
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

pub fn format_availability(avails: &Vec<Availability<Local>>) -> String {
    let avail_days = avails.into_iter().group_by(|e| (e.start.date()));

    let mut iter = avail_days.into_iter().peekable();

    let mut s = String::new();

    while iter.peek().is_some() {
        let i = iter.next();
        let (day, avails) = i.unwrap();

        let _ = write!(s, "{}\n", day.format("%a %b %d %Y"));

        for avail in avails {
            let _ = write!(
                s,
                "- {} to {}\n",
                avail.start.format("%I:%M %p"),
                avail.end.format("%I:%M %p")
            );
        }
    }

    s
}

pub trait Round {
    fn ceil(&self) -> Self;
    fn floor(&self) -> Self;
}

impl<T: TimeZone> Round for DateTime<T> {
    fn ceil(&self) -> Self {
        let round_to_minute = 30;

        let minute = self.minute();

        if minute % round_to_minute == 0 {
            return self.clone();
        }

        let new_minute = (minute / round_to_minute + 1) * round_to_minute;

        self.clone() + Duration::minutes((new_minute - minute).into())
    }

    fn floor(&self) -> Self {
        let round_to_minute: i64 = 30;

        let minute: i64 = self.minute().into();

        if minute % round_to_minute == 0 {
            return self.clone();
        }

        let new_minute = (minute / round_to_minute) * round_to_minute;

        let delta: i64 = (new_minute - minute).into();

        self.clone() + Duration::minutes(delta)
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;

    use super::*;

    fn create_local_datetime(dt_str: &str) -> DateTime<Local> {
        let datetime_fmt = "%m-%d-%Y %H:%M";
        let ndt = NaiveDateTime::parse_from_str(dt_str, datetime_fmt).unwrap();
        Local.from_local_datetime(&ndt).unwrap()
    }

    #[test]
    fn test_round_datetime_up() {
        let dt = create_local_datetime("10-05-2022 00:00");
        assert_eq!(dt, dt.ceil());

        let dt = create_local_datetime("10-05-2022 00:02");
        assert_eq!(create_local_datetime("10-05-2022 00:30"), dt.ceil());

        let dt = create_local_datetime("10-05-2022 00:42");
        assert_eq!(create_local_datetime("10-05-2022 01:00"), dt.ceil());

        // Next day
        let dt = create_local_datetime("10-05-2022 23:42");
        assert_eq!(create_local_datetime("10-06-2022 00:00"), dt.ceil());
    }

    #[test]
    fn test_round_datetime_down() {
        let dt = create_local_datetime("10-05-2022 00:00");
        assert_eq!(dt, dt.floor());

        let dt2 = create_local_datetime("10-05-2022 00:02");
        assert_eq!(dt, dt2.floor());

        let dt = create_local_datetime("10-05-2022 00:42");
        assert_eq!(create_local_datetime("10-05-2022 00:30"), dt.floor());
    }

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

        let finder = AvailabilityFinder {
            start: create_local_datetime("10-05-2022 00:00"),
            end: create_local_datetime("10-07-2022 00:00"),
            min: NaiveTime::from_hms(9, 0, 0),
            max: NaiveTime::from_hms(17, 0, 0),
            duration: Duration::minutes(30),
            create_hold_event: false,
            include_weekends: true,
        };
        let avails = finder.get_availability(events).unwrap();

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
    fn test_get_availability_without_weekends() {
        let events = vec![
            // 12pm - 2pm, Friday
            create_event("11-18-2022 12:00", "11-18-2022 14:00"),
            // 3:30pm - 5pm, Friday
            create_event("11-18-2022 15:30", "11-18-2022 17:00"),
            // 3pm - 5pm, Saturday
            create_event("11-19-2022 15:00", "11-19-2022 17:00"),
            // Monday, 8:30am to 11am
            create_event("11-21-2022 08:30", "11-21-2022 11:00"),
            // Monday, 1pm to 2pm
            create_event("11-21-2022 13:00", "11-21-2022 14:00"),
        ];

        let finder = AvailabilityFinder {
            start: create_local_datetime("11-18-2022 00:00"),
            end: create_local_datetime("11-22-2022 00:00"),
            min: NaiveTime::from_hms(9, 0, 0),
            max: NaiveTime::from_hms(17, 0, 0),
            duration: Duration::minutes(30),
            create_hold_event: false,
            include_weekends: false,
        };
        let avails = finder.get_availability(events).unwrap();

        assert_eq!(avails.len(), 2);
        let mut day_avails = &avails.get(0).unwrap().1;
        assert_eq!(day_avails.len(), 2);

        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("11-18-2022 09:00"),
                end: create_local_datetime("11-18-2022 12:00"),
            }
        );
        assert_eq!(
            *day_avails.get(1).unwrap(),
            Availability {
                start: create_local_datetime("11-18-2022 14:00"),
                end: create_local_datetime("11-18-2022 15:30"),
            }
        );

        day_avails = &avails.get(1).unwrap().1;
        assert_eq!(day_avails.len(), 2);
        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("11-21-2022 11:00"),
                end: create_local_datetime("11-21-2022 13:00"),
            }
        );
        assert_eq!(
            *day_avails.get(1).unwrap(),
            Availability {
                start: create_local_datetime("11-21-2022 14:00"),
                end: create_local_datetime("11-21-2022 17:00"),
            }
        );
    }

    #[test]
    fn test_get_availability_rounding() {
        let events = vec![
            // 11:55am - 12:35pm
            create_event("10-05-2022 11:55", "10-05-2022 12:35"),
            // 1:35pm - 2:10pm
            create_event("10-05-2022 13:35", "10-05-2022 14:10"),
            // 3:30pm - 4:05pm
            create_event("10-05-2022 15:30", "10-05-2022 16:05"),
        ];
        let finder = AvailabilityFinder {
            start: create_local_datetime("10-05-2022 00:00"),
            end: create_local_datetime("10-06-2022 00:00"),
            min: NaiveTime::from_hms(9, 0, 0),
            max: NaiveTime::from_hms(17, 0, 0),
            duration: Duration::minutes(30),
            create_hold_event: false,
            include_weekends: true,
        };
        let avails = finder.get_availability(events).unwrap();

        assert_eq!(avails.len(), 1);
        let day_avails = &avails.get(0).unwrap().1;
        assert_eq!(day_avails.len(), 3);

        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 09:00"),
                end: create_local_datetime("10-05-2022 11:30"),
            }
        );
        assert_eq!(
            *day_avails.get(1).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 13:00"),
                end: create_local_datetime("10-05-2022 13:30"),
            }
        );
        assert_eq!(
            *day_avails.get(2).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 14:30"),
                end: create_local_datetime("10-05-2022 15:30"),
            }
        );
    }

    #[test]
    fn test_get_availability_no_events() {
        let finder = AvailabilityFinder {
            start: create_local_datetime("10-05-2022 00:00"),
            end: create_local_datetime("10-07-2022 00:00"),
            min: NaiveTime::from_hms(9, 0, 0),
            max: NaiveTime::from_hms(17, 0, 0),
            duration: Duration::minutes(30),
            create_hold_event: false,
            include_weekends: true,
        };
        let avails = finder.get_availability(vec![]).unwrap();

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
        let finder = AvailabilityFinder {
            start: create_local_datetime("10-05-2022 00:00"),
            end: create_local_datetime("10-07-2022 00:00"),
            min: NaiveTime::from_hms(9, 0, 0),
            max: NaiveTime::from_hms(17, 0, 0),
            duration: Duration::minutes(30),
            create_hold_event: false,
            include_weekends: true,
        };
        let avails = finder.get_availability(events).unwrap();

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
