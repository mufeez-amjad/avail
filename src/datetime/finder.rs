use chrono::{prelude::*, Duration};
use itertools::Itertools;

use crate::events::Event;

use super::availability::Availability;

pub struct AvailabilityFinder {
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub min: NaiveTime,
    pub max: NaiveTime,
    pub duration: Duration,
    pub include_weekends: bool,
}

fn is_weekend(weekday: Weekday) -> bool {
    weekday == Weekday::Sat || weekday == Weekday::Sun
}

#[allow(clippy::type_complexity)]
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

        // Set curr to be max of now and curr.
        curr = DateTime::max(curr, self.start);
        curr = curr.ceil();

        while curr < self.end {
            let day = iter.next();

            // Have another day of events to process
            if let Some((date, events)) = day {
                // Add days that are entirely free
                //
                // If curr.date < date and curr.time < max, then we advance to the start of the next day
                while curr.date() < date {
                    if curr.time() < self.max {
                        // Whole day till max
                        let end = curr.date().and_hms(self.max.hour(), self.max.minute(), 0);

                        if self.include_weekends || !is_weekend(curr.weekday()) {
                            avail.push((
                                curr.date(),
                                vec![Availability {
                                    start: curr.date().and_hms(
                                        self.min.hour(),
                                        self.max.minute(),
                                        0,
                                    ),
                                    end,
                                }],
                            ));
                        }
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

                    if avail_end - avail_start >= self.duration {
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
                    if !is_weekend(curr.weekday()) || self.include_weekends {
                        let start = curr.ceil();

                        // Whole day
                        let end = curr + (self.max - start.time());

                        if start.time() <= self.max && end - start >= self.duration {
                            avail.push((curr.date(), vec![Availability { start, end }]));
                        }
                    }

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

pub trait Round {
    fn ceil(&self) -> Self;
    fn floor(&self) -> Self;
}

impl<T: TimeZone> Round for DateTime<T> {
    fn ceil(&self) -> Self {
        let time = self.date().and_hms(self.hour(), self.minute(), 0);
        let minute = self.minute();

        let round_to_minute = 30;

        if minute % round_to_minute == 0 {
            return time;
        }

        let new_minute = (minute / round_to_minute + 1) * round_to_minute;

        time + Duration::minutes((new_minute - minute).into())
    }

    fn floor(&self) -> Self {
        let time = self.date().and_hms(self.hour(), self.minute(), 0);

        let round_to_minute: i64 = 30;

        let minute: i64 = self.minute().into();

        if minute % round_to_minute == 0 {
            return time;
        }

        let new_minute = (minute / round_to_minute) * round_to_minute;

        let delta: i64 = new_minute - minute;

        time + Duration::minutes(delta)
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

        // Should disregard seconds
        let dt = create_local_datetime("10-05-2022 00:02") + Duration::seconds(30);
        assert_eq!(create_local_datetime("10-05-2022 00:30"), dt.ceil());
    }

    #[test]
    fn test_round_datetime_down() {
        let dt = create_local_datetime("10-05-2022 00:00");
        assert_eq!(dt, dt.floor());

        let dt2 = create_local_datetime("10-05-2022 00:02");
        assert_eq!(dt, dt2.floor());

        let dt3 = create_local_datetime("10-05-2022 00:42");
        assert_eq!(create_local_datetime("10-05-2022 00:30"), dt3.floor());

        // Should disregard seconds
        let dt4 = create_local_datetime("10-05-2022 00:02") + Duration::seconds(30);
        assert_eq!(dt, dt4.floor());
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
            include_weekends: true,
        };
        let avails = finder.get_availability(events).unwrap();

        assert_eq!(avails.len(), 1);
        let day_avails = &avails.get(0).unwrap().1;
        assert_eq!(day_avails.len(), 4);

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
        assert_eq!(
            *day_avails.get(3).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 16:30"),
                end: create_local_datetime("10-05-2022 17:00"),
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
