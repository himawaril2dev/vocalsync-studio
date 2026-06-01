#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const EXPORT_CEILING = 0.84;
const VOCAL_LIMIT_CEILING = 0.78;

function readWav(file) {
  const buffer = fs.readFileSync(file);
  if (buffer.toString("ascii", 0, 4) !== "RIFF" || buffer.toString("ascii", 8, 12) !== "WAVE") {
    throw new Error(`Not a WAV file: ${file}`);
  }

  let offset = 12;
  let fmt = null;
  let dataOffset = -1;
  let dataSize = 0;

  while (offset + 8 <= buffer.length) {
    const id = buffer.toString("ascii", offset, offset + 4);
    const size = buffer.readUInt32LE(offset + 4);
    const start = offset + 8;

    if (id === "fmt ") {
      fmt = {
        audioFormat: buffer.readUInt16LE(start),
        channels: buffer.readUInt16LE(start + 2),
        sampleRate: buffer.readUInt32LE(start + 4),
        byteRate: buffer.readUInt32LE(start + 8),
        blockAlign: buffer.readUInt16LE(start + 12),
        bitsPerSample: buffer.readUInt16LE(start + 14),
        subFormat: null,
      };
      if (size >= 40 && fmt.audioFormat === 65534) {
        fmt.subFormat = buffer.readUInt16LE(start + 24);
      }
    } else if (id === "data") {
      dataOffset = start;
      dataSize = size;
      break;
    }

    offset = start + size + (size % 2);
  }

  if (!fmt || dataOffset < 0) {
    throw new Error(`Missing fmt/data chunks: ${file}`);
  }

  const bytesPerSample = fmt.bitsPerSample / 8;
  const sampleCount = dataSize / bytesPerSample;
  const samples = new Float32Array(sampleCount);

  for (let i = 0, pos = dataOffset; i < sampleCount; i += 1, pos += bytesPerSample) {
    let sample;
    const isFloat = fmt.audioFormat === 3 || (fmt.audioFormat === 65534 && fmt.subFormat === 3);
    const isPcm = fmt.audioFormat === 1 || (fmt.audioFormat === 65534 && fmt.subFormat === 1);
    if (isFloat && fmt.bitsPerSample === 32) {
      sample = buffer.readFloatLE(pos);
    } else if (isPcm && fmt.bitsPerSample === 16) {
      sample = buffer.readInt16LE(pos) / 32768;
    } else if (isPcm && fmt.bitsPerSample === 32) {
      sample = buffer.readInt32LE(pos) / 2147483648;
    } else {
      throw new Error(
        `Unsupported WAV format=${fmt.audioFormat} sub=${fmt.subFormat} bits=${fmt.bitsPerSample}: ${file}`,
      );
    }
    samples[i] = Number.isFinite(sample) ? sample : 0;
  }

  return {
    file,
    fmt,
    samples,
    frames: sampleCount / fmt.channels,
    duration: sampleCount / fmt.channels / fmt.sampleRate,
  };
}

function toMono(wav) {
  if (wav.fmt.channels === 1) {
    return wav.samples;
  }
  const mono = new Float32Array(wav.frames);
  for (let frame = 0; frame < wav.frames; frame += 1) {
    let sum = 0;
    for (let ch = 0; ch < wav.fmt.channels; ch += 1) {
      sum += wav.samples[frame * wav.fmt.channels + ch];
    }
    mono[frame] = sum / wav.fmt.channels;
  }
  return mono;
}

function signalStats(samples, ceiling = EXPORT_CEILING) {
  let peak = 0;
  let sumSq = 0;
  let overCeiling = 0;
  let near95 = 0;
  let near99 = 0;

  for (const sample of samples) {
    const abs = Math.abs(sample);
    peak = Math.max(peak, abs);
    sumSq += sample * sample;
    if (abs > ceiling) overCeiling += 1;
    if (abs > 0.95) near95 += 1;
    if (abs > 0.99) near99 += 1;
  }

  const rms = Math.sqrt(sumSq / Math.max(1, samples.length));
  return {
    peak,
    rms,
    crestDb: 20 * Math.log10((peak || 1e-12) / (rms || 1e-12)),
    overCeiling,
    overCeilingPct: (overCeiling / Math.max(1, samples.length)) * 100,
    near95Pct: (near95 / Math.max(1, samples.length)) * 100,
    near99Pct: (near99 / Math.max(1, samples.length)) * 100,
  };
}

function percentileAbs(samples, percentile) {
  const step = Math.max(1, Math.floor(samples.length / 300_000));
  const values = [];
  for (let i = 0; i < samples.length; i += step) {
    values.push(Math.abs(samples[i]));
  }
  values.sort((a, b) => a - b);
  return values[Math.min(values.length - 1, Math.floor(values.length * percentile))] ?? 0;
}

function estimateVocalVsBacking(mixPath, mixMono) {
  const vocalPath = mixPath.replace(/_mix\.wav$/i, "_vocal.wav");
  if (!fs.existsSync(vocalPath)) {
    return null;
  }

  const vocalWav = readWav(vocalPath);
  const vocal = toMono(vocalWav);
  const n = Math.min(mixMono.length, vocal.length);
  const vocalStats = signalStats(vocal, 0.98);
  const gate = Math.max(vocalStats.rms * 0.2, 0.003);

  let dot = 0;
  let vocalSq = 0;
  let active = 0;
  for (let i = 0; i < n; i += 1) {
    if (Math.abs(vocal[i]) >= gate) {
      dot += mixMono[i] * vocal[i];
      vocalSq += vocal[i] * vocal[i];
      active += 1;
    }
  }

  const gain = vocalSq > 1e-12 ? dot / vocalSq : 0;
  let vocalEstSq = 0;
  let backingEstSq = 0;
  let activeMixSq = 0;

  for (let i = 0; i < n; i += 1) {
    if (Math.abs(vocal[i]) >= gate) {
      const vocalEstimate = Math.max(-VOCAL_LIMIT_CEILING, Math.min(VOCAL_LIMIT_CEILING, vocal[i] * gain));
      const backingEstimate = mixMono[i] - vocalEstimate;
      vocalEstSq += vocalEstimate * vocalEstimate;
      backingEstSq += backingEstimate * backingEstimate;
      activeMixSq += mixMono[i] * mixMono[i];
    }
  }

  const vocalRms = Math.sqrt(vocalEstSq / Math.max(1, active));
  const backingRms = Math.sqrt(backingEstSq / Math.max(1, active));
  return {
    vocalPeak: vocalStats.peak,
    vocalRmsDry: vocalStats.rms,
    vocalGainEstimate: gain,
    activeFrames: active,
    vocalRmsEstimate: vocalRms,
    backingRmsEstimate: backingRms,
    activeMixRms: Math.sqrt(activeMixSq / Math.max(1, active)),
    vocalToBackingDb: 20 * Math.log10((vocalRms || 1e-12) / (backingRms || 1e-12)),
  };
}

function analyzeMix(mixPath) {
  const mixWav = readWav(mixPath);
  const mixMono = toMono(mixWav);
  const stats = signalStats(mixWav.samples, EXPORT_CEILING);
  const estimate = estimateVocalVsBacking(mixPath, mixMono);

  return {
    file: mixPath,
    duration: mixWav.duration,
    channels: mixWav.fmt.channels,
    sampleRate: mixWav.fmt.sampleRate,
    peak: stats.peak,
    rms: stats.rms,
    crestDb: stats.crestDb,
    overCeilingPct: stats.overCeilingPct,
    near95Pct: stats.near95Pct,
    near99Pct: stats.near99Pct,
    p995: percentileAbs(mixWav.samples, 0.995),
    vocalEstimate: estimate,
  };
}

function latestMixesFromDefaultFolder(limit = 5) {
  const dir = path.join(process.env.USERPROFILE ?? "", "Downloads", "YouTube");
  if (!fs.existsSync(dir)) return [];
  return fs
    .readdirSync(dir)
    .filter((name) => /_mix\.wav$/i.test(name))
    .map((name) => path.join(dir, name))
    .sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs)
    .slice(0, limit);
}

function formatNumber(value, digits = 4) {
  return Number.isFinite(value) ? value.toFixed(digits) : "n/a";
}

function printSummary(result) {
  const estimate = result.vocalEstimate;
  console.log(path.basename(result.file));
  console.log(`  duration=${formatNumber(result.duration, 2)}s channels=${result.channels} sr=${result.sampleRate}`);
  console.log(
    `  mix peak=${formatNumber(result.peak)} rms=${formatNumber(result.rms)} crest=${formatNumber(result.crestDb, 2)}dB p99.5=${formatNumber(result.p995)}`,
  );
  console.log(
    `  over>${EXPORT_CEILING}=${formatNumber(result.overCeilingPct, 5)}% near>0.95=${formatNumber(result.near95Pct, 5)}% near>0.99=${formatNumber(result.near99Pct, 5)}%`,
  );
  if (estimate) {
    console.log(
      `  vocal/backing active=${formatNumber(estimate.vocalToBackingDb, 2)}dB vocal_rms=${formatNumber(estimate.vocalRmsEstimate)} backing_rms=${formatNumber(estimate.backingRmsEstimate)} gain_est=${formatNumber(estimate.vocalGainEstimate, 3)}`,
    );
  }
}

const args = process.argv.slice(2);
const mixPaths = args.length > 0 ? args : latestMixesFromDefaultFolder(5);

if (mixPaths.length === 0) {
  console.error("No mix WAV files found. Pass one or more *_mix.wav paths.");
  process.exitCode = 1;
} else {
  for (const mixPath of mixPaths) {
    printSummary(analyzeMix(path.resolve(mixPath)));
  }
}
