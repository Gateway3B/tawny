use super::*;
use std::sync::atomic::{AtomicI32, Ordering};

/// The viewer's offset from UTC, east positive. Rust cannot find it reliably
/// in a WebView (wasm has no zone database, Android hides it), so the shell
/// asks the page once at startup. Until then times read as UTC.
static LOCAL_UTC_OFFSET_MINUTES: AtomicI32 = AtomicI32::new(0);

pub fn set_local_utc_offset_minutes(minutes: i32) {
    LOCAL_UTC_OFFSET_MINUTES.store(minutes, Ordering::Relaxed);
}

/// Leads a stored publish date that is only approximate: the moment a
/// relative date such as "3 months ago" stood for when it was read, written
/// `~2026-07-19T04:00:00Z`. Stored as the text it was read as, "3 months ago"
/// stayed three months ago for as long as the row lived; stored as a moment, it
/// is shown relative to now and so ages, while still ranking below an exact
/// date.
const APPROXIMATE_MARK: char = '~';

fn now_epoch() -> i64 {
    use web_time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// How long ago something `seconds` old was, as YouTube writes it ("2 years
/// ago").
pub fn age_label(seconds: i64) -> String {
    let (count, unit) = match seconds.max(0) {
        s if s < 3_600 => ((s / 60).max(1), "minute"),
        s if s < 86_400 => (s / 3_600, "hour"),
        s if s < 7 * 86_400 => (s / 86_400, "day"),
        s if s < 30 * 86_400 => (s / (7 * 86_400), "week"),
        s if s < 365 * 86_400 => (s / (30 * 86_400), "month"),
        s => (s / (365 * 86_400), "year"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel_id: String,
    pub channel_name: String,
    pub thumbnail_url: String,
    pub published_at: String,
    pub duration_seconds: u64,
    pub view_count: String,
    pub progress_seconds: u64,
    pub watched: bool,
    pub is_live: bool,
    #[serde(default)]
    pub is_short: bool,
    /// Play this one without its video stream. Remembered per video, because it
    /// is a property of the thing being watched - a podcast stays audio, a music
    /// video does not - rather than a global mode.
    #[serde(default)]
    pub audio_only: bool,
    /// The channel's avatar, filled in by the server on every video it hands
    /// out so a card need not look the channel up.
    #[serde(default)]
    pub channel_avatar_url: Option<String>,
}

impl Video {
    pub fn duration_label(&self) -> String {
        if self.is_live {
            return "LIVE".to_string();
        }
        let hours = self.duration_seconds / 3600;
        let minutes = (self.duration_seconds % 3600) / 60;
        let seconds = self.duration_seconds % 60;
        if hours > 0 {
            format!("{hours}:{minutes:02}:{seconds:02}")
        } else {
            format!("{minutes}:{seconds:02}")
        }
    }

    pub fn progress_percent(&self) -> f64 {
        if self.duration_seconds == 0 {
            0.0
        } else {
            (self.progress_seconds as f64 / self.duration_seconds as f64 * 100.0).min(100.0)
        }
    }

    /// Where opening this video should start. A finished video starts over,
    /// and "finished" is judged from the position as well as the flag: the
    /// copy a Back or a tap reopens can predate the flag being set, and
    /// resuming a few seconds before the end only jumps on to the next video.
    pub fn resume_seconds(&self) -> u64 {
        if self.watched || self.is_effectively_over(self.progress_seconds) {
            0
        } else {
            self.progress_seconds
        }
    }

    /// Whether `seconds` is close enough to the end to count as finished.
    /// YouTube stops short of the exact duration often enough that an exact
    /// match would rarely fire.
    pub fn is_effectively_over(&self, seconds: u64) -> bool {
        self.duration_seconds > 0 && seconds + 15 >= self.duration_seconds
    }

    /// `published_at` for a date known only roughly, at `timestamp`: see
    /// [`APPROXIMATE_MARK`].
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    pub fn approximate_published_at(timestamp: i64) -> String {
        // Written out by hand: the web client builds `time` without its
        // formatting feature.
        time::OffsetDateTime::from_unix_timestamp(timestamp)
            .map(|moment| {
                format!(
                    "{APPROXIMATE_MARK}{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                    moment.year(),
                    u8::from(moment.month()),
                    moment.day(),
                    moment.hour(),
                    moment.minute(),
                    moment.second()
                )
            })
            .unwrap_or_default()
    }

    /// Store relative text such as "3 months ago" as the moment it stands
    /// for, marked approximate, so it reads correctly later. Anything else is
    /// left as it is: a date already pins the day, and "Streaming now" or
    /// "Yesterday" are not worth the guess.
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    pub fn settle_relative_date(&mut self) {
        let value = self.published_at.trim();
        if value.starts_with(APPROXIMATE_MARK)
            || self.publish_precision() != 1
            || !value.to_ascii_lowercase().ends_with(" ago")
        {
            return;
        }
        self.published_at = Self::approximate_published_at(self.published_epoch());
    }

    pub fn published_epoch(&self) -> i64 {
        use time::{Date, OffsetDateTime, format_description::well_known::Rfc3339};

        let value = self.published_at.trim();
        let value = value.strip_prefix(APPROXIMATE_MARK).unwrap_or(value);
        if let Ok(timestamp) = OffsetDateTime::parse(value, &Rfc3339) {
            return timestamp.unix_timestamp();
        }
        if value.len() >= 10
            && let Ok(format) =
                time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
            && let Ok(date) = Date::parse(&value[..10], &format)
        {
            return date.midnight().assume_utc().unix_timestamp();
        }
        if let Some(date) = text_date(value) {
            return date.midnight().assume_utc().unix_timestamp();
        }

        let lower = value.to_ascii_lowercase();
        let now = now_epoch();
        if lower.contains("streaming now") || lower == "live" || lower.contains("just now") {
            return now;
        }
        if lower.contains("yesterday") {
            return now - 86_400;
        }
        let words = lower.split_whitespace().collect::<Vec<_>>();
        for (index, word) in words.iter().enumerate() {
            let count = word
                .trim_matches(|character: char| !character.is_ascii_digit())
                .parse::<i64>()
                .ok()
                .or_else(|| matches!(*word, "a" | "an").then_some(1));
            let Some(count) = count else { continue };
            let Some(unit) = words.get(index + 1) else {
                continue;
            };
            let seconds = if unit.starts_with("second") {
                1
            } else if unit.starts_with("minute") {
                60
            } else if unit.starts_with("hour") {
                3_600
            } else if unit.starts_with("day") {
                86_400
            } else if unit.starts_with("week") {
                7 * 86_400
            } else if unit.starts_with("month") {
                30 * 86_400
            } else if unit.starts_with("year") {
                365 * 86_400
            } else {
                continue;
            };
            return now.saturating_sub(count.saturating_mul(seconds));
        }
        0
    }

    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    /// How exactly `published_at` pins the upload down: 3 for a timestamp, 2 for
    /// a calendar date, 1 for relative text ("3 days ago") or an approximate
    /// moment, 0 for nothing usable.
    pub(crate) fn publish_precision(&self) -> u8 {
        use time::{OffsetDateTime, format_description::well_known::Rfc3339};

        let value = self.published_at.trim();
        if value.starts_with(APPROXIMATE_MARK) {
            return if self.published_epoch() != 0 { 1 } else { 0 };
        }
        if OffsetDateTime::parse(value, &Rfc3339).is_ok() {
            return 3;
        }
        let iso_date = value.len() >= 10
            && time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
                .is_ok_and(|format| time::Date::parse(&value[..10], &format).is_ok());
        if iso_date || text_date(value).is_some() {
            return 2;
        }
        if self.published_epoch() != 0 { 1 } else { 0 }
    }

    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    /// Keep `previous`'s publish date when it is more exact than this one.
    ///
    /// A video's date does not change, but the sources describing it do: the
    /// feed gives a timestamp, the watch page a day, a related-videos shelf "11
    /// months ago". Taking whichever arrived last moved a video within a
    /// newest-first playlist just for having been opened.
    pub fn keep_finer_publish_date(&mut self, previous: &Video) {
        if previous.publish_precision() > self.publish_precision() {
            self.published_at = previous.published_at.clone();
        }
    }

    /// The publish date as it should be shown.
    ///
    /// Most sources give human text ("2 hours ago", "Jul 7, 2026") which is
    /// passed through untouched. RSS entries and extractor records that carry
    /// no display text fall back to a machine timestamp such as
    /// `2026-07-07 0:00:00.0 +00:00:00`, which must not reach the UI. Those are
    /// reformatted here rather than at ingest so rows already cached with a raw
    /// timestamp render correctly too.
    pub fn published_label(&self) -> String {
        use time::{Date, OffsetDateTime, format_description::well_known::Rfc3339};

        let value = self.published_at.trim();
        // "From YouTube" is the ingest placeholder for "no date was reported".
        // It is not a date, so it should not be rendered where one belongs.
        if value.is_empty() || value == "From YouTube" {
            return String::new();
        }
        // Only roughly known, so it says how long ago, as of now.
        if value.starts_with(APPROXIMATE_MARK) {
            let epoch = self.published_epoch();
            return if epoch == 0 {
                String::new()
            } else {
                age_label(now_epoch() - epoch)
            };
        }
        // A real timestamp gets its time of day, in the viewer's zone. Midnight
        // UTC is how a source with only a date writes it, so that stays a date.
        if let Ok(timestamp) = OffsetDateTime::parse(value, &Rfc3339)
            && timestamp.time() != time::Time::MIDNIGHT
        {
            let minutes = LOCAL_UTC_OFFSET_MINUTES.load(Ordering::Relaxed);
            let offset =
                time::UtcOffset::from_whole_seconds(minutes * 60).unwrap_or(time::UtcOffset::UTC);
            let local = timestamp.to_offset(offset);
            let hour = match local.hour() % 12 {
                0 => 12,
                hour => hour,
            };
            return format!(
                "{} {}, {}, {hour}:{:02} {}",
                short_month(local.month()),
                local.day(),
                local.year(),
                local.minute(),
                if local.hour() < 12 { "AM" } else { "PM" },
            );
        }
        let date = OffsetDateTime::parse(value, &Rfc3339)
            .ok()
            .map(|timestamp| timestamp.date())
            .or_else(|| {
                let format =
                    time::format_description::parse_borrowed::<2>("[year]-[month]-[day]").ok()?;
                Date::parse(value.get(..10)?, &format).ok()
            });
        match date {
            Some(date) => format!(
                "{} {}, {}",
                short_month(date.month()),
                date.day(),
                date.year()
            ),
            None => value.to_string(),
        }
    }
}

impl Video {
    /// The "views · date" line, dropping either half when it is unknown so the
    /// separator never dangles.
    pub fn stats_label(&self) -> String {
        let date = self.published_label();
        match (self.view_count.trim().is_empty(), date.is_empty()) {
            (true, true) => String::new(),
            (false, true) => self.view_count.trim().to_string(),
            (true, false) => date,
            (false, false) => format!("{} · {date}", self.view_count.trim()),
        }
    }
}

/// A written-out English date such as "Aug 2, 2013", "Premiered Aug 2, 2013"
/// or "Streamed live on August 2, 2013".
///
/// Video details used to be stored with YouTube's display text, and a date the
/// sorts cannot read counts as 1970: the video opened last sank to the bottom
/// of a newest-first playlist, stranding a run on it. Rows cached that way are
/// still around, so they are read rather than refetched.
pub fn text_date(value: &str) -> Option<time::Date> {
    let words = value
        .split(|character: char| character.is_whitespace() || character == ',')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    words.windows(3).find_map(|window| {
        let month = month_from_name(window[0])?;
        let day = window[1].parse::<u8>().ok()?;
        let year = window[2].parse::<i32>().ok().filter(|year| *year >= 1000)?;
        time::Date::from_calendar_date(year, month, day).ok()
    })
}

pub(crate) fn month_from_name(word: &str) -> Option<time::Month> {
    use time::Month::*;
    let prefix = word.get(..3)?.to_ascii_lowercase();
    Some(match prefix.as_str() {
        "jan" => January,
        "feb" => February,
        "mar" => March,
        "apr" => April,
        "may" => May,
        "jun" => June,
        "jul" => July,
        "aug" => August,
        "sep" => September,
        "oct" => October,
        "nov" => November,
        "dec" => December,
        _ => return None,
    })
}

pub(crate) fn short_month(month: time::Month) -> &'static str {
    use time::Month::*;
    match month {
        January => "Jan",
        February => "Feb",
        March => "Mar",
        April => "Apr",
        May => "May",
        June => "Jun",
        July => "Jul",
        August => "Aug",
        September => "Sep",
        October => "Oct",
        November => "Nov",
        December => "Dec",
    }
}
