"""
錄音核心模組。
處理：伴奏播放、麥克風錄音（同步）、回放、導出三個檔案。
"""

import os
import threading
import time
from collections.abc import Callable

import numpy as np
import sounddevice as sd
import soundfile as sf

from core.pitch_data import PitchSample, PitchTrack
from core.pitch_detector import PitchDetector


class AudioRecorder:
    """
    同步播放伴奏並錄製麥克風的錄音器。

    使用流程：
        recorder = AudioRecorder()
        recorder.load_backing("backing.wav")
        recorder.start_recording(on_progress=..., on_finished=...)
        recorder.stop()
        recorder.start_playback()
        recorder.export("~/Downloads/session1")
    """

    SAMPLERATE = 44100

    def __init__(self):
        self._backing_data: np.ndarray | None = None   # shape: (N, 2) stereo
        self._vocal_chunks: list[np.ndarray] = []
        self._vocal_data: np.ndarray | None = None     # 錄音完成後的完整資料
        self._samplerate = self.SAMPLERATE
        self._stream: sd.Stream | None = None
        self._playback_stream: sd.OutputStream | None = None
        self._playback_pos = 0
        self._record_pos = 0
        self._is_recording = False
        self._is_playing_back = False
        self._duration: float = 0.0
        self._volume: float = 1.0          # 0.0 ~ 2.0
        self._elapsed: float = 0.0         # 目前播放秒數（供影片同步）
        self._mic_rms: float = 0.0         # 麥克風 RMS（供 VU 表）
        self._backing_rms: float = 0.0     # 伴奏輸出 RMS（供 VU 表）
        self._playback_frame_pos: int = 0  # 目前播放幀位置（供暫停/繼續）
        self._playback_gen: int = 0        # 播放世代計數器（防止舊回呼干擾）
        self._lock = threading.Lock()      # 保護跨執行緒共享狀態

        # 音高偵測
        self._pitch_detector = PitchDetector(samplerate=self.SAMPLERATE)
        self._current_pitch: PitchSample | None = None
        self._pitch_track = PitchTrack()
        self._pitch_buf = np.zeros(2048, dtype=np.float64)
        self._pitch_buf_pos: int = 0

    # ------------------------------------------------------------------ #
    #  載入伴奏                                                            #
    # ------------------------------------------------------------------ #

    def load_backing(self, path: str) -> float:
        """
        載入伴奏 WAV 檔案。
        回傳總時長（秒）。
        """
        data, sr = sf.read(path, dtype="float32", always_2d=True)
        if data.shape[1] == 1:
            data = np.column_stack([data, data])   # mono → stereo
        if sr != self.SAMPLERATE:
            # FFT-based 重採樣（品質優於線性插值）
            from numpy.fft import rfft, irfft
            orig_len = len(data)
            target_len = int(orig_len * self.SAMPLERATE / sr)
            resampled_channels = []
            for ch in range(data.shape[1]):
                spectrum = rfft(data[:, ch])
                new_spectrum = np.zeros(target_len // 2 + 1, dtype=spectrum.dtype)
                copy_len = min(len(spectrum), len(new_spectrum))
                new_spectrum[:copy_len] = spectrum[:copy_len]
                resampled = irfft(new_spectrum, n=target_len)
                resampled *= target_len / orig_len  # 補償能量
                resampled_channels.append(resampled.astype(np.float32))
            data = np.column_stack(resampled_channels)
        self._backing_data = data
        self._samplerate = self.SAMPLERATE
        self._duration = len(data) / self.SAMPLERATE
        self._vocal_data = None
        self._vocal_chunks = []
        return self._duration

    # ------------------------------------------------------------------ #
    #  錄音（同步播放伴奏）                                                #
    # ------------------------------------------------------------------ #

    def start_recording(
        self,
        on_progress: Callable[[float, float], None] | None = None,
        on_finished: Callable[[], None] | None = None,
        on_error: Callable[[str], None] | None = None,
        input_device: int | None = None,
        output_device: int | None = None,
    ) -> None:
        """
        開始錄音。在背景執行緒中啟動，立即回傳。
        on_progress(elapsed_sec, total_sec) 每 0.1 秒呼叫一次。
        """
        if self._backing_data is None:
            if on_error:
                on_error("尚未載入伴奏檔案")
            return

        self._vocal_chunks = []
        self._vocal_data = None
        self._playback_pos = 0
        self._elapsed = 0.0
        self._mic_rms = 0.0
        self._backing_rms = 0.0
        self._current_pitch = None
        self._pitch_track.clear()
        self._pitch_buf_pos = 0
        self._is_recording = True

        def callback(indata: np.ndarray, outdata: np.ndarray, frames: int, ts, status):
            # 播放伴奏（套用音量）
            start = self._playback_pos
            end = start + frames
            chunk = self._backing_data[start:end]
            actual = len(chunk)
            out_chunk = chunk * self._volume
            outdata[:actual] = out_chunk
            if actual < frames:
                outdata[actual:] = 0
            self._playback_pos += actual
            self._elapsed = self._playback_pos / self._samplerate

            # RMS 追蹤
            self._mic_rms = float(np.sqrt(np.mean(indata ** 2)))
            self._backing_rms = float(np.sqrt(np.mean(out_chunk ** 2)))

            # 音高偵測（累積到 2048 samples 後執行一次）
            mono = indata[:, 0] if indata.ndim > 1 else indata
            self._feed_pitch(mono, self._elapsed)

            # 錄製麥克風
            self._vocal_chunks.append(indata.copy())

            # 伴奏播完則停止
            if actual < frames:
                raise sd.CallbackStop()

        def run():
            try:
                self._stream = sd.Stream(
                    samplerate=self._samplerate,
                    blocksize=1024,
                    channels=(1, 2),     # input=mono, output=stereo
                    dtype="float32",
                    device=(input_device, output_device),
                    callback=callback,
                    finished_callback=lambda: self._on_record_finished(on_finished),
                )
                self._stream.start()
                start_time = time.time()
                while self._stream.active and self._is_recording:
                    elapsed = time.time() - start_time
                    if on_progress:
                        on_progress(elapsed, self._duration)
                    time.sleep(0.1)
                # 確保完成
                if self._stream.active:
                    self._stream.stop()
            except Exception as e:
                self._is_recording = False
                if on_error:
                    on_error(str(e))

        threading.Thread(target=run, daemon=True).start()

    def stop(self) -> None:
        """停止錄音（使用者手動停止）。"""
        self._is_recording = False
        if self._stream and self._stream.active:
            self._stream.stop()
            self._stream.close()
            self._stream = None
        self._finalize_vocal()

    def _on_record_finished(self, on_finished: Callable | None):
        """錄音串流自然結束（伴奏播完）時呼叫。"""
        self._is_recording = False
        self._finalize_vocal()
        if on_finished:
            on_finished()

    def _finalize_vocal(self) -> None:
        """合併錄音 chunks 為完整資料。以 Lock 保護防止競態條件。"""
        with self._lock:
            if self._vocal_data is not None:
                return
            if self._vocal_chunks:
                raw = np.concatenate(self._vocal_chunks, axis=0)
                target = len(self._backing_data) if self._backing_data is not None else len(raw)
                n = min(len(raw), target)
                self._vocal_data = raw[:n]

    # ------------------------------------------------------------------ #
    #  回放                                                                #
    # ------------------------------------------------------------------ #

    def start_playback(
        self,
        on_progress: Callable[[float, float], None] | None = None,
        on_finished: Callable[[], None] | None = None,
        on_error: Callable[[str], None] | None = None,
        output_device: int | None = None,
        start_frame: int = 0,
    ) -> None:
        """
        播放伴奏（若有錄音則混入人聲）。
        無錄音資料時仍可單純試聽伴奏。
        start_frame: 從指定幀開始播放（用於暫停後繼續）。
        """
        if self._backing_data is None:
            return

        self._is_playing_back = True
        with self._lock:
            self._playback_gen += 1
            gen = self._playback_gen
        self._playback_frame_pos = start_frame
        self._elapsed = start_frame / self._samplerate
        self._backing_rms = 0.0
        pos = [start_frame]

        n = len(self._backing_data)
        if self._vocal_data is not None:
            n = min(n, len(self._vocal_data))
            vocal_stereo = np.column_stack(
                [self._vocal_data[:n, 0], self._vocal_data[:n, 0]])
            mix = np.clip(
                self._backing_data[:n] * self._volume + vocal_stereo, -1.0, 1.0)
        else:
            mix = self._backing_data[:n] * self._volume

        def callback(outdata: np.ndarray, frames: int, ts, status):
            start = pos[0]
            end = start + frames
            chunk = mix[start:end]
            actual = len(chunk)
            outdata[:actual] = chunk
            if actual < frames:
                outdata[actual:] = 0
            pos[0] += actual
            self._playback_frame_pos = pos[0]
            self._elapsed = pos[0] / self._samplerate
            self._backing_rms = float(np.sqrt(np.mean(chunk[:actual] ** 2))) if actual else 0.0
            if actual < frames:
                raise sd.CallbackStop()

        def run():
            try:
                stream = sd.OutputStream(
                    samplerate=self._samplerate,
                    channels=2,
                    dtype="float32",
                    blocksize=1024,
                    device=output_device,
                    callback=callback,
                )
                stream.start()
                while stream.active and self._is_playing_back:
                    time.sleep(0.05)
                stream.stop()
                stream.close()
                # 只有世代匹配時才修改共用狀態，
                # 避免舊執行緒覆蓋新播放的 _is_playing_back
                if self._playback_gen == gen:
                    self._is_playing_back = False
                    self._backing_rms = 0.0
                    if on_finished:
                        on_finished()
            except Exception as e:
                if self._playback_gen == gen:
                    self._is_playing_back = False
                    self._backing_rms = 0.0
                if on_error:
                    on_error(str(e))
                elif on_finished and self._playback_gen == gen:
                    on_finished()

        threading.Thread(target=run, daemon=True).start()

    def stop_playback(self) -> None:
        self._is_playing_back = False

    def pause_playback(self) -> int:
        """
        暫停回放，保留目前位置。
        回傳暫停時的幀位置（可用於 resume）。
        """
        self._is_playing_back = False
        return self._playback_frame_pos

    def resume_playback(
        self,
        on_progress: Callable[[float, float], None] | None = None,
        on_finished: Callable[[], None] | None = None,
        on_error: Callable[[str], None] | None = None,
        output_device: int | None = None,
    ) -> None:
        """從上次暫停位置繼續回放。"""
        self.start_playback(
            on_progress=on_progress,
            on_finished=on_finished,
            on_error=on_error,
            output_device=output_device,
            start_frame=self._playback_frame_pos,
        )

    @property
    def playback_frame_pos(self) -> int:
        """目前回放幀位置（暫停後可用於繼續）。"""
        return self._playback_frame_pos

    # ------------------------------------------------------------------ #
    #  導出                                                                #
    # ------------------------------------------------------------------ #

    def export(self, output_dir: str, prefix: str = "session") -> dict[str, str]:
        """
        導出三個檔案到 output_dir：
          {prefix}_vocal.wav       — 人聲（mono）
          {prefix}_backing.wav     — 伴奏（stereo）
          {prefix}_multitrack.wav  — 3聲道 [backing_L, backing_R, vocal]

        回傳各檔案的完整路徑 dict。
        """
        import re as _re
        prefix = _re.sub(r'[^\w\-.]', '_', prefix)

        if self._backing_data is None:
            raise ValueError("尚未載入伴奏")
        if self._vocal_data is None:
            raise ValueError("尚未完成錄音")

        os.makedirs(output_dir, exist_ok=True)

        n = min(len(self._backing_data), len(self._vocal_data))
        backing = self._backing_data[:n]
        vocal = self._vocal_data[:n, 0]   # 取 mono

        vocal_path = os.path.join(output_dir, f"{prefix}_vocal.wav")
        backing_path = os.path.join(output_dir, f"{prefix}_backing.wav")
        multi_path = os.path.join(output_dir, f"{prefix}_multitrack.wav")

        sf.write(vocal_path, vocal, self._samplerate)
        sf.write(backing_path, backing, self._samplerate)

        # 3聲道：backing_L, backing_R, vocal
        multi = np.column_stack([backing, vocal.reshape(-1, 1)])
        sf.write(multi_path, multi, self._samplerate)

        return {
            "vocal": vocal_path,
            "backing": backing_path,
            "multitrack": multi_path,
        }

    # ------------------------------------------------------------------ #
    #  音量控制                                                            #
    # ------------------------------------------------------------------ #

    @property
    def volume(self) -> float:
        """目前音量（0.0 = 靜音，1.0 = 原始，2.0 = 最大）。"""
        return self._volume

    @volume.setter
    def volume(self, v: float):
        self._volume = max(0.0, min(2.0, float(v)))

    # ------------------------------------------------------------------ #
    #  狀態查詢                                                            #
    # ------------------------------------------------------------------ #

    @property
    def has_backing(self) -> bool:
        return self._backing_data is not None

    @property
    def has_recording(self) -> bool:
        return self._vocal_data is not None

    @property
    def is_recording(self) -> bool:
        return self._is_recording

    @property
    def is_playing_back(self) -> bool:
        return self._is_playing_back

    @property
    def samplerate(self) -> int:
        return self._samplerate

    @property
    def duration(self) -> float:
        return self._duration

    @property
    def elapsed(self) -> float:
        """目前播放進度（秒），供影片同步使用。"""
        return self._elapsed

    @property
    def mic_rms(self) -> float:
        """即時麥克風 RMS（0.0–1.0），供 VU 表使用。"""
        return self._mic_rms

    @property
    def backing_rms(self) -> float:
        """即時伴奏輸出 RMS（0.0–1.0），供 VU 表使用。"""
        return self._backing_rms

    @property
    def current_pitch(self) -> PitchSample | None:
        """即時音高偵測結果，供 UI 顯示。"""
        return self._current_pitch

    @property
    def pitch_track(self) -> PitchTrack:
        """完整的音高軌跡（錄音期間累積）。"""
        return self._pitch_track

    def _feed_pitch(self, mono: np.ndarray, timestamp: float) -> None:
        """將單聲道音訊餵入音高偵測緩衝區。"""
        samples = mono.astype(np.float64).ravel()
        pos = 0
        while pos < len(samples):
            space = 2048 - self._pitch_buf_pos
            chunk = samples[pos:pos + space]
            self._pitch_buf[self._pitch_buf_pos:self._pitch_buf_pos + len(chunk)] = chunk
            self._pitch_buf_pos += len(chunk)
            pos += len(chunk)

            if self._pitch_buf_pos >= 2048:
                result = self._pitch_detector.detect(self._pitch_buf, timestamp)
                self._current_pitch = result
                if result is not None:
                    self._pitch_track.append(result)
                self._pitch_buf_pos = 0

    def seek(self, seconds: float) -> None:
        """跳轉到指定秒數（僅在非錄音狀態下有效）。"""
        if self._backing_data is None:
            return
        frame = int(seconds * self._samplerate)
        frame = max(0, min(frame, len(self._backing_data) - 1))
        self._playback_pos = frame
        self._playback_frame_pos = frame
        self._elapsed = frame / self._samplerate
