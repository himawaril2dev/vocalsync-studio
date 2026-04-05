"""
音高資料模型 — 頻率與音符之間的轉換，以及音高軌跡管理。
"""

from __future__ import annotations

import math
from dataclasses import dataclass

# A4 = 440 Hz，MIDI note 69
_A4_FREQ = 440.0
_A4_MIDI = 69

_NOTE_NAMES = ("C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B")


@dataclass(frozen=True, slots=True)
class PitchSample:
    """單一時間點的音高偵測結果。"""
    timestamp_sec: float
    frequency_hz: float
    confidence: float
    note_name: str
    octave: int
    cent_offset: int  # -50 ~ +50


class PitchTrack:
    """管理一串 PitchSample 的時間序列。"""

    def __init__(self) -> None:
        self._samples: list[PitchSample] = []

    @property
    def samples(self) -> list[PitchSample]:
        """回傳副本，保護內部資料不被外部修改。"""
        return list(self._samples)

    def append(self, sample: PitchSample) -> None:
        self._samples.append(sample)

    def clear(self) -> None:
        self._samples.clear()

    def get_range(self, start_sec: float, end_sec: float) -> list[PitchSample]:
        return [s for s in self._samples
                if start_sec <= s.timestamp_sec <= end_sec]

    def __len__(self) -> int:
        return len(self._samples)


def freq_to_note(hz: float) -> tuple[str, int, int]:
    """
    將頻率轉換為 (音符名稱, 八度, cent 偏移)。

    >>> freq_to_note(440.0)
    ('A', 4, 0)
    >>> freq_to_note(261.63)
    ('C', 4, 0)
    """
    if hz <= 0:
        return ("--", 0, 0)

    midi = _A4_MIDI + 12 * math.log2(hz / _A4_FREQ)
    midi_rounded = round(midi)
    cent = round((midi - midi_rounded) * 100)

    note_index = midi_rounded % 12
    octave = (midi_rounded // 12) - 1

    return (_NOTE_NAMES[note_index], octave, cent)


def freq_to_midi(hz: float) -> float:
    """將頻率轉換為 MIDI note number（連續值）。"""
    if hz <= 0:
        return 0.0
    return _A4_MIDI + 12 * math.log2(hz / _A4_FREQ)
