use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use podcast_editor::{
    audio::{Track, processed},
    edit::{stamp, time_span},
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "声剪 · 播客剪辑编辑器",
    after_help = "双击或不带参数运行即可打开桌面编辑器。"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 查看完整音轨的时长、采样率、声道数和帧数
    Info { input: PathBuf },
    /// 删除原时间轴上的所有剪辑标记并导出 WAV
    Cut {
        input: PathBuf,
        /// 原音轨上的 开始~结束，可重复传入
        #[arg(long = "cut", required = true, value_name = "START~END")]
        marks: Vec<String>,
        /// 输出文件名，以 .wav 结尾
        #[arg(short, long)]
        output: PathBuf,
        /// 允许替换已有的输出文件
        #[arg(long)]
        overwrite: bool,
    },
}

fn run(command: Command) -> Result<()> {
    match command {
        Command::Info { input } => {
            let track = Track::open(&input)?;
            println!(
                "{}  |  {} Hz  |  {} 声道  |  {} 帧",
                stamp(track.frames, track.rate),
                track.rate,
                track.channels,
                track.frames
            );
        }
        Command::Cut {
            input,
            marks,
            output,
            overwrite,
        } => {
            let track = Track::open(&input)?;
            let spans = marks
                .iter()
                .map(|arg| {
                    let (start, end) = arg
                        .split_once('~')
                        .context("剪辑标记需要用 ~ 分隔开始和结束时间")?;
                    time_span(start, end, track.rate, track.frames)
                        .with_context(|| format!("无效标记 {arg}"))
                })
                .collect::<Result<Vec<_>>>()?;
            let plan = processed(track.frames, &spans)?;
            track.export(&plan, &output, overwrite)?;
            println!(
                "已处理 {} 个标记，剪去 {}，保留 {}（{} 帧）\n{}",
                spans.len(),
                stamp(track.frames - plan.frames, track.rate),
                stamp(plan.frames, track.rate),
                plan.frames,
                output.display()
            );
        }
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    let Some(command) = Cli::parse().command else {
        // Explorer creates a console for this binary. Release it before opening the GUI.
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Console::FreeConsole();
        }
        return podcast_editor::run_studio(None);
    };
    match run(command) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("错误：{error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
