# VocalSync Studio User Guide

[繁體中文](USER_GUIDE.md) | **English** | [日本語](USER_GUIDE.ja.md)

> Applies to: v0.4.0 or newer
> Last updated: 2026-06

---

## Table of Contents

1. [Welcome](#1-welcome)
2. [5-Minute Quick Start](#2-5-minute-quick-start)
3. [Main Interface Tour](#3-main-interface-tour)
4. [Feature Details](#4-feature-details)
   - [4.1 YouTube Backing Download](#41-youtube-backing-download)
   - [4.2 Importing a Backing Track](#42-importing-a-backing-track)
   - [4.3 Lyrics and Subtitles](#43-lyrics-and-subtitles)
   - [4.4 Pitch Curve Source and Guide Vocal](#44-pitch-curve-source-and-guide-vocal)
   - [4.5 Audio Device Setup](#45-audio-device-setup)
   - [4.6 Latency Calibration](#46-latency-calibration)
   - [4.7 Preview · Record · Playback](#47-preview--record--playback)
   - [4.8 Pitch Timeline](#48-pitch-timeline)
   - [4.9 A-B Loop](#49-a-b-loop)
   - [4.10 Speed · Pitch Shift](#410-speed--pitch-shift)
   - [4.11 Exporting Your Recording](#411-exporting-your-recording)
   - [4.12 Song Library](#412-song-library)
5. [Keyboard Shortcuts](#5-keyboard-shortcuts)
6. [FAQ](#6-faq)
7. [Troubleshooting](#7-troubleshooting)
8. [Reporting Issues](#8-reporting-issues)

---

## 1. Welcome

VocalSync Studio is a **desktop practice room** for vocalists who want to:

- Record while listening to a backing track and see their live pitch curve
- Compare their singing against a target melody (vocals separated from the original)
- Drill specific passages with A-B looping
- Slow down without changing pitch, so they can practice difficult phrases

The app runs **fully offline** (except for downloading backing tracks). Your recordings stay on your computer — nothing is uploaded to any cloud service.

> 📢 **Transparency statement**

---

## 2. 5-Minute Quick Start

### Step 1｜Launch the app

Extract `VocalSync.Studio.Portable.x.y.z.zip` and double-click **`vocalsync-studio.exe`** inside the folder.

> ⚠️ Do not rearrange the folder structure. `DirectML.dll` and `models/` are bundled runtime dependencies — **never move them separately**. Auto-installed yt-dlp / FFmpeg files prefer the portable root, so the entire folder remains portable as a unit.

### Step 2｜Prepare a backing track

**Option A: Download from YouTube**

1. Click the **Setup** tab in the left sidebar
2. Expand the **YouTube Download** section at the top
3. Paste a YouTube URL (e.g. `https://www.youtube.com/watch?v=xxxxxxxxxxx`)
4. Choose `mp3` as the format, then click **Download**

If yt-dlp or FFmpeg is missing on first use, click **Auto Install** and the app will set them up for you.

**Option B: Use a local audio file**

Open **Songs** → in **Song and Pitch Sources**, click **Import Backing** → choose an MP3 / WAV / M4A (etc.) file.

### Step 3｜Add a pitch curve and guide vocal (optional, recommended)

If you have the original song, first separate `vocals.wav` with UVR5 / Moises / Demucs. Then open **Songs** → **Song and Pitch Sources** → click **Import vocals track (vocals.wav)**.

After importing, VocalSync does two things:

1. Creates a grey-blue reference curve on the pitch timeline
2. Enables the **Guide** volume row on the Recording tab, so the original vocal can play quietly in your headphones while you sing

Start with the guide around **15 to 30%**. For final takes, lower it heavily or turn it off.

### Step 4｜Configure input and output devices

Expand the **Device Selection** section:

- **Input device**: your microphone
- **Output device**: your headphones (**we strongly recommend headphones** — otherwise the mic picks up the backing and your recording ends up muddy)

### Step 5 - Calibrate latency (recommended on first use)

Expand **Latency Calibration** and click **Calibrate latency**. The app reads stream timestamp latency and buffer latency reported by the audio devices, estimates the recording/playback delay for this microphone/headphone pair, and saves it to the device-pair profile.

When you need a value closer to real singing, expand **Rhythm voice calibration**, wear headphones, and make short voice sounds into the mic on the beat. The manual fine-tune slider and shortcut buttons also save immediately to the same device-pair profile.

### Step 6 - Start recording

1. Switch to the **Recording** tab
2. Press the spacebar or the **red record button** to start
3. Sing along with the backing
4. Press space to pause, or `Esc` to stop
5. Click **Export** to save your recording as a WAV file

The lower half shows your pitch curve in real time. The top area shows lyrics or video in sync.

---

## 3. Main Interface Tour

The left sidebar has 5 tabs:

### 🎯 Setup

This is where the app opens by default. Global app and device settings live here, grouped into collapsible sections:

- YouTube Download
- Device Selection
- Latency Calibration

Collapsed/expanded state is preserved across tab switches, so you don't have to re-open sections every time.

### 🎼 Songs

This is the per-song preparation page. Load the backing, lyrics/subtitles, melody source, and guide vocal here, then save them as a song profile.

### 🎤 Recording

This is the main workspace. The top half is split into two columns (drag the divider to adjust the ratio):

- **Left**: video / audio file info (video shows its frame if imported)
- **Right**: lyrics panel with current-line highlight, global delay, and per-line Start / End sync buttons

The bottom half is a **full-width pitch timeline** showing your singing versus the target melody in real time.

Two control rows at the very bottom:

- **Transport row**: Preview / Pause / Stop / Record / progress bar / A-B loop
- **Control row**: Backing volume / Mic volume / Guide vocal volume / Speed / Pitch / Export / Playback

### 📊 Pitch

A tab dedicated to pitch analysis. The header shows melody status and alignment results, the center is a full-screen pitch timeline, and the bottom has a mini transport bar so you can listen while watching.

Use this tab when you just want to review your last recording's pitch curve against the target without recording anything new.

### ℹ️ About

### Language Switcher

At the very bottom of the left sidebar you'll find a language selector supporting **繁體中文 / English / 日本語**. Switching is instant — no restart needed.

---

## 4. Feature Details

### 4.1 YouTube Backing Download

**Location**: Setup tab → YouTube Download section

The app does **not bundle** yt-dlp or FFmpeg. On first use, their status appears here:

| Status | Meaning | Action |
|---|---|---|
| 🟢 yt-dlp v2026.x.x | Installed and ready | None |
| 🔴 yt-dlp not installed | yt-dlp not found | Click **Auto Install** |
| 🟢 FFmpeg ✓ | Available on your system | None |
| 🔴 FFmpeg not installed | Not on your system | Click **Auto Install** |

**What local trust and Auto Install do**:

- **Use local tools**: Detects yt-dlp / FFmpeg already installed on your computer. After you trust them, VocalSync records their path and SHA-256, then only runs the same verified files.
- **Auto-install yt-dlp**: Downloads `yt-dlp.exe` from GitHub Releases, verifies SHA-256, and prefers the portable root.
- **Auto-install FFmpeg**: Downloads a pinned essentials build, verifies SHA-256, extracts `ffmpeg.exe` / `ffprobe.exe`, and prefers the portable root.
- **Install location**: Tools are kept inside the portable root. If the folder is not writable, move the whole portable folder to a writable location before installing tools.

If your corporate network blocks outbound connections, install yt-dlp / FFmpeg from a trusted source first, then return to the Download tab, detect the local tools, and trust them.

#### Download Options

| Field | Description |
|---|---|
| **URL** | Paste a YouTube video, playlist, or channel link. The app auto-detects the type and shows a tag |
| **Format** | `mp3` / `m4a` / `wav` (audio only) / `video` (video + audio) / `subtitle_only` (captions only) |
| **Quality** | Shown only when `video` is selected: `best` / 1080p / 720p / 480p / 360p |
| **Subtitles** | `none` / Traditional Chinese / Simplified Chinese / English / Japanese / `all`. Falls back to auto-translated captions if the requested language isn't available |
| **Output folder** | Defaults to Desktop. Click **Browse** to change |

While a download runs, you'll see a progress bar, speed, and ETA. Click **Cancel Download** any time to abort.

#### After the Download Finishes

If any subtitle files (`.srt` / `.vtt` / `.lrc`) were captured, a subtitle list appears below. Click **Load as Lyrics** next to an entry to send it directly to the lyrics panel on the Recording tab — no extra file picking required.

### 4.2 Importing a Backing Track

**Location**: Songs tab → Song and Pitch Sources section

Click **Import Backing** and pick a local file. Supported formats:

- **Audio**: WAV / MP3 / M4A / AAC / FLAC / OGG
- **Video**: MP4 / MKV / WebM (the video frame appears on the left side of the Recording tab)

After a successful import you'll see file name, duration, and type (audio / video). The app then automatically:

1. Tries to detect a matching target melody (same-name vocals file or UVR cache)
2. Scans for embedded subtitle tracks if it's a video file

### 4.3 Lyrics and Subtitles

**Location**: Songs tab → Lyrics / Subtitles section

Supported lyrics formats:

| Format | Source | Time precision |
|---|---|---|
| **LRC** | Converted from KRC, hand-authored | Per line |
| **SRT** | YouTube subtitles, Netflix subtitles | Per line (start/end time) |
| **VTT** | Web Video Text Tracks | Per line |
| **TXT** | Plain-text lyrics | No timing (display only) |

#### Three Ways to Load

1. **Manual load**: Click **Load Lyrics File** → pick LRC / SRT / VTT / TXT
2. **Embedded video subtitles**: After importing a video, detected tracks show up as buttons like "#0 zho · subrip" — click to extract
3. **YouTube-downloaded subtitles**: After a download, click **Load as Lyrics** in the subtitle list

#### Live Sync Display

On the Recording tab, switch the top-right panel to **Lyrics** — the current line auto-centers and is highlighted as playback progresses.

If subtitle timing is off, use the **Global delay** slider directly in the Recording lyrics panel to shift the whole track. During playback, click a line's Start / End buttons to write the current playback time into that lyric line. LRC / SRT / ASS exports use the adjusted timing.

### 4.4 Pitch Curve Source and Guide Vocal

**Location**: Songs tab → Song and Pitch Sources section

The pitch curve source appears as a **grey-blue line** on the pitch timeline and is your reference while practicing. Two sources are supported:

#### Source A: Import Clean Vocals (**recommended**)

Best when: you have the original song and want to align precisely to the original vocal's pitch.

**Prerequisite**: use an **external tool** like UVR5 / Moises / Demucs to separate `vocals.wav` from the original (the app itself does not perform source separation, for legal reasons).

**Steps**: Click **Import vocals track (vocals.wav)** → pick `vocals.wav`. The backend will:

1. Run PYIN pitch extraction to produce per-timestep notes
2. Automatically align with your backing track via cross-correlation to compute the time offset
3. Load the same vocals track as a guide vocal for low-volume monitoring while recording or playing back

The guide vocal is monitor-only and is not mixed into exported vocal / mix files. If the vocals and backing come from different sources, the guide uses the same auto-alignment offset; adjusting fine-tune moves the guide along with the pitch curve.

#### Guide vocal monitoring tips

- **Learning a new song**: set Guide to 15 to 30% so you can hear the melody and entry points
- **Recording a final take**: lower Guide to 0 to 10%, or uncheck guide monitoring
- **Alignment check**: if the guide and backing feel offset, adjust fine-tune on the Songs tab; the guide and pitch curve move together
- **Export behavior**: Guide is headphone monitoring only, and exported vocal / mix WAV files do not include the guide vocal

#### Source B: Load MIDI (**manual reference**)

Best when: you have a prepared melody MIDI file, or can export the melody line from notation or arrangement software.

Click **Load MIDI** → pick a `.mid` / `.midi` file. The app displays the MIDI melody as the grey-blue reference line.

#### Alignment Fine-Tune

After loading a melody, if auto-alignment isn't precise enough, a **fine-tune slider** (±500 ms) appears below. As you drag, the grey-blue line shifts in real time — release when alignment feels right.

**Alignment confidence labels**:

- 🟢 **High confidence**: high peak-to-mean ratio — usually works out of the box
- 🟡 **Medium confidence**: may need ±50 ms tweaking
- 🔴 **Low confidence**: auto-alignment likely failed — try manual tuning or a different source

### 4.5 Audio Device Setup

**Location**: Setup tab → Device Selection section

#### Input Device

Your microphone. Common options:

- Built-in mic: the microphone array above your laptop keyboard
- USB mic: Blue Yeti, HyperX QuadCast, Shure MV7, etc.
- Audio interface: PreSonus Quantum ES2, Focusrite Scarlett, MOTU M2, etc.

**Recommendation**: pick a device that supports **exclusive mode** (e.g. an ASIO interface) when possible — it gives lower latency and won't get grabbed by other apps.

#### Output Device

**Use headphones**, not your computer's built-in speakers. Reasons:

- Speaker-emitted backing bleeds into your mic, creating double-layered audio
- Higher latency and less accurate pitch detection

#### Pitch Engine

| Option | Description | Recommended |
|---|---|---|
| **auto** | App picks automatically: CREPE by default, falls back to PYIN if loading fails | **Default** |
| **crepe** | Force CREPE neural-network model — high accuracy, requires DirectML | If you have a discrete GPU |
| **yin** | Force the PYIN classical algorithm — lower CPU load, slightly lower accuracy | Lower-end machines |

The CREPE model is `models/crepe-tiny.onnx` (~22 MB) and loads into memory at startup.

### 4.6 Latency Calibration

**Location**: Setup tab > Latency Calibration section.

**Why calibration is needed**:

Mic input, audio drivers, system processing, app processing, and headphone output create a short delay. Calibration makes the recorded vocal line up more closely with the backing track, pitch curve, and lyrics.

#### Recommended Flow: Pre-Recording Estimate + Manual Fine-Tune

1. Choose the input and output devices.
2. In **Latency Calibration**, click **Calibrate latency**.
3. The app briefly opens input and output streams and reads timestamp latency reported by the device.
4. When the device reports partial data, the app estimates the remaining part from buffer latency.
5. Successful results are saved to the current input device + output device + sample-rate profile.
6. When the device does not provide usable latency data, the previous latency value is kept.

The pre-recording estimate finishes with one click after device selection.

Manual fine-tune on the Setup tab and **Sync fine-tune** on the Recording tab use the same latency value. The slider and `-20`, `-10`, `-5`, `+5`, `+10`, `+20 ms` buttons adjust the current value directly and save immediately.

#### Optional Tool: Rhythm Voice Calibration

1. Expand **Rhythm voice calibration**.
2. Wear headphones and say a short `ta`, `da`, or `ah` into the mic on each beat.
3. The app gives 2 warm-up beats, measures 8 beats, and stops automatically.
4. High or medium confidence results are saved; low confidence results keep the previous latency value.

### 4.7 Preview · Record · Playback

**Location**: Recording tab → transport row (upper)

Four round buttons cover the full practice loop:

| Button | Icon | Function |
|---|---|---|
| **Preview** | ▶ (blue) | Plays backing only, no recording. For getting familiar with the song |
| **Pause** | ‖ (blue) | Pauses the current mode (preview/record/playback) and retains position |
| **Stop** | ■ (dark) | Stops and returns to 0 s |
| **Record** | ● (red) | Plays backing and records your mic simultaneously |

#### State Machine

- **Idle → Preview**: press ▶
- **Idle → Record**: press ● (the previous recording buffer is cleared first)
- **Recording → Pause**: press ‖
- **Paused → Resume**: press ▶ (continues the same mode — modes are never mixed)
- **Any state → Stop & reset**: press ■ or `Esc`

#### Continue-Recording

Pause mid-recording, drag the progress bar to a new position, then press ● — the app continues recording from the new point. If you drag **backwards** (to a time that's already recorded), a confirmation dialog appears to prevent you from accidentally destroying prior content.

#### Clear Recording

If you're unhappy with a take and want to redo the whole thing, press **Clear Recording** in the control row. Confirmation is required to prevent mistakes.

#### Playback

After recording, press **Playback** in the control row — the app plays the backing and your recorded vocal together (automatically compensated by the calibrated latency) so you can immediately hear the alignment.

After recording, **Sync fine-tune** appears on the Recording tab. While playback is running, drag the slider or press `±5 / ±10 / ±20 ms` to move the vocal earlier or later in real time. The saved value also applies to the pitch curve and export.

### 4.8 Pitch Timeline

Shown in the lower half of the Recording tab / full-screen on the Pitch tab. It scrolls left-right following playback and updates in real time.

#### Line Colors

| Color | Meaning |
|---|---|
| **Grey-blue line** | Target melody (from vocals / MIDI) |
| **Gold line** | Your live singing pitch |
| **Beige grid** | Semitone ruler + octave labels (C4, C5, ...) |
| **Red vertical line** | Current playback cursor |

#### Pitch Detection Mechanism

The backend runs CREPE or PYIN at **16 ms** frames, outputting frequency (Hz) → note (with cents offset). A line is drawn only when the RMS exceeds a threshold and the model confidence (CREPE) or clarity (PYIN) is high enough — breath sounds and room noise don't cause the line to jump around.

#### Latency Compensation

The vocal pitch shown on screen is compensated by the current latency value. When you drag **Sync fine-tune**, the gold line moves horizontally in real time. The alignment between your line, the lyrics, and the target melody represents the true sync point.

### 4.9 A-B Loop

**Location**: Recording tab → A-B buttons in control row, or keyboard shortcuts `A` / `B`

For drilling choruses or tricky phrases. Workflow:

1. Play to the desired loop **start** → press `A` or the A button (lights up blue)
2. Play to the desired loop **end** → press `B` or the B button (lights up blue; only active after A is set)
3. A blue region appears on the progress bar, and playback auto-loops within it
4. To cancel the loop → click the `✕` button

A-B loop works in combination with record, preview, or playback.

### 4.10 Speed · Pitch Shift

**Location**: Recording tab → Speed / Pitch buttons in control row

#### Speed (without changing pitch)

The backend uses the **WSOLA** (Waveform Similarity Overlap-Add) time-stretch algorithm. Six fixed steps:

`0.5x / 0.75x / 0.9x / 1.0x / 1.1x / 1.25x`

- **Slow down for hard phrases**: try 0.75x or 0.5x to learn the melodic contour
- **Fast browse a long track**: try 1.25x

At 1.0x the app bypasses WSOLA (zero-latency passthrough). Other steps incur a small quality loss.

#### Pitch Shift (transposition)

Shifts pitch up/down without changing speed. Range: -12 to +12 semitones (one octave each way).

**How to use**:

- Original key too high → press `-` to shift down a few semitones (usually 2–3)
- Want to practice a transposed version → press `+` to shift up

Pitch shift and speed change can be combined, but **the further you move from 1.0x + 0 semitones, the more audible the quality loss**.

#### Reset Button (↺)

Each control has a reset button next to it that returns to defaults (1.0x / 0 semitones) with one click.

### 4.11 Exporting Your Recording

**Location**: Recording tab → **Export** button in control row

The **Export** button lights up as soon as you've recorded at least once (even if you subsequently cleared the buffer, the last take is still in memory).

#### What Gets Exported

After you pick a destination folder and filename prefix, the app writes **two WAV files**:

| Filename | Contents | Use case |
|---|---|---|
| `<prefix>_vocal.wav` | Pure vocal track (no backing) | Upload to mixing studio, further production |
| `<prefix>_mix.wav` | Backing + vocal mixed | Publish directly, share with friends |

#### Auto Volume Balance (optional)

A checkbox labeled **Normalize** sits in the bottom-right of the control row. When enabled, the exported `_mix.wav` is balanced by a LUFS-ish rule so the vocal sits naturally above the backing — no more "backing drowns the vocal" or vice versa.

If you prefer full manual control over the mix, uncheck it — export will respect the exact ratio you set with the volume sliders.

---

### 4.12 Song Library

**Location**: left sidebar -> **Songs**, and the quick song selector at the top of the Recording tab.

The song library saves each song's pre-recording setup: backing track, lyrics and subtitle timing, melody, guide vocal, guide enabled state, sync fine-tune, mix levels, vocal-forward preset, A-B loop, playback speed, and key shift.

Workflow:

1. Open **Songs**, load the backing, lyrics, melody, and guide vocal, then adjust sync and mix.
2. Enter a song title and click **Save to library**.
3. Later, click **Load** on a song card in Songs, or select that song from the Recording tab selector. The setup is restored.
4. After changing lyrics, sync, guide vocal, or mix from Songs or Recording, the current song is marked changed. Click **Save** on the song card or **Update song** on Recording to save the new setup.

The song library does not store the recorded vocal buffer. When loading another song while a recording exists, the app asks for confirmation before clearing it.

---

## 5. Keyboard Shortcuts

| Key | Function | When |
|---|---|---|
| `Space` | Toggle play / pause | Idle → preview; Recording → pause; Paused → resume |
| `R` | Start recording | While idle or paused (no effect mid-recording) |
| `Esc` | Stop and return to start | Any non-idle state |
| `A` | Set loop A point | Any playback position |
| `B` | Set loop B point | Only after A has been pressed |
| `+` / `=` | Transpose up one semitone (+1) | Adjust pitch |
| `-` | Transpose down one semitone (-1) | Adjust pitch |

> Shortcuts are automatically disabled while focus is on an input (input / textarea / select), so they won't interfere with your typing.

---

## 6. FAQ

### Q1. My recordings have echo / double audio?

**A**: Almost always because you weren't wearing headphones, or your headphones leaked — the mic captured the backing coming out of the speakers/headphones. Switch to **closed-back headphones**.

### Q2. The pitch curve jumps all over the place — it's not matching what I sang.

**A**: Likely causes:

1. **Mic gain too high**: lower the Mic slider in the control row below 100% and watch the meter — aim to keep peaks in the yellow-green zone
2. **High noise floor**: AC or fans push the RMS over threshold. Turn them off or switch to a quieter environment
3. **Wrong pitch engine**: on low-end hardware, CREPE may stutter. Switch to `yin`

### Q3. YouTube download says "HTTP Error 403 Forbidden" or "Sign in to confirm you're not a bot".

**A**: YouTube occasionally blocks yt-dlp requests. How to fix:

1. Check the yt-dlp version in the download section's tool status — if outdated, click **Auto Install** to update
2. If it still fails, the yt-dlp community may need a week or so to publish a fix
3. Or use yt-dlp externally to download first, then import the resulting file via **Import Backing**

### Q4. My PC has no discrete GPU — will CREPE stutter?

**A**: The CREPE-tiny model is already optimized to **run on low-end CPUs** (about 15–25% CPU usage). If you still see stutter or missed frames, switch to the `yin` engine — accuracy drops slightly but CPU load becomes negligible.

### Q5. Can I load the exported WAV into other programs?

**A**: Yes. The export format is **16-bit PCM WAV at the same sample rate as the backing** (typically 44.1 kHz or 48 kHz). Any DAW (Reaper, FL Studio, Logic, Audition) can open it.

### Q6. Does it support macOS or Linux?

### Q7. Does this app upload my recordings?

**A**: **No**. The app is fully offline. Aside from network requests to YouTube when you download a backing track, recording, pitch analysis, and export all happen on your machine. Exported WAV files go to the folder you specify — they are never uploaded anywhere.

### Q8. Can I use this commercially?

**A**: VocalSync Studio itself is MIT-licensed — the app is free to use (including commercially). However:

- Content downloaded via yt-dlp is subject to the **original content's copyright** and YouTube's Terms of Service
- If you publish your recordings (YouTube, Spotify, etc.), you need a **cover license** for the original song (karaoke/KTV licensing does not cover public distribution)

### Q9. Can I report a bug or request a feature?

**A**: Yes. You can report through GitHub Issues or email. See "[Reporting Issues](#8-reporting-issues)" below.

---

## 7. Troubleshooting

### The app won't launch

| Symptom | Possible cause | Fix |
|---|---|---|
| Double-click does nothing | WebView2 runtime missing (early Windows 10 builds) | Download and install from [Microsoft's site](https://developer.microsoft.com/en-us/microsoft-edge/webview2/) |
| "DirectML.dll not found" | Folder structure changed | Re-extract the whole zip — don't move the exe by itself |
| SmartScreen warning | The app isn't code-signed | Click **More info** → **Run anyway** |

### No sound while recording

1. Check Setup tab → Device Selection → correct microphone selected?
2. Is the Mic slider in the control row at 0?
3. Windows Settings → Privacy → Microphone → app access allowed?
4. Is another program (Discord, OBS) claiming exclusive mic access?

### The pitch curve doesn't appear

- Mic level too low to cross the RMS threshold → raise Mic gain
- Pitch engine failed to load (CREPE can't find `models/crepe-tiny.onnx`) → re-extract the portable zip

### Alignment keeps failing

- Your backing and vocals come from too-different sources (different versions, remixes) → manually drag the fine-tune slider until it sounds right
- Source volume too low → make sure `vocals.wav` isn't silent

### Time-stretched audio sounds rough

- Extreme steps (0.5x or 1.25x) are inherently lossy — that's WSOLA's fundamental limit
- For high-quality slow practice, do the time-stretch in Reaper / Audition beforehand and import the result

---

## 8. Reporting Issues

### Reporting a bug or suggesting a feature

Including the following information will make debugging much faster:

- OS version (e.g. Windows 11 Pro 23H2)
- VocalSync Studio version (visible on the About tab / exe filename)
- Steps to reproduce (the more detail the better)
- Screenshot of the error message
- For audio-related issues, include input / output device model

Report through either channel:

- **GitHub Issues**: <https://github.com/himawaril2dev/vocalsync-studio/issues>
- **Email**: `himawaril2dev@gmail.com`

The About tab also includes GitHub, Ko-fi, and a manual **Check update** button. The update check contacts GitHub Releases only when you press the button.

If VocalSync Studio helps your practice, you can support continued development on Ko-fi: <https://ko-fi.com/himawari168>

---

*Last updated: 2026-06 · VocalSync Studio v0.4.0*
