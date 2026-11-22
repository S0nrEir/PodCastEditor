use crate::{
    audio::{SegmentReader, Track},
    edit::Plan,
};
use anyhow::{Context, Result};
use rodio::{OutputStream, Sink, Source};
use std::{
    io::{BufReader, Read},
    time::Duration,
};

struct PcmSource {
    reader: BufReader<SegmentReader>,
    remaining: u64,
    rate: u32,
    channels: u16,
    duration: Duration,
}

impl Iterator for PcmSource {
    type Item = i16;
    fn next(&mut self) -> Option<i16> {
        if self.remaining == 0 {
            return None;
        }
        let mut b = [0; 2];
        self.reader.read_exact(&mut b).ok()?;
        self.remaining -= 1;
        Some(i16::from_le_bytes(b))
    }
}

impl Source for PcmSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(self.duration)
    }
}

pub struct Playback {
    sink: Sink,
    _stream: OutputStream,
    start: u64,
    end: u64,
    rate: u32,
}

impl Playback {
    pub fn play(track: &Track, plan: &Plan, start: u64, end: u64) -> Result<Self> {
        let source = PcmSource {
            reader: track.reader(plan, start)?,
            remaining: (end - start) * u64::from(track.channels),
            rate: track.rate,
            channels: track.channels,
            duration: Duration::from_secs_f64((end - start) as f64 / f64::from(track.rate)),
        };
        let (stream, handle) = OutputStream::try_default().context("无法打开默认播放设备")?;
        let sink = Sink::try_new(&handle).context("无法开始试听")?;
        sink.append(source);
        Ok(Self {
            sink,
            _stream: stream,
            start,
            end,
            rate: track.rate,
        })
    }
    pub fn position(&self) -> u64 {
        if self.finished() {
            self.end
        } else {
            (self.start + (self.sink.get_pos().as_secs_f64() * f64::from(self.rate)) as u64)
                .min(self.end)
        }
    }
    pub fn finished(&self) -> bool {
        self.sink.empty()
    }
}
