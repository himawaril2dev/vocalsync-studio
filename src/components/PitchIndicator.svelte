<script lang="ts">
  import { currentPitch } from "../stores/pitch";
  import { t, tSync } from "../i18n";

  // 把 cent 對應到水平 bar 的位置（0~100%）
  function centToPosition(cent: number): number {
    const clamped = Math.max(-50, Math.min(50, cent));
    return 50 + clamped;
  }

  // 根據音準偏差決定主色（綠/黃/紅三段）
  function statusColor(cent: number): string {
    const abs = Math.abs(cent);
    if (abs <= 10) return "#27ae60";
    if (abs <= 25) return "#f9a825";
    return "#d50000";
  }

  function statusText(cent: number): string {
    void $t;
    const abs = Math.abs(cent);
    if (abs <= 10) return tSync("pitchIndicator.status.inTune");
    return cent > 0 ? tSync("pitchIndicator.status.high") : tSync("pitchIndicator.status.low");
  }
</script>

<div class="pitch-card">
  {#if $currentPitch}
    <!-- 主視覺：音名 + 八度 -->
    <div class="note-block" style="color: {statusColor($currentPitch.cent)};">
      <span class="note-letter">{$currentPitch.note}</span>
      <span class="note-octave">{$currentPitch.octave}</span>
    </div>

    <!-- 偏差條 + 文字狀態 -->
    <div class="meter-block">
      <div class="status-row">
        <span
          class="status-label"
          style="color: {statusColor($currentPitch.cent)};"
        >
          {($t, statusText($currentPitch.cent))}
        </span>
        <span class="cent-num">
          {$currentPitch.cent > 0 ? "+" : ""}{$currentPitch.cent.toFixed(0)}¢
        </span>
      </div>

      <div class="meter-track">
        <!-- 左半：偏低區 -->
        <div class="meter-half left"></div>
        <!-- 右半：偏高區 -->
        <div class="meter-half right"></div>
        <!-- 中央準音線 -->
        <div class="meter-center"></div>
        <!-- 當前位置指示 -->
        <div
          class="meter-cursor"
          style="left: {centToPosition($currentPitch.cent)}%; background: {statusColor($currentPitch.cent)};"
        ></div>
      </div>

      <div class="freq-row">{$currentPitch.freq.toFixed(1)} Hz</div>
    </div>
  {:else}
    <div class="empty-state">
      <span class="empty-dot"></span>
      <span class="empty-text">{$t("pitchIndicator.empty")}</span>
    </div>
  {/if}
</div>

<style>
  .pitch-card {
    background: #fff;
    border-radius: 10px;
    padding: 14px 20px;
    display: flex;
    align-items: center;
    gap: 24px;
    min-height: 80px;
  }

  /* ── 主視覺：音名 ── */
  .note-block {
    display: flex;
    align-items: baseline;
    gap: 4px;
    min-width: 100px;
    transition: color 0.2s ease;
  }

  .note-letter {
    font-family: "Consolas", "GenSenRounded2 PJP B", monospace;
    font-size: 56px;
    font-weight: 700;
    line-height: 1;
    letter-spacing: -2px;
  }

  .note-octave {
    font-family: "Consolas", monospace;
    font-size: 24px;
    font-weight: 500;
    opacity: 0.7;
  }

  /* ── 偏差條 ── */
  .meter-block {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .status-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .status-label {
    font-size: 14px;
    font-weight: 600;
    transition: color 0.2s ease;
  }

  .cent-num {
    font-family: "Consolas", monospace;
    font-size: 13px;
    color: #b0a898;
  }

  .meter-track {
    position: relative;
    height: 10px;
    border-radius: 5px;
    overflow: hidden;
    background: #f0ece4;
  }

  .meter-half {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 50%;
    opacity: 0.15;
  }

  .meter-half.left {
    left: 0;
    background: linear-gradient(to right, #d50000, transparent);
  }

  .meter-half.right {
    right: 0;
    background: linear-gradient(to left, #d50000, transparent);
  }

  .meter-center {
    position: absolute;
    top: -2px;
    bottom: -2px;
    left: 50%;
    width: 2px;
    background: #755700;
    transform: translateX(-50%);
    opacity: 0.5;
  }

  .meter-cursor {
    position: absolute;
    top: 50%;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    transform: translate(-50%, -50%);
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.25);
    border: 2px solid #fff;
    transition:
      left 0.08s ease-out,
      background 0.2s ease;
  }

  .freq-row {
    font-family: "Consolas", monospace;
    font-size: 11px;
    color: #d0ccc4;
    text-align: right;
  }

  /* ── 空狀態 ── */
  .empty-state {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: #d0ccc4;
  }

  .empty-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #e8e2d8;
  }

  .empty-text {
    font-size: 13px;
  }
</style>
