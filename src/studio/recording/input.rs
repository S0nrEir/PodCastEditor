use crate::audio::export_pcm;
use anyhow::{Context, Result, anyhow, bail, ensure};
use cpal::{
    FromSample, Sample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering::Relaxed},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
};
use tempfile::NamedTempFile;

pub(super) struct Draft {
    pcm: NamedTempFile,
    pub rate: u32,
    pub channels: u16,
    frames: Arc<AtomicU64>,
}

impl Draft {
    pub(super) fn new(rate: u32, channels: u16) -> Result<Self> {
        Ok(Self {
            pcm: NamedTempFile::new().context("无法创建录音临时文件")?,
            rate,
            channels,
            frames: Default::default(),
        })
    }

    pub fn frames(&self) -> u64 {
        self.frames.load(Relaxed)
    }

    // Only the writer touches the tail. Exports read exactly the committed frames,
    // including when an interrupted disk write left a partial block behind.
    fn writer(&self) -> Result<(File, u64)> {
        let frames = self.frames();
        let mut file = self.pcm.reopen()?;
        file.seek(SeekFrom::Start(frames * u64::from(self.channels) * 2))?;
        Ok((file, frames))
    }

    pub fn export(&self, path: &Path, overwrite: bool) -> Result<()> {
        ensure!(self.frames() > 0, "尚未录到声音");
        export_pcm(
            self.pcm.reopen()?,
            self.rate,
            self.channels,
            self.frames(),
            path,
            overwrite,
        )
    }
}

struct Sink {
    blocks: SyncSender<Vec<u8>>,
    errors: SyncSender<String>,
    peak: Arc<AtomicU32>,
    failed: bool,
}

impl Sink {
    fn receive<T: Sample>(&mut self, data: &[T])
    where
        i16: FromSample<T>,
    {
        if self.failed || data.is_empty() {
            return;
        }
        let mut bytes = Vec::with_capacity(data.len() * 2);
        let mut peak = 0;
        for &s in data {
            let value = i16::from_sample(s);
            peak = peak.max(value.unsigned_abs());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        self.peak.store(u32::from(peak), Relaxed);
        if self.blocks.try_send(bytes).is_err() {
            self.failed = true;
            let _ = self
                .errors
                .try_send("录音写入中断，请检查临时目录的空间和磁盘速度".into());
        }
    }
}

fn input_stream<T: SizedSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut sink: Sink,
) -> Result<cpal::Stream>
where
    i16: FromSample<T>,
{
    let errors = sink.errors.clone();
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| sink.receive(data),
        move |e| {
            let _ = errors.try_send(format!("麦克风中断：{e}"));
        },
        None,
    )?)
}

fn write_blocks(
    mut file: File,
    mut frames: u64,
    channels: u16,
    progress: &AtomicU64,
    blocks: Receiver<Vec<u8>>,
) -> Result<()> {
    let stride = usize::from(channels) * 2;
    for block in blocks {
        ensure!(block.len() % stride == 0, "麦克风返回了不完整的音频帧");
        let end = frames + (block.len() / stride) as u64;
        ensure!(
            end * stride as u64 + 68 <= u64::from(u32::MAX),
            "已达到 WAV 的大小上限，请导出本次录音"
        );
        file.write_all(&block).context("写入录音临时文件失败")?;
        frames = end;
        progress.store(frames, Relaxed);
    }
    Ok(())
}

pub(super) struct Capture {
    stream: cpal::Stream,
    writer: JoinHandle<Result<()>>,
    errors: Receiver<String>,
    peak: Arc<AtomicU32>,
    pub device_name: String,
}

impl Capture {
    pub fn start(existing: Option<&Arc<Draft>>) -> Result<(Self, Arc<Draft>)> {
        let device = cpal::default_host()
            .default_input_device()
            .context("找不到默认麦克风，请连接设备并在系统声音设置中选择输入设备")?;
        let supported = device
            .default_input_config()
            .context("无法读取麦克风设置，请检查设备及麦克风权限")?;
        let config = supported.config();
        let draft = match existing {
            Some(d) if d.frames() > 0 => {
                ensure!(
                    d.rate == config.sample_rate.0 && d.channels == config.channels,
                    "麦克风格式已改变，请先导出当前录音"
                );
                Arc::clone(d)
            }
            _ => Arc::new(Draft::new(config.sample_rate.0, config.channels)?),
        };
        let (file, start) = draft.writer()?;
        let (blocks, rx) = mpsc::sync_channel(64);
        let (errors, error_rx) = mpsc::sync_channel(1);
        let peak = Arc::new(AtomicU32::new(0));
        let sink = Sink {
            blocks,
            errors,
            peak: peak.clone(),
            failed: false,
        };
        macro_rules! stream {
            ($($variant:ident => $ty:ty),+) => {
                match supported.sample_format() {
                    $(SampleFormat::$variant => input_stream::<$ty>(&device, &config, sink),)+
                    other => bail!("暂不支持麦克风的采样格式 {other}"),
                }
            }
        }
        let stream = stream!(I8=>i8, I16=>i16, I32=>i32, I64=>i64, U8=>u8, U16=>u16, U32=>u32, U64=>u64, F32=>f32, F64=>f64)
            .context("无法打开麦克风，请检查系统的麦克风访问权限")?;
        let progress = draft.frames.clone();
        let writer = thread::Builder::new()
            .name("recording-writer".into())
            .spawn(move || write_blocks(file, start, config.channels, &progress, rx))?;
        if let Err(e) = stream.play() {
            drop(stream);
            let _ = writer.join();
            return Err(e).context("麦克风无法开始录制");
        }
        let device_name = device.name().unwrap_or_else(|_| "默认麦克风".into());
        Ok((
            Self {
                stream,
                writer,
                errors: error_rx,
                peak,
                device_name,
            },
            draft,
        ))
    }

    pub fn level(&self) -> f32 {
        self.peak.load(Relaxed) as f32 / 32768.
    }

    pub fn interrupted(&self) -> bool {
        self.writer.is_finished()
    }

    pub fn error(&self) -> Option<String> {
        self.errors.try_recv().ok()
    }

    pub fn stop(self) -> Result<()> {
        drop(self.stream);
        self.writer
            .join()
            .map_err(|_| anyhow!("录音写入线程意外退出"))??;
        if let Ok(e) = self.errors.try_recv() {
            bail!(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::recording::Recording;
    use eframe::egui::Context;
    use std::time::{Duration, Instant};

    #[test]
    fn two_takes_retry_export_and_verify_every_sample() {
        let draft = Arc::new(Draft::new(48000, 2).unwrap());
        for take in [[-1.0f32, 0.5, 0.0, -0.5], [0.25, 0.75, -0.75, 0.0]] {
            let (blocks, rx) = mpsc::sync_channel(2);
            let (errors, _) = mpsc::sync_channel(1);
            let mut sink = Sink {
                blocks,
                errors,
                peak: Default::default(),
                failed: false,
            };
            sink.receive(&take);
            drop(sink);
            let (file, start) = draft.writer().unwrap();
            write_blocks(file, start, draft.channels, &draft.frames, rx).unwrap();
        }
        let ctx = Context::default();
        let mut page = Recording {
            draft: Some(draft),
            ..Default::default()
        };
        assert_eq!(page.draft.as_ref().unwrap().frames(), 4);
        page.finish(&ctx, |_| None);
        assert!(!page.exporting() && page.has_audio());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("验收.wav");
        std::fs::write(&path, b"already exists").unwrap();
        for overwrite in [false, true] {
            page.finish(&ctx, |_| Some((path.clone(), overwrite)));
            let deadline = Instant::now() + Duration::from_secs(5);
            while page.exporting() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
                page.poll();
            }
            assert!(!page.exporting());
            assert_eq!(page.has_audio(), !overwrite);
            if !overwrite {
                assert_eq!(std::fs::read(&path).unwrap(), b"already exists");
            }
        }
        let mut wav = hound::WavReader::open(path).unwrap();
        assert_eq!(
            (
                wav.spec().sample_rate,
                wav.spec().channels,
                wav.spec().bits_per_sample,
                wav.duration()
            ),
            (48000, 2, 16, 4)
        );
        assert_eq!(
            wav.samples::<i16>()
                .collect::<hound::Result<Vec<_>>>()
                .unwrap(),
            [-32768, 16384, 0, -16384, 8192, 24576, -24576, 0]
        );
    }
}
