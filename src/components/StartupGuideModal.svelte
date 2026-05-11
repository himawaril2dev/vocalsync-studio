<script lang="ts">
  import { t } from "../i18n";
  import GuideContent from "./GuideContent.svelte";
  import UvrGuideModal from "./UvrGuideModal.svelte";

  interface Props {
    showNextLaunch: boolean;
    onClose: (showNextLaunch: boolean) => void;
  }

  let { showNextLaunch = true, onClose }: Props = $props();
  let localShowNextLaunch = $state(true);
  let showUvrGuide = $state(false);

  $effect(() => {
    localShowNextLaunch = showNextLaunch;
  });
</script>

<div class="guide-backdrop" role="presentation">
  <div class="guide-modal" role="dialog" aria-modal="true" aria-label="VocalSync Studio guide">
    <header class="guide-header">
      <span class="eyebrow">VocalSync Studio</span>
    </header>

    <GuideContent />

    <button class="uvr-guide-entry" type="button" onclick={() => (showUvrGuide = true)}>
      <div>
        <strong>{$t("uvrGuide.entry.title")}</strong>
        <p>{$t("uvrGuide.entry.body")}</p>
      </div>
      <span>{$t("uvrGuide.entry.action")}</span>
    </button>

    <footer class="guide-footer">
      <label class="show-again">
        <input type="checkbox" bind:checked={localShowNextLaunch} />
        <span>{$t("startupGuide.showAgain")}</span>
      </label>
      <button class="primary-action" onclick={() => onClose(localShowNextLaunch)}>
        {$t("startupGuide.close")}
      </button>
    </footer>
  </div>
</div>

{#if showUvrGuide}
  <UvrGuideModal onClose={() => (showUvrGuide = false)} />
{/if}

<style>
  .guide-backdrop {
    position: fixed;
    inset: 0;
    z-index: 10000;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(33, 27, 21, 0.58);
    backdrop-filter: blur(8px);
  }

  .guide-modal {
    width: min(920px, 100%);
    max-height: min(760px, calc(100vh - 48px));
    overflow-y: auto;
    border-radius: 22px;
    background:
      radial-gradient(circle at top left, rgba(253, 192, 3, 0.18), transparent 36%),
      #fffaf1;
    box-shadow: 0 28px 80px rgba(0, 0, 0, 0.35);
    padding: 28px;
    border: 1px solid rgba(117, 87, 0, 0.14);
  }

  .guide-header {
    text-align: center;
    margin: 0 auto 10px;
  }

  .eyebrow {
    display: inline-block;
    color: #9a7a12;
    font-size: 12px;
    font-weight: 800;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    margin-bottom: 8px;
  }

  .uvr-guide-entry {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    width: 100%;
    margin-top: 18px;
    padding: 16px 18px;
    border: 1px solid rgba(159, 122, 0, 0.28);
    border-radius: 16px;
    background:
      radial-gradient(circle at top left, rgba(253, 192, 3, 0.16), transparent 34%),
      #fffaf1;
    color: #3d3630;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      border-color 0.16s ease,
      transform 0.16s ease,
      box-shadow 0.16s ease;
  }

  .uvr-guide-entry:hover {
    border-color: rgba(117, 87, 0, 0.46);
    box-shadow: 0 12px 28px rgba(117, 87, 0, 0.12);
    transform: translateY(-1px);
  }

  .uvr-guide-entry strong {
    display: block;
    margin-bottom: 5px;
    color: #3d3630;
    font-size: 15px;
  }

  .uvr-guide-entry p {
    margin: 0;
    color: #6f655b;
    font-size: 13px;
    line-height: 1.65;
  }

  .uvr-guide-entry > span {
    flex: 0 0 auto;
    padding: 9px 14px;
    border-radius: 999px;
    background: #755700;
    color: #fff;
    font-size: 12px;
    font-weight: 700;
    white-space: nowrap;
  }

  .guide-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 16px;
    margin-top: 20px;
  }

  .show-again {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: #5f554b;
    font-size: 13px;
    cursor: pointer;
  }

  .show-again input {
    accent-color: #755700;
  }

  .primary-action {
    border: none;
    border-radius: 12px;
    background: #755700;
    color: #fff;
    padding: 11px 24px;
    font-size: 14px;
    font-weight: 700;
    cursor: pointer;
  }

  .primary-action:hover {
    background: #5c4400;
  }

  @media (max-width: 760px) {
    .guide-modal {
      padding: 20px;
    }

    .uvr-guide-entry {
      align-items: stretch;
      flex-direction: column;
    }

    .uvr-guide-entry > span {
      align-self: flex-start;
    }

    .guide-footer {
      align-items: stretch;
      flex-direction: column;
    }
  }
</style>
