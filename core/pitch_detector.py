"""
音高偵測引擎 — 基於 YIN 演算法的純 numpy 實作。

參考：De Cheveigné, A., & Kawahara, H. (2002).
      YIN, a fundamental frequency estimator for speech and music.
      The Journal of the Acoustical Society of America, 111(4), 1917-1930.

實作參考：Patrice Guyot (MIT License)
https://github.com/patriceguyot/Yin
"""

from __future__ import annotations

import numpy as np

from core.pitch_data import PitchSample, freq_to_note


class PitchDetector:
    """
    即時音高偵測器。

    每次呼叫 detect() 餵入一段音訊 chunk，回傳偵測到的 PitchSample 或 None。
    使用 YIN 演算法，純 numpy 實作，不需要額外依賴。
    """

    def __init__(
        self,
        samplerate: int = 44100,
        buf_size: int = 2048,
        f0_min: float = 80.0,
        f0_max: float = 1000.0,
        harmo_thresh: float = 0.15,
        rms_thresh: float = 0.005,
    ) -> None:
        self._sr = samplerate
        self._buf_size = buf_size
        self._tau_min = max(2, int(samplerate / f0_max))
        self._tau_max = min(buf_size, int(samplerate / f0_min))
        self._harmo_thresh = harmo_thresh
        self._rms_thresh = rms_thresh

    def detect(self, audio: np.ndarray, timestamp: float = 0.0) -> PitchSample | None:
        """
        偵測音訊 chunk 的基頻。

        Parameters
        ----------
        audio : np.ndarray
            單聲道音訊資料（float64 or float32），長度應 >= buf_size。
        timestamp : float
            此 chunk 的時間戳記（秒）。

        Returns
        -------
        PitchSample | None
            偵測到的音高，靜音或無法判定時回傳 None。
        """
        if len(audio) < self._buf_size:
            return None

        mono = audio[:self._buf_size].astype(np.float64)

        # 靜音偵測
        rms = np.sqrt(np.mean(mono ** 2))
        if rms < self._rms_thresh:
            return None

        # YIN 差分函數
        df = _difference_function(mono, self._buf_size, self._tau_max)

        # 累積平均正規化差分函數 (CMNDF)
        cmndf = _cmndf(df, self._tau_max)

        # 取得基頻週期
        tau = _get_pitch_period(cmndf, self._tau_min, self._tau_max, self._harmo_thresh)
        if tau == 0:
            return None

        # 拋物線插值提高精度
        tau_refined = _parabolic_interpolation(cmndf, tau)
        freq = self._sr / tau_refined

        # 計算信心度（1 - CMNDF 值，越小越好）
        confidence = 1.0 - float(cmndf[tau])

        note_name, octave, cent = freq_to_note(freq)

        return PitchSample(
            timestamp_sec=timestamp,
            frequency_hz=round(freq, 1),
            confidence=round(confidence, 3),
            note_name=note_name,
            octave=octave,
            cent_offset=cent,
        )


def _difference_function(x: np.ndarray, n: int, tau_max: int) -> np.ndarray:
    """
    計算 YIN 差分函數（equation 6）。

    使用 FFT 加速，時間複雜度 O(n log n)。
    """
    tau_max = min(tau_max, n)
    x_cumsum = np.concatenate((np.array([0.0]), np.cumsum(x ** 2)))

    size = n + tau_max
    p2 = (size // 32).bit_length()
    nice_numbers = (16, 18, 20, 24, 25, 27, 30, 32)
    size_pad = min(v * (1 << p2) for v in nice_numbers if v * (1 << p2) >= size)

    fc = np.fft.rfft(x, size_pad)
    conv = np.fft.irfft(fc * fc.conjugate())[:tau_max]

    return x_cumsum[n:n - tau_max:-1] + x_cumsum[n] - x_cumsum[:tau_max] - 2 * conv


def _cmndf(df: np.ndarray, tau_max: int) -> np.ndarray:
    """累積平均正規化差分函數 (equation 8)。"""
    cmndf = np.zeros(tau_max)
    cmndf[0] = 1.0
    cumsum = np.cumsum(df[1:])
    # 避免除以零
    safe_cumsum = np.where(cumsum == 0, 1e-10, cumsum)
    cmndf[1:] = df[1:] * np.arange(1, tau_max) / safe_cumsum
    return cmndf


def _get_pitch_period(
    cmndf: np.ndarray, tau_min: int, tau_max: int, threshold: float
) -> int:
    """
    從 CMNDF 中找到基頻週期。

    回傳基頻週期（samples），若為無聲/不確定則回傳 0。
    """
    tau = tau_min
    while tau < tau_max:
        if cmndf[tau] < threshold:
            while tau + 1 < tau_max and cmndf[tau + 1] < cmndf[tau]:
                tau += 1
            return tau
        tau += 1
    return 0


def _parabolic_interpolation(cmndf: np.ndarray, tau: int) -> float:
    """拋物線插值，提高頻率估計精度。"""
    if tau < 1 or tau >= len(cmndf) - 1:
        return float(tau)

    s0 = cmndf[tau - 1]
    s1 = cmndf[tau]
    s2 = cmndf[tau + 1]

    denom = 2.0 * (2.0 * s1 - s2 - s0)
    if abs(denom) < 1e-10:
        return float(tau)

    return tau + (s2 - s0) / denom
