<script lang="ts">
  import { locale, setLocale, t, LOCALES, type Locale } from "../i18n";

  function onChange(e: Event) {
    const value = (e.currentTarget as HTMLSelectElement).value as Locale;
    setLocale(value);
  }
</script>

<div class="lang-switcher">
  <svg
    class="lang-icon"
    width="14"
    height="14"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <circle cx="12" cy="12" r="10" />
    <line x1="2" y1="12" x2="22" y2="12" />
    <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
  </svg>
  <select
    class="lang-select"
    value={$locale}
    onchange={onChange}
    aria-label={$t("language.switcher.aria")}
  >
    {#each LOCALES as option (option.code)}
      <option value={option.code}>{option.label}</option>
    {/each}
  </select>
</div>

<style>
  .lang-switcher {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    margin: 0 10px var(--space-sm);
    color: var(--color-text-muted);
    border-radius: var(--radius-md);
    transition: background var(--transition-fast), color var(--transition-fast);
  }

  .lang-switcher:hover {
    background: var(--color-bg-hover);
    color: var(--color-text-secondary);
  }

  .lang-icon {
    flex-shrink: 0;
  }

  .lang-select {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: inherit;
    font-size: 12px;
    cursor: pointer;
    padding: 2px 0;
    appearance: none;
    -webkit-appearance: none;
    outline: none;
  }

  .lang-select:focus-visible {
    outline: 2px solid var(--color-brand);
    outline-offset: 2px;
    border-radius: var(--radius-sm);
  }

  .lang-select option {
    color: var(--color-text);
    background: var(--color-bg-surface);
  }
</style>
