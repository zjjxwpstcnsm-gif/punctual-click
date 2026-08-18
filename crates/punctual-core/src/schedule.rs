use chrono::{
    DateTime, Datelike, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc,
};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::ScheduleError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalScheduleInput {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millisecond: u32,
    pub timezone: String,
}

impl LocalScheduleInput {
    pub fn to_utc(&self) -> Result<DateTime<Utc>, ScheduleError> {
        if self.millisecond > 999 {
            return Err(ScheduleError::InvalidMillisecond);
        }

        let timezone = self
            .timezone
            .parse::<Tz>()
            .map_err(|_| ScheduleError::UnknownTimezone(self.timezone.clone()))?;
        let date = NaiveDate::from_ymd_opt(self.year, self.month, self.day)
            .ok_or(ScheduleError::InvalidDate)?;
        let time =
            NaiveTime::from_hms_milli_opt(self.hour, self.minute, self.second, self.millisecond)
                .ok_or(ScheduleError::InvalidTime)?;
        let local = NaiveDateTime::new(date, time);

        match timezone.from_local_datetime(&local) {
            LocalResult::Single(value) => Ok(value.with_timezone(&Utc)),
            LocalResult::None => Err(ScheduleError::NonexistentLocalTime(self.timezone.clone())),
            LocalResult::Ambiguous(_, _) => {
                Err(ScheduleError::AmbiguousLocalTime(self.timezone.clone()))
            }
        }
    }

    pub fn from_utc(value: DateTime<Utc>, timezone: &str) -> Result<Self, ScheduleError> {
        let timezone_value = timezone
            .parse::<Tz>()
            .map_err(|_| ScheduleError::UnknownTimezone(timezone.to_owned()))?;
        let local = value.with_timezone(&timezone_value);
        Ok(Self {
            year: local.year(),
            month: local.month(),
            day: local.day(),
            hour: local.hour(),
            minute: local.minute(),
            second: local.second(),
            millisecond: local.nanosecond() / 1_000_000,
            timezone: timezone.to_owned(),
        })
    }
}

pub fn format_in_timezone(value: DateTime<Utc>, timezone: &str) -> Result<String, ScheduleError> {
    let timezone = timezone
        .parse::<Tz>()
        .map_err(|_| ScheduleError::UnknownTimezone(timezone.to_owned()))?;
    Ok(value
        .with_timezone(&timezone)
        .format("%Y-%m-%d %H:%M:%S%.3f %Z")
        .to_string())
}

pub fn truncate_to_millis(value: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(value.timestamp_millis())
        .expect("a valid DateTime always has a valid millisecond timestamp")
}

pub fn utc_now_millis() -> DateTime<Utc> {
    truncate_to_millis(Utc::now())
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Timelike};

    use super::*;

    #[test]
    fn converts_tokyo_millisecond_time_to_utc() {
        let input = LocalScheduleInput {
            year: 2026,
            month: 8,
            day: 20,
            hour: 20,
            minute: 0,
            second: 0,
            millisecond: 123,
            timezone: "Asia/Tokyo".into(),
        };

        let utc = input.to_utc().unwrap();
        assert_eq!(utc.year(), 2026);
        assert_eq!(utc.month(), 8);
        assert_eq!(utc.day(), 20);
        assert_eq!(utc.hour(), 11);
        assert_eq!(utc.nanosecond() / 1_000_000, 123);
    }

    #[test]
    fn rejects_invalid_millisecond() {
        let input = LocalScheduleInput {
            year: 2026,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            millisecond: 1_000,
            timezone: "UTC".into(),
        };
        assert_eq!(
            input.to_utc().unwrap_err(),
            ScheduleError::InvalidMillisecond
        );
    }
}
