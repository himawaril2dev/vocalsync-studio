<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { get } from "svelte/store";
  import {
    inputDeviceIndex,
    outputDeviceIndex,
    latencyMs,
  } from "../stores/settings";
  import { loadedMedia } from "../stores/media";
  import { t, tSync } from "../i18n";

  interface DeviceInfo {
    name: string;
    index: number;
    is_default: boolean;
  }

  interface DeviceList {
    input_devices: DeviceInfo[];
    output_devices: DeviceInfo[];
  }

  interface Props {
    title: string;
    description?: string;
    disabled?: boolean;
    compact?: boolean;
    showStatus?: boolean;
  }

  let {
    title,
    description = "",
    disabled = false,
    compact = false,
    showStatus = true,
  }: Props = $props();

  const LATENCY_MIN_MS = 0;
  const LATENCY_MAX_MS = 5000;
  const LATENCY_NUDGES = [-20, -10, -5, 5, 10, 20];

  let devices = $state<DeviceList | null>(null);
  let saveTimer: number | null = null;
  let saveRevision = 0;
  let statusText = $state("");
  let statusKind = $state<"idle" | "success" | "error">("idle");

  function clampLatency(value: number): number {
    return Math.max(LATENCY_MIN_MS, Math.min(LATENCY_MAX_MS, Math.round(value)));
  }

  function selectedDeviceName(
    list: DeviceInfo[] | undefined,
    index: number | null,
  ): string | null {
    if (index === null) return null;
    return list?.find((device) => device.index === index)?.name ?? null;
  }

  function currentSampleRate(): number {
    return get(loadedMedia)?.sample_rate ?? 44_100;
  }

  function clearSaveTimer(): void {
    if (saveTimer !== null) {
      window.clearTimeout(saveTimer);
      saveTimer = null;
    }
  }

  async function updateRuntimeLatency(latency: number): Promise<void> {
    try {
      await invoke("update_runtime_latency", { latencyMs: latency });
    } catch (error) {
      console.warn("[latency] runtime update failed:", error);
    }
  }

  async function persistManualLatency(latency: number): Promise<void> {
    const revision = ++saveRevision;
    try {
      await invoke("update_calibrated_latency", {
        latencyMs: latency,
        inputDeviceName: selectedDeviceName(devices?.input_devices, get(inputDeviceIndex)),
        outputDeviceName: selectedDeviceName(devices?.output_devices, get(outputDeviceIndex)),
        sampleRate: currentSampleRate(),
        confidence: "manual",
      });
      if (revision === saveRevision) {
        statusText = tSync("setup.calibration.manual.saved", { ms: latency });
        statusKind = "success";
      }
    } catch (error) {
      if (revision === saveRevision) {
        statusText = tSync("setup.calibration.manual.saveFailed", { error: String(error) });
        statusKind = "error";
      }
    }
  }

  function applyLatency(value: number, saveMode: "debounced" | "immediate"): void {
    const latency = clampLatency(value);
    $latencyMs = latency;
    void updateRuntimeLatency(latency);

    if (saveMode === "immediate") {
      clearSaveTimer();
      void persistManualLatency(latency);
      return;
    }

    clearSaveTimer();
    saveTimer = window.setTimeout(() => {
      saveTimer = null;
      void persistManualLatency(latency);
    }, 250);
  }

  function handleSliderInput(event: Event): void {
    const input = event.currentTarget as HTMLInputElement;
    applyLatency(Number(input.value), "debounced");
  }

  function nudgeLatency(deltaMs: number): void {
    applyLatency($latencyMs + deltaMs, "immediate");
  }

  onMount(() => {
    void invoke<DeviceList>("list_devices")
      .then((result) => {
        devices = result;
      })
      .catch((error) => console.warn("[latency] device list load failed:", error));
  });

  onDestroy(() => {
    clearSaveTimer();
  });
</script>

<section
  class="latency-fine-tune"
  class:compact
  class:disabled
  aria-label={title}
>
  <div class="latency-header">
    <div>
      <strong>{title}</strong>
      {#if description}
        <p>{description}</p>
      {/if}
    </div>
    <span class="latency-value">{$latencyMs} ms</span>
  </div>

  <div class="latency-slider-row">
    <input
      type="range"
      min={LATENCY_MIN_MS}
      max={LATENCY_MAX_MS}
      step="1"
      value={$latencyMs}
      disabled={disabled}
      oninput={handleSliderInput}
      aria-label={title}
    />
  </div>

  <div class="latency-nudge-row" aria-label={$t("setup.calibration.nudge.aria")}>
    {#each LATENCY_NUDGES as delta}
      <button
        type="button"
        onclick={() => nudgeLatency(delta)}
        disabled={disabled}
      >
        {delta > 0 ? "+" : ""}{delta} ms
      </button>
    {/each}
  </div>

  {#if showStatus && statusText}
    <p
      class="latency-status"
      class:success={statusKind === "success"}
      class:error={statusKind === "error"}
    >
      {statusText}
    </p>
  {/if}
</section>

<style>
  .latency-fine-tune {
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    background: var(--color-bg-surface);
    padding: var(--space-md);
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }

  .latency-fine-tune.compact {
    padding: var(--space-sm) var(--space-md);
  }

  .latency-fine-tune.disabled {
    opacity: 0.72;
  }

  .latency-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-md);
  }

  .latency-header strong {
    color: var(--color-text);
    font-size: 0.95rem;
  }

  .latency-header p {
    margin: 2px 0 0;
    color: var(--color-text-light);
    font-size: 0.82rem;
    line-height: 1.35;
  }

  .latency-value {
    color: var(--color-primary);
    font-weight: 800;
    white-space: nowrap;
  }

  .latency-slider-row input {
    width: 100%;
    accent-color: var(--color-primary);
  }

  .latency-nudge-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }

  .latency-nudge-row button {
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-bg);
    color: var(--color-text);
    min-width: 72px;
    height: 32px;
    font-weight: 700;
    cursor: pointer;
  }

  .latency-nudge-row button:hover:not(:disabled) {
    border-color: var(--color-primary);
    color: var(--color-primary);
  }

  .latency-nudge-row button:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }

  .latency-status {
    margin: 0;
    font-size: 0.82rem;
    color: var(--color-text-light);
  }

  .latency-status.success {
    color: var(--color-success);
  }

  .latency-status.error {
    color: var(--color-danger);
  }
</style>
