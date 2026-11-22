use crate::edit::{Plan, Span};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use symphonia::core::{
    audio::SampleBuffer, codecs::CODEC_TYPE_AAC, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, probe::Hint,
};
use tempfile::NamedTempFile;

pub struct Track {
    pub path: PathBuf,
    pub rate: u32,
    pub channels: u16,
    pub frames: u64,
    first_frame: u64,
    pcm: NamedTempFile,
}

pub(crate) fn peak_stride(frames: u64) -> u64 {
    frames.div_ceil(2048).max(1)
}

impl Track {
    pub fn open(path: &Path) -> Result<Self> {
        let path = path
            .canonicalize()
            .with_context(|| format!("找不到 {}", path.display()))?;
        let stream = MediaSourceStream::new(Box::new(File::open(&path)?), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            hint.with_extension(ext);
        }
        let options = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let mut format = symphonia::default::get_probe()
            .format(&hint, stream, &options, &Default::default())
            .context("无法识别音频格式")?
            .format;
        let (id, mut decoder) = format
            .tracks()
            .iter()
            .find_map(|track| {
                symphonia::default::get_codecs()
                    .make(&track.codec_params, &Default::default())
                    .ok()
                    .map(|decoder| (track.id, decoder))
            })
            .context("文件中没有支持解码的音轨")?;
        let mut pcm = NamedTempFile::new().context("无法创建音频缓存")?;
        let mut writer = BufWriter::with_capacity(128 * 1024, pcm.as_file_mut());
        let mut signal = None;
        let mut frames = 0;
        let mut samples: Option<SampleBuffer<i16>> = None;
        let mut bytes = Vec::new();
        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e).context("读取音频包失败"),
            };
            if packet.track_id() != id {
                continue;
            }
            let decoded = decoder.decode(&packet).context("音频数据解码失败")?;
            let spec = *decoded.spec();
            ensure!(spec.rate > 0 && spec.channels.count() > 0, "音轨参数无效");
            ensure!(
                *signal.get_or_insert(spec) == spec,
                "音轨中途改变了采样率或声道"
            );
            let count = decoded.frames();
            if count == 0 {
                continue;
            }
            if samples
                .as_ref()
                .is_none_or(|b| b.capacity() < count * spec.channels.count())
            {
                samples = Some(SampleBuffer::new(count as u64, spec));
            }
            let buffer = samples.as_mut().unwrap();
            buffer.copy_interleaved_ref(decoded);
            bytes.clear();
            bytes.extend(buffer.samples().iter().flat_map(|s| s.to_le_bytes()));
            writer
                .write_all(&bytes)
                .context("写入音频缓存失败，请检查临时目录可用空间")?;
            frames += count as u64;
        }
        writer.flush()?;
        drop(writer);
        ensure!(frames > 0, "音轨为空");
        let spec = signal.context("缺少音轨参数")?;
        // AAC packets in MP4 often include encoder priming and a padded final packet.
        // The container specifies which decoded samples belong on the presentation timeline.
        let visible = if decoder.codec_params().codec == CODEC_TYPE_AAC {
            aac_presentation(&path, id, spec.rate, frames)?.unwrap_or(0..frames)
        } else {
            0..frames
        };
        Ok(Self {
            path,
            rate: spec.rate,
            channels: spec.channels.count() as u16,
            first_frame: visible.start,
            frames: visible.end - visible.start,
            pcm,
        })
    }

    pub fn original(&self) -> Plan {
        Plan {
            keep: std::iter::once(0..self.frames).collect(),
            frames: self.frames,
        }
    }

    /// Read a result timeline directly from the cached original, including after a seek.
    pub fn reader(&self, plan: &Plan, mut start: u64) -> Result<BufReader<SegmentReader>> {
        ensure!(start <= plan.frames, "播放位置超出结果音轨");
        let stride = u64::from(self.channels) * 2;
        let mut spans = VecDeque::new();
        for span in &plan.keep {
            let length = span.end - span.start;
            if start >= length {
                start -= length;
            } else {
                spans.push_back(
                    (self.first_frame + span.start + start) * stride
                        ..(self.first_frame + span.end) * stride,
                );
                start = 0;
            }
        }
        Ok(BufReader::with_capacity(
            64 * 1024,
            SegmentReader {
                file: self.pcm.reopen()?,
                spans,
                remaining: 0,
            },
        ))
    }

    pub fn waveform(&self, plan: &Plan) -> Result<Vec<(i16, i16)>> {
        self.waveform_range(plan, 0..plan.frames, peak_stride(plan.frames))
    }

    pub(crate) fn waveform_range(
        &self,
        plan: &Plan,
        range: Span,
        bucket_frames: u64,
    ) -> Result<Vec<(i16, i16)>> {
        ensure!(
            range.start <= range.end && range.end <= plan.frames && bucket_frames > 0,
            "波形范围无效"
        );
        let mut peaks = Vec::new();
        let mut peak = (0i16, 0i16);
        let mut in_bucket = 0;
        let mut reader = self.reader(plan, range.start)?;
        let stride = usize::from(self.channels) * 2;
        let mut buf = vec![0; stride * 4096];
        let mut remaining = range.end - range.start;
        while remaining > 0 {
            let n = remaining.min(4096);
            let block = &mut buf[..n as usize * stride];
            reader.read_exact(block)?;
            for frame in block.chunks_exact(stride) {
                for sample in frame.chunks_exact(2) {
                    let s = i16::from_le_bytes([sample[0], sample[1]]);
                    peak = (peak.0.min(s), peak.1.max(s));
                }
                in_bucket += 1;
                if in_bucket == bucket_frames {
                    peaks.push(peak);
                    peak = (0, 0);
                    in_bucket = 0;
                }
            }
            remaining -= n;
        }
        if in_bucket > 0 {
            peaks.push(peak);
        }
        Ok(peaks)
    }

    pub fn export(&self, plan: &Plan, path: &Path, overwrite: bool) -> Result<()> {
        ensure!(
            plan.frames > 0,
            "所有音频都被标记为剪辑，请至少保留一段音频"
        );
        ensure!(
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("wav")),
            "导出文件请使用 .wav 扩展名"
        );
        if path.try_exists()? {
            ensure!(
                !same_file::is_same_file(&self.path, path)?,
                "请选择不同于原音频的文件名"
            );
            ensure!(
                overwrite,
                "输出文件已存在，请换一个文件名或使用 --overwrite"
            );
        }
        let samples = plan.frames * u64::from(self.channels);
        ensure!(
            samples * 2 + 68 <= u64::from(u32::MAX),
            "结果超过标准 WAV 的 4 GiB 上限"
        );
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut tmp = NamedTempFile::new_in(parent).context("无法在输出目录创建文件")?;
        let spec = hound::WavSpec {
            channels: self.channels,
            sample_rate: self.rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut reader = self.reader(plan, 0)?;
        {
            let mut out = BufWriter::new(tmp.as_file_mut());
            let mut wav = hound::WavWriter::new(&mut out, spec)?;
            let mut remaining = samples;
            let mut buf = vec![0; 64 * 1024];
            while remaining > 0 {
                let n = remaining.min(buf.len() as u64 / 2) as usize;
                reader.read_exact(&mut buf[..2 * n])?;
                let mut block = wav.get_i16_writer(n as u32);
                for b in buf[..2 * n].chunks_exact(2) {
                    block.write_sample(i16::from_le_bytes([b[0], b[1]]));
                }
                block.flush()?;
                remaining -= n as u64;
            }
            wav.finalize()?;
            out.flush()?;
        }
        tmp.as_file().sync_all()?;
        if overwrite {
            tmp.persist(path)?;
        } else {
            tmp.persist_noclobber(path)?;
        }
        Ok(())
    }
}

fn aac_presentation(path: &Path, track_index: u32, rate: u32, frames: u64) -> Result<Option<Span>> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let mut header = [0; 8];
    if reader.read_exact(&mut header).is_err() || &header[4..] != b"ftyp" {
        return Ok(None);
    }
    reader.rewind()?;
    let Ok(mp4) = mp4::Mp4Reader::read_header(reader, size) else {
        return Ok(None);
    };
    let Some(track) = mp4.moov.traks.get(track_index as usize) else {
        return Ok(None);
    };
    let Some(list) = track.edts.as_ref().and_then(|e| e.elst.as_ref()) else {
        return Ok(None);
    };
    let [edit] = list.entries.as_slice() else {
        return Ok(None);
    };
    if edit.media_rate != 1 || edit.media_rate_fraction != 0 || edit.segment_duration == 0 {
        return Ok(None);
    }
    let scale = |ticks: u64, timescale: u32| -> Result<u64> {
        ensure!(timescale > 0, "M4A 时间刻度无效");
        let n = (u128::from(ticks) * u128::from(rate) + u128::from(timescale / 2))
            / u128::from(timescale);
        u64::try_from(n).context("M4A 时间超出范围")
    };
    let media = &track.mdia.mdhd;
    let start = scale(edit.media_time, media.timescale)?;
    let len = scale(edit.segment_duration, mp4.timescale())?;
    let end = start.saturating_add(len).min(frames);
    let end = if media.duration > 0 {
        end.min(scale(media.duration, media.timescale)?)
    } else {
        end
    };
    Ok((start < end).then_some(start..end))
}

pub struct SegmentReader {
    file: File,
    spans: VecDeque<Span>,
    remaining: u64,
}

impl Read for SegmentReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let Some(span) = self.spans.pop_front() else {
                return Ok(0);
            };
            self.file.seek(SeekFrom::Start(span.start))?;
            self.remaining = span.end - span.start;
        }
        let len = self.remaining.min(buf.len() as u64) as usize;
        let n = self.file.read(&mut buf[..len])?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}

pub fn processed(total: u64, marks: &[Span]) -> Result<Plan> {
    ensure!(!marks.is_empty(), "请先添加剪辑标记");
    let plan = Plan::from_marks(total, marks)?;
    if plan.frames == 0 {
        bail!("这些标记会删除整条音轨，请至少保留一段音频");
    }
    Ok(plan)
}
