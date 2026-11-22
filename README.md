# 声剪

用 Rust 编写的单音轨播客剪辑器。每个【剪辑】标记表示删除原录音中的一段，保留的音频按原顺序拼接。桌面版与命令行共用按采样帧计算的剪辑引擎。

## 在 Windows 上使用

双击 [podcast-editor.exe](dist/podcast-editor.exe) 即可打开编辑器，同一个程序也支持下方的命令行调用。已有的 [podcast-studio.exe](dist/podcast-studio.exe) 也可以打开桌面界面。可以先拖入工程附带的[人声样本](samples/jfk.wav)试剪。

1. 导入录音后，波形显示完整的原始音轨。单击波形可定位播放头，空格可播放或暂停。
2. 将鼠标放在要细看的位置，按住 Ctrl 向上滚动可放大时间轴，向下滚动可缩小。「显示全轨」可恢复全段视图。在波形上左右拖选，或在右侧输入起止时间，再点击「试听选区」检查边界。
3. 点击「添加剪辑标记」，已标记的片段以橙色显示。重复选区和添加操作即可标记多段。点击标记列表中的时间范围可以定位到原片段。
4. 点击「处理所有标记」生成结果音轨。可切换「原始音轨」和「剪辑结果」试听对比。修改标记后重新处理即可。
5. 点击「导出 WAV…」指定目录和文件名。标记保存在当前会话中，重新导入音频或关闭窗口会结束本次编辑。

所有标记都相对于原录音。重叠和相接的部分合并处理，仅拖选而未添加标记的片段会保留。起点包含在删除范围内，终点是第一帧保留的音频。

## 命令行

在工程目录打开 PowerShell。查看录音时长，然后删除第 30 分钟到第 31 分 15 秒的片段：

```powershell
.\dist\podcast-editor.exe info "访谈.mp3"
.\dist\podcast-editor.exe cut "访谈.mp3" --cut "30:00~31:15" -o "访谈-剪辑版.wav"
```

用多次 `--cut` 传入多个标记。下面的示例可以直接用随附录音运行：

```powershell
.\dist\podcast-editor.exe cut ".\samples\jfk.wav" --cut "2~4" --cut "8.25~9" -o "试听.wav"
```

时间支持秒、`分:秒`、`时:分:秒`，秒可带最多六位小数，边界就近取整到采样帧。命令成功返回 0，失败返回非零。替换已有输出文件需显式传入 `--overwrite`。输出路径须与原音频不同。

## 音频与源码

支持常见 WAV、MP3、FLAC、M4A（AAC、ALAC）、AAC、Ogg Vorbis 和 AIFF 录音。导出格式固定为 16 位 PCM WAV，沿用导入音轨的采样率和声道数。时间轴以导入后的音频为准。常规 AAC/M4A 的单段播放时长和编码延迟按容器记录校正。

导入时分块解码到系统临时目录。以 48 kHz 双声道为例，每小时录音约需 660 MiB 临时空间。标准 WAV 的单文件大小上限约 4 GiB。

界面使用 egui/eframe，解码使用 Symphonia，试听使用 Rodio。[egui](https://github.com/emilk/egui) · [Symphonia](https://github.com/pdeljanov/Symphonia) · [Rodio](https://github.com/RustAudio/rodio)

使用 Rust 1.97.1 和 Windows x64 MSVC 工具链构建：

```powershell
cargo build --release --locked
cargo run --release --bin podcast-studio -- ".\samples\jfk.wav"
```

## 自测

实测使用 whisper.cpp 提供的 11 秒 JFK 人声录音，原始格式为 16 kHz 单声道 PCM WAV。[样本来源](https://github.com/ggml-org/whisper.cpp/blob/master/samples/jfk.wav)

| 场景 | 检查结果 |
| --- | --- |
| 桌面导入、鼠标拖选、手输时间、添加两段标记、处理、试听、指定中文文件名导出 | 剪去 2.432 秒，导出 8.568 秒。全部 137,088 帧与原录音应保留的样本一致。[实际导出](samples/人声剪辑自测.wav) |
| 命令行处理乱序及重叠标记 | 剪去 4.5 秒，导出 6.5 秒，全部 104,000 帧一致 |
| 真人录音循环扩成 32 分钟，左右声道使用不同样本，剪去 `30:00~31:15` | 结果 30 分 45 秒，两个声道全部 29,520,000 帧一致 |

可用 Python 3 标准库独立复核实际导出的 WAV，长样本在测试结束后自动清理：

```powershell
cargo test --release --locked
python tests/verify_audio.py .\dist\podcast-editor.exe --long
```
