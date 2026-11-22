"""Black-box checks using the shipped speech recording and Python's WAV reader."""

import argparse
from pathlib import Path
import subprocess
import tempfile
import wave


ROOT = Path(__file__).resolve().parents[1]
SPEECH = ROOT / "samples" / "jfk.wav"


def verify(source, result, keep):
    with wave.open(str(source), "rb") as a, wave.open(str(result), "rb") as b:
        assert a.getparams()[:3] == b.getparams()[:3], "audio parameters differ"
        assert b.getnframes() == sum(end - start for start, end in keep)
        for start, end in keep:
            a.setpos(start)
            while start < end:
                count = min(65536, end - start)
                assert a.readframes(count) == b.readframes(count), f"PCM mismatch at {start}"
                start += count
        print(f"{result.name}: all {b.getnframes():,} frames match")


def long_fixture(path):
    with wave.open(str(SPEECH), "rb") as source, wave.open(str(path), "wb") as out:
        n = source.getnframes()
        mono = source.readframes(n)
        stereo = bytearray(n * 4)
        # A delayed right channel also detects channel swaps at the splice.
        stereo[0::4], stereo[1::4] = mono[::2], mono[1::2]
        right = mono[-128:] + mono[:-128]
        stereo[2::4], stereo[3::4] = right[::2], right[1::2]
        rate = source.getframerate()
        out.setparams((2, 2, rate, 0, "NONE", "not compressed"))
        total = 32 * 60 * rate
        repeats, tail = divmod(total, n)
        for _ in range(repeats):
            out.writeframesraw(stereo)
        out.writeframesraw(stereo[:tail * 4])
        return rate, total


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--long", action="store_true")
    args = parser.parse_args()
    exe = args.cli.resolve()
    verify(SPEECH, ROOT / "samples" / "人声剪辑自测.wav",
           [(0, 30599), (52711, 68800), (85600, 176000)])
    with tempfile.TemporaryDirectory(dir=ROOT / "target") as temp:
        temp = Path(temp)
        output = temp / "overlap.wav"
        command = [exe, "cut", SPEECH, "--cut", "3.5~5.75", "--cut", "2~4",
                   "--cut", "8.25~9", "-o", output]
        subprocess.run(command, check=True)
        verify(SPEECH, output, [(0, 32000), (92000, 132000), (144000, 176000)])
        saved = output.read_bytes()
        source_copy = temp / "source.wav"
        source_copy.write_bytes(SPEECH.read_bytes())
        for invalid in [
            command,
            [exe, "cut", source_copy, "--cut", "0~1", "-o", source_copy, "--overwrite"],
            [exe, "cut", SPEECH, "--cut", "0~11", "-o", output, "--overwrite"],
            [exe, "cut", SPEECH, "--cut", "7~20", "-o", output, "--overwrite"],
        ]:
            assert subprocess.run(invalid, capture_output=True).returncode != 0
        assert source_copy.read_bytes() == SPEECH.read_bytes()
        assert output.read_bytes() == saved
        if args.long:
            source = temp / "32min-stereo.wav"
            output = temp / "30min45s.wav"
            rate, total = long_fixture(source)
            subprocess.run([exe, "cut", source, "--cut", "30:00~31:15", "-o", output],
                           check=True)
            verify(source, output, [(0, 1800 * rate), (1875 * rate, total)])


if __name__ == "__main__":
    main()
