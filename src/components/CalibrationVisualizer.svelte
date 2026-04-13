<script lang="ts">
  import { onDestroy } from "svelte";
  import { calibrationStatus } from "../stores/settings";

  let { onFinish }: { onFinish: () => void } = $props();

  let canvasEl = $state<HTMLCanvasElement | null>(null);
  let rafId: number = 0;
  let animationStartTime = 0;

  // 從 store 讀後端 emit 的真實時間軸參數，避免前後端不同步
  let isRunning = $derived($calibrationStatus.isRunning);
  let bpm = $derived($calibrationStatus.bpm);
  let warmupBeats = $derived($calibrationStatus.warmupBeats);
  let measurementBeats = $derived($calibrationStatus.measurementBeats);
  let totalBeats = $derived(warmupBeats + measurementBeats);
  let prepMs = $derived($calibrationStatus.prepMs);
  let beatIntervalMs = $derived(60000 / bpm);
  // 拍序時間 (ms)：i=0..totalBeats-1
  let hitTimes = $derived(
    Array.from({ length: totalBeats }, (_, i) =>
      i === 0 ? prepMs : prepMs + i * beatIntervalMs,
    ),
  );
  let lastHitTime = $derived(hitTimes[totalBeats - 1] ?? prepMs);
  // 完成 / 失敗訊息會顯示這段時間
  const TAIL_MS = 1500;
  let totalDuration = $derived(lastHitTime + TAIL_MS);
  const FLASH_DURATION = 280;

  // 即時拍子結果（從 store 取，會被 Rust 事件填入）
  let beatResults = $derived($calibrationStatus.beats);
  let finalLatency = $derived($calibrationStatus.finalLatencyMs);
  let stdDev = $derived($calibrationStatus.stdDevMs);
  let errorMsg = $derived($calibrationStatus.error);

  $effect(() => {
    if (isRunning && canvasEl) {
      animationStartTime = performance.now();
      cancelAnimationFrame(rafId);
      animate();
    } else {
      cancelAnimationFrame(rafId);
    }
  });

  function animate() {
    if (!canvasEl) return;
    const ctx = canvasEl.getContext("2d");
    if (!ctx) return;

    const W = canvasEl.width;
    const H = canvasEl.height;
    ctx.clearRect(0, 0, W, H);

    const now = performance.now();
    const t = now - animationStartTime;

    // 動畫長度結束後通知父元件，但若仍在 isRunning 則繼續顯示等待結果
    if (t > totalDuration && !isRunning) {
      onFinish();
      return;
    }

    // ── 背景準線 ──
    const groundY = H - 110;
    ctx.beginPath();
    ctx.moveTo(40, groundY);
    ctx.lineTo(W - 40, groundY);
    ctx.strokeStyle = "#e8e2d8";
    ctx.lineWidth = 4;
    ctx.stroke();

    // ── 標題 ──
    ctx.fillStyle = "#fff";
    ctx.font = "bold 22px sans-serif";
    ctx.textAlign = "center";
    ctx.fillText("當球碰到準線時，對麥克風拍手！", W / 2, 38);

    // 第一段為「暖身」提示
    ctx.fillStyle = "rgba(255, 255, 255, 0.55)";
    ctx.font = "14px sans-serif";
    if (t < prepMs + warmupBeats * beatIntervalMs) {
      ctx.fillText(
        `前 ${warmupBeats} 拍是暖身，幫你抓節奏，不會納入計算`,
        W / 2,
        62,
      );
    } else if (t < lastHitTime + 200) {
      ctx.fillText(
        `量測中（共 ${measurementBeats} 拍）`,
        W / 2,
        62,
      );
    }

    // ── 找出狀態 ──
    let activeBall = -1;
    let ballProgress = 0;
    let lastHitIdx = -1;
    let timeSinceLastHit = Infinity;

    for (let i = 0; i < totalBeats; i++) {
      const dropStart = i === 0 ? 0 : hitTimes[i - 1];
      const hitTime = hitTimes[i];
      if (t >= dropStart && t < hitTime) {
        activeBall = i;
        ballProgress = (t - dropStart) / (hitTime - dropStart);
      }
      if (t >= hitTime) {
        lastHitIdx = i;
        timeSinceLastHit = t - hitTime;
      }
    }

    // ── 落地閃光 ──
    if (lastHitIdx >= 0 && timeSinceLastHit < FLASH_DURATION) {
      const fadeP = timeSinceLastHit / FLASH_DURATION;
      const hitAlpha = 1 - fadeP;
      const isWarmup = lastHitIdx < warmupBeats;
      const color = isWarmup ? "180, 180, 180" : "255, 170, 0";

      ctx.beginPath();
      ctx.arc(W / 2, groundY, 50 + hitAlpha * 30, 0, Math.PI * 2);
      ctx.fillStyle = `rgba(${color}, ${hitAlpha * 0.5})`;
      ctx.fill();

      ctx.fillStyle = `rgba(255, 255, 255, ${hitAlpha})`;
      ctx.font = "bold 64px sans-serif";
      const display = isWarmup
        ? `暖${lastHitIdx + 1}`
        : (lastHitIdx - warmupBeats + 1).toString();
      ctx.fillText(display, W / 2, groundY - 80);
    }

    // ── 下落中的球 ──
    if (activeBall >= 0) {
      const ballTop = 90;
      const radius = 22;
      const targetY = groundY - radius;
      const eased = ballProgress * ballProgress;
      const realY = ballTop + (targetY - ballTop) * eased;

      const isWarmupBall = activeBall < warmupBeats;
      ctx.beginPath();
      ctx.arc(W / 2, realY, radius, 0, Math.PI * 2);
      ctx.fillStyle = isWarmupBall ? "#aaaaaa" : "#ffaa00";
      ctx.fill();
      ctx.lineWidth = 3;
      ctx.strokeStyle = "#fff";
      ctx.stroke();
    } else if (lastHitIdx === totalBeats - 1) {
      // 全部拍完，顯示等待結果或最終結果
      ctx.textAlign = "center";
      if (errorMsg) {
        ctx.fillStyle = "rgba(255, 120, 120, 0.95)";
        ctx.font = "bold 22px sans-serif";
        ctx.fillText("校準失敗", W / 2, groundY - 40);
        ctx.font = "14px sans-serif";
        ctx.fillStyle = "rgba(255, 200, 200, 0.85)";
        // 自動換行（簡單處理）
        const maxWidth = W - 80;
        const words = errorMsg.split("");
        let line = "";
        let yLine = groundY - 12;
        for (const ch of words) {
          const test = line + ch;
          if (ctx.measureText(test).width > maxWidth) {
            ctx.fillText(line, W / 2, yLine);
            line = ch;
            yLine += 20;
          } else {
            line = test;
          }
        }
        if (line) ctx.fillText(line, W / 2, yLine);
      } else if (finalLatency !== null) {
        ctx.fillStyle = "rgba(255, 255, 255, 0.95)";
        ctx.font = "bold 32px sans-serif";
        ctx.fillText(`延遲 ${finalLatency} ms`, W / 2, groundY - 40);
        if (stdDev !== null) {
          ctx.font = "14px sans-serif";
          ctx.fillStyle = "rgba(255, 255, 255, 0.7)";
          ctx.fillText(
            `標準差 ${stdDev.toFixed(1)} ms（越小越穩）`,
            W / 2,
            groundY - 14,
          );
        }
      } else {
        ctx.fillStyle = "rgba(255, 255, 255, 0.85)";
        ctx.font = "bold 22px sans-serif";
        ctx.fillText("分析中…", W / 2, groundY - 40);
      }
    }

    // ── 拍數指示點 + 偏差數字 ──
    const dotY = H - 55;
    const numY = H - 22;
    const dotSpacing = Math.min(56, (W - 80) / totalBeats);
    const dotsStartX = W / 2 - ((totalBeats - 1) * dotSpacing) / 2;

    for (let i = 0; i < totalBeats; i++) {
      const cx = dotsStartX + i * dotSpacing;
      const isWarmupDot = i < warmupBeats;
      const passed = i <= lastHitIdx;
      const result = beatResults.find((b) => b.beatIdx === i);

      // 圓點
      ctx.beginPath();
      ctx.arc(cx, dotY, 9, 0, Math.PI * 2);
      if (!passed) {
        ctx.fillStyle = "#444";
      } else if (isWarmupDot) {
        ctx.fillStyle = "#888";
      } else if (result && !result.detected) {
        ctx.fillStyle = "#cc4444"; // 紅 = 沒偵測到
      } else if (result && !result.accepted) {
        ctx.fillStyle = "#cc8844"; // 橘 = 離群值
      } else {
        ctx.fillStyle = "#ffaa00"; // 黃 = 接受
      }
      ctx.fill();
      ctx.lineWidth = 2;
      ctx.strokeStyle = "#fff";
      ctx.stroke();

      // 偏差數字（只對量測拍且有結果顯示）
      if (!isWarmupDot && result && result.detected) {
        ctx.fillStyle = result.accepted
          ? "rgba(255, 220, 130, 0.9)"
          : "rgba(255, 180, 130, 0.7)";
        ctx.font = "bold 12px sans-serif";
        ctx.textAlign = "center";
        const sign = result.offsetMs >= 0 ? "+" : "";
        ctx.fillText(`${sign}${result.offsetMs.toFixed(0)}`, cx, numY);
      }

      // 暖身拍小標
      if (isWarmupDot) {
        ctx.fillStyle = "rgba(180, 180, 180, 0.7)";
        ctx.font = "10px sans-serif";
        ctx.textAlign = "center";
        ctx.fillText("暖身", cx, numY);
      }
    }

    rafId = requestAnimationFrame(animate);
  }

  onDestroy(() => {
    cancelAnimationFrame(rafId);
  });
</script>

{#if isRunning || $calibrationStatus.beats.length > 0}
  <div class="visualizer-overlay">
    <canvas
      bind:this={canvasEl}
      width="640"
      height="440"
      class="calibration-canvas"
    ></canvas>
  </div>
{/if}

<style>
  .visualizer-overlay {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    border-radius: 12px;
    background: rgba(26, 23, 20, 0.95);
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    backdrop-filter: blur(4px);
  }

  .calibration-canvas {
    width: min(640px, 90vw);
    height: min(440px, 70vh);
    background: transparent;
  }
</style>
