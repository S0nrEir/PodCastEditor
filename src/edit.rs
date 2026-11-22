use anyhow::{Context, Result, ensure};
use std::ops::Range;

pub type Span = Range<u64>;

/// Positions are sample frames in the imported track, with an exclusive end.
pub fn time_frame(text: &str, rate: u32, limit: u64) -> Result<u64> {
    let parts: Vec<_> = text.trim().split(':').collect();
    ensure!(parts.len() <= 3, "请输入 秒、分:秒 或 时:分:秒");
    let (&last, prefix) = parts.split_last().context("请输入时间")?;
    let (seconds, fraction) = last
        .split_once('.')
        .map_or((last, None), |(a, b)| (a, Some(b)));
    let integer = |s: &str| -> Result<u64> {
        ensure!(
            !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()),
            "时间含有无效字符"
        );
        s.parse().context("时间数值过大")
    };
    let mut whole = 0u64;
    for (i, part) in prefix.iter().copied().chain([seconds]).enumerate() {
        let v = integer(part)?;
        ensure!(i == 0 || v < 60, "冒号后的分、秒必须小于 60");
        whole = whole
            .checked_mul(60)
            .and_then(|n| n.checked_add(v))
            .context("时间数值过大")?;
    }
    let micros = if let Some(digits) = fraction {
        ensure!(digits.len() <= 6, "秒的小数最多 6 位");
        integer(digits)? * 10u64.pow(6 - digits.len() as u32)
    } else {
        0
    };
    let frame = ((u128::from(whole) * 1_000_000 + u128::from(micros)) * u128::from(rate) + 500_000)
        / 1_000_000;
    ensure!(
        frame <= u128::from(limit),
        "时间超过音轨结尾 {}",
        stamp(limit, rate)
    );
    Ok(frame as u64)
}

pub fn stamp(frame: u64, rate: u32) -> String {
    let us = (u128::from(frame) * 1_000_000 + u128::from(rate / 2)) / u128::from(rate);
    let fraction = us % 1_000_000;
    let clock = format!("{:02}:{:02}", us / 60_000_000, us / 1_000_000 % 60);
    if fraction % 1_000 == 0 {
        format!("{clock}.{:03}", fraction / 1_000)
    } else {
        format!("{clock}.{fraction:06}")
            .trim_end_matches('0')
            .to_owned()
    }
}

pub fn time_span(start: &str, end: &str, rate: u32, frames: u64) -> Result<Span> {
    let start = time_frame(start, rate, frames)?;
    let end = time_frame(end, rate, frames)?;
    ensure!(start < end, "结束时间必须晚于开始时间");
    Ok(start..end)
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub keep: Vec<Span>,
    pub frames: u64,
}

impl Plan {
    pub fn from_marks(total: u64, marks: &[Span]) -> Result<Self> {
        let mut sorted = marks.to_vec();
        sorted.sort_unstable_by_key(|r| r.start);
        let mut cursor = 0;
        let mut keep = Vec::new();
        for mark in sorted {
            ensure!(mark.start < mark.end && mark.end <= total, "标记范围无效");
            if cursor < mark.start {
                keep.push(cursor..mark.start);
            }
            cursor = cursor.max(mark.end);
        }
        if cursor < total {
            keep.push(cursor..total);
        }
        let frames = keep.iter().map(|r| r.end - r.start).sum();
        Ok(Self { keep, frames })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_coordinates_survive_overlapping_adjacent_and_nested_marks() {
        let plan =
            Plan::from_marks(150, &[80..100, 20..35, 35..40, 19..25, 22..24, 140..150]).unwrap();
        assert_eq!(plan.keep, [0..19, 40..80, 100..140]);
        assert_eq!(plan.frames, 99);
    }
    #[test]
    fn time_input_roundtrips_frames_and_rejects_invalid_ranges() {
        assert_eq!(time_frame("1:30:00.5", 2, 20000).unwrap(), 10801);
        assert_eq!(time_frame("31:15", 100, 200000).unwrap(), 187500);
        for rate in [8000, 44100, 48000, 192000] {
            for frame in [0, 1, 333, 123456789] {
                assert_eq!(time_frame(&stamp(frame, rate), rate, frame).unwrap(), frame);
            }
        }
        for invalid in ["", "3.", "0:60", "-2", "0.1234567", "NaN", "01:63:12", "10"] {
            assert!(time_frame(invalid, 100, 999).is_err(), "{invalid}");
        }
        assert!(time_span("1", "0", 100, 999).is_err());
        assert!(time_span("0", "0", 100, 999).is_err());
    }
}
