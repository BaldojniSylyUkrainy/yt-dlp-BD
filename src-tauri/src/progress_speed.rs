//! Smoothed download speed and time left.
//!
//! yt-dlp reports the speed of the last moment, which swings to zero between
//! fragments and turns the time left into "Unknown". This averages the bytes
//! received over the last few seconds and refreshes the shown values at most
//! once a second, so the estimate stays readable.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(10);
/// Below this much history the average is noise; yt-dlp's own value is used.
const MIN_SPAN: Duration = Duration::from_secs(1);
const REFRESH: Duration = Duration::from_secs(1);

#[derive(Debug, Default, PartialEq)]
pub(crate) struct SpeedReading {
    pub speed: Option<String>,
    pub eta: Option<String>,
}

#[derive(Default)]
pub(crate) struct SpeedSmoother {
    samples: VecDeque<(Instant, u64)>,
    last_shown: Option<Instant>,
}

impl SpeedSmoother {
    /// `None` fields mean "keep what is on screen".
    pub(crate) fn sample(
        &mut self,
        now: Instant,
        downloaded: Option<u64>,
        total: Option<u64>,
        raw_speed: Option<&str>,
        raw_eta: Option<&str>,
    ) -> SpeedReading {
        if let Some(downloaded) = downloaded {
            // A smaller count means yt-dlp moved on to the next file or stream.
            if self
                .samples
                .back()
                .is_some_and(|&(_, last)| downloaded < last)
            {
                self.samples.clear();
            }
            self.samples.push_back((now, downloaded));
            while self.samples.len() > 2
                && self
                    .samples
                    .get(1)
                    .is_some_and(|&(time, _)| now.duration_since(time) >= WINDOW)
            {
                self.samples.pop_front();
            }
        }
        if self
            .last_shown
            .is_some_and(|shown| now.duration_since(shown) < REFRESH)
        {
            return SpeedReading::default();
        }

        let average = match (self.samples.front(), self.samples.back()) {
            (Some(&(first_time, first)), Some(&(last_time, last)))
                if last_time.duration_since(first_time) >= MIN_SPAN =>
            {
                Some((last - first) as f64 / last_time.duration_since(first_time).as_secs_f64())
            }
            _ => None,
        };
        let reading = match average {
            Some(bytes_per_second) => SpeedReading {
                speed: Some(format_speed(bytes_per_second)),
                eta: match (total, downloaded) {
                    (Some(total), Some(downloaded)) if bytes_per_second >= 1.0 => Some(format_eta(
                        total.saturating_sub(downloaded) as f64 / bytes_per_second,
                    )),
                    _ => known(raw_eta),
                },
            },
            None => SpeedReading {
                speed: known(raw_speed),
                eta: known(raw_eta),
            },
        };
        if reading != SpeedReading::default() {
            self.last_shown = Some(now);
        }
        reading
    }
}

fn known(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("unknown"))
        .filter(|value| !value.contains("Unknown") && *value != "NA")
        .map(str::to_string)
}

pub(crate) fn parse_byte_count(value: Option<&str>) -> Option<u64> {
    let number = value?.trim().parse::<f64>().ok()?;
    (number.is_finite() && number >= 0.0).then_some(number as u64)
}

fn format_speed(bytes_per_second: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes_per_second.max(0.0);
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.2}{}/s", UNITS[unit])
}

fn format_eta(seconds: f64) -> String {
    let total = seconds.round().clamp(0.0, 99.0 * 3600.0) as u64;
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn a_short_stall_does_not_drop_the_speed_to_zero() {
        let start = Instant::now();
        let mut smoother = SpeedSmoother::default();
        let total = Some(100 * MIB);
        // 5 seconds at 2 MiB/s.
        for second in 0..=5 {
            smoother.sample(
                start + Duration::from_secs(second),
                Some(second * 2 * MIB),
                total,
                Some("2.00MiB/s"),
                Some("00:45"),
            );
        }
        // Two seconds with no new bytes; yt-dlp now says the speed is unknown.
        let reading = smoother.sample(
            start + Duration::from_secs(7),
            Some(10 * MIB),
            total,
            Some("Unknown B/s"),
            Some("Unknown"),
        );
        assert_eq!(reading.speed.as_deref(), Some("1.43MiB/s"));
        assert_eq!(reading.eta.as_deref(), Some("01:03"));
    }

    #[test]
    fn values_refresh_at_most_once_a_second() {
        let start = Instant::now();
        let mut smoother = SpeedSmoother::default();
        let first = smoother.sample(start, Some(0), None, Some("1.00MiB/s"), Some("00:10"));
        assert_eq!(first.speed.as_deref(), Some("1.00MiB/s"));
        let soon = smoother.sample(
            start + Duration::from_millis(250),
            Some(MIB / 4),
            None,
            Some("9.00MiB/s"),
            Some("00:01"),
        );
        assert_eq!(soon, SpeedReading::default());
    }

    #[test]
    fn unknown_values_keep_the_previous_ones_on_screen() {
        let mut smoother = SpeedSmoother::default();
        let reading = smoother.sample(
            Instant::now(),
            None,
            None,
            Some("Unknown B/s"),
            Some("Unknown"),
        );
        assert_eq!(reading, SpeedReading::default());
    }

    #[test]
    fn the_next_stream_starts_a_fresh_average() {
        let start = Instant::now();
        let mut smoother = SpeedSmoother::default();
        smoother.sample(start, Some(0), None, None, None);
        smoother.sample(
            start + Duration::from_secs(5),
            Some(50 * MIB),
            None,
            None,
            None,
        );
        // The audio stream starts from zero bytes.
        let reading = smoother.sample(
            start + Duration::from_secs(6),
            Some(0),
            Some(4 * MIB),
            Some("500.00KiB/s"),
            Some("00:08"),
        );
        assert_eq!(reading.speed.as_deref(), Some("500.00KiB/s"));
        assert_eq!(reading.eta.as_deref(), Some("00:08"));
    }

    #[test]
    fn formats_like_yt_dlp() {
        assert_eq!(format_speed(4.66 * MIB as f64), "4.66MiB/s");
        assert_eq!(format_eta(190.0), "03:10");
        assert_eq!(format_eta(3723.0), "1:02:03");
        assert_eq!(parse_byte_count(Some("1234.7")), Some(1234));
        assert_eq!(parse_byte_count(Some("NA")), None);
    }
}
