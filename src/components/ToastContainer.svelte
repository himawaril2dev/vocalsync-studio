<script lang="ts">
  import { toasts, dismissToast } from "../stores/toast";
</script>

{#if $toasts.length > 0}
  <div class="toast-container" role="status" aria-live="polite">
    {#each $toasts as toast (toast.id)}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="toast toast-{toast.level}" onclick={() => dismissToast(toast.id)}>
        <span class="toast-icon">
          {#if toast.level === "success"}
            &#10003;
          {:else if toast.level === "error"}
            &#10007;
          {:else if toast.level === "warning"}
            &#9888;
          {:else}
            &#8505;
          {/if}
        </span>
        <span class="toast-message">{toast.message}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-container {
    position: fixed;
    bottom: var(--space-lg, 16px);
    right: var(--space-lg, 16px);
    display: flex;
    flex-direction: column-reverse;
    gap: var(--space-sm, 8px);
    z-index: 9999;
    pointer-events: none;
    max-width: 400px;
  }

  .toast {
    display: flex;
    align-items: center;
    gap: var(--space-sm, 8px);
    padding: var(--space-md, 12px) var(--space-lg, 16px);
    border-radius: var(--radius-md, 8px);
    font-size: 13px;
    color: var(--color-text, #3d3630);
    background: var(--color-bg-surface, #fff);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.12);
    pointer-events: auto;
    cursor: pointer;
    animation: toast-in 0.25s ease-out;
  }

  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .toast-icon {
    font-size: 16px;
    flex-shrink: 0;
  }

  .toast-message {
    flex: 1;
    line-height: 1.4;
  }

  .toast-info {
    border-left: 4px solid var(--color-info, #2563eb);
  }

  .toast-success {
    border-left: 4px solid var(--color-success, #00c853);
  }

  .toast-warning {
    border-left: 4px solid var(--color-accent, #fdc003);
  }

  .toast-error {
    border-left: 4px solid var(--color-danger, #c0392b);
  }
</style>
