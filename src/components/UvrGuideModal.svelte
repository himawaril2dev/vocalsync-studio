<script lang="ts">
  import { t } from "../i18n";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  const UVR_RELEASES_URL = "https://github.com/Anjok07/ultimatevocalremovergui/releases";
</script>

<div class="guide-backdrop" role="presentation">
  <div class="guide-modal" role="dialog" aria-modal="true" aria-label="UVR5 guide">
    <header class="guide-header">
      <span class="eyebrow">UVR5 GUIDE</span>
      <button class="close-btn" type="button" onclick={onClose} aria-label={$t("uvrGuide.close")}>
        x
      </button>
    </header>

    <section class="guide-content">
      <h2>{$t("uvrGuide.title")}</h2>
      <p class="guide-intro">{$t("uvrGuide.subtitle")}</p>

      <div class="safe-download">
        <div>
          <strong>{$t("uvrGuide.safeDownload.title")}</strong>
          <p>{$t("uvrGuide.safeDownload.body")}</p>
        </div>
        <a href={UVR_RELEASES_URL} target="_blank" rel="noopener">
          {$t("uvrGuide.safeDownload.action")}
        </a>
      </div>

      <div class="guide-steps">
        <article>
          <span class="step-num">1</span>
          <div>
            <h3>{$t("uvrGuide.step1.title")}</h3>
            <p>{$t("uvrGuide.step1.body")}</p>
          </div>
        </article>
        <article>
          <span class="step-num">2</span>
          <div>
            <h3>{$t("uvrGuide.step2.title")}</h3>
            <p>{$t("uvrGuide.step2.body")}</p>
          </div>
        </article>
        <article>
          <span class="step-num">3</span>
          <div>
            <h3>{$t("uvrGuide.step3.title")}</h3>
            <p>{$t("uvrGuide.step3.body")}</p>
          </div>
        </article>
        <article>
          <span class="step-num">4</span>
          <div>
            <h3>{$t("uvrGuide.step4.title")}</h3>
            <p>{$t("uvrGuide.step4.body")}</p>
          </div>
        </article>
        <article>
          <span class="step-num">5</span>
          <div>
            <h3>{$t("uvrGuide.step5.title")}</h3>
            <p>{$t("uvrGuide.step5.body")}</p>
          </div>
        </article>
        <article>
          <span class="step-num">6</span>
          <div>
            <h3>{$t("uvrGuide.step6.title")}</h3>
            <p>{$t("uvrGuide.step6.body")}</p>
          </div>
        </article>
      </div>

      <section class="faq">
        <h3>{$t("uvrGuide.faq.title")}</h3>
        <div class="faq-grid">
          <div>
            <strong>{$t("uvrGuide.faq.output.title")}</strong>
            <p>{$t("uvrGuide.faq.output.body")}</p>
          </div>
          <div>
            <strong>{$t("uvrGuide.faq.slow.title")}</strong>
            <p>{$t("uvrGuide.faq.slow.body")}</p>
          </div>
          <div>
            <strong>{$t("uvrGuide.faq.noise.title")}</strong>
            <p>{$t("uvrGuide.faq.noise.body")}</p>
          </div>
          <div>
            <strong>{$t("uvrGuide.faq.sync.title")}</strong>
            <p>{$t("uvrGuide.faq.sync.body")}</p>
          </div>
        </div>
      </section>
    </section>

    <footer class="guide-footer">
      <button class="primary-action" type="button" onclick={onClose}>
        {$t("uvrGuide.close")}
      </button>
    </footer>
  </div>
</div>

<style>
  .guide-backdrop {
    position: fixed;
    inset: 0;
    z-index: 10001;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(33, 27, 21, 0.58);
    backdrop-filter: blur(8px);
  }

  .guide-modal {
    width: min(960px, 100%);
    max-height: min(800px, calc(100vh - 48px));
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
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 10px;
  }

  .eyebrow {
    color: #9a7a12;
    font-size: 12px;
    font-weight: 800;
    letter-spacing: 0.18em;
    text-transform: uppercase;
  }

  .close-btn {
    width: 34px;
    height: 34px;
    border: none;
    border-radius: 50%;
    background: #f0ece4;
    color: #5c5248;
    font-size: 22px;
    line-height: 1;
    cursor: pointer;
  }

  .guide-content {
    color: #3d3630;
  }

  h2 {
    margin: 0;
    font-size: 22px;
    color: #3d3630;
  }

  .guide-intro {
    margin: 8px 0 18px;
    color: #6f655b;
    font-size: 14px;
    line-height: 1.8;
  }

  .safe-download {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    align-items: center;
    margin-bottom: 16px;
    padding: 16px 18px;
    border-radius: 16px;
    background: rgba(253, 192, 3, 0.14);
    border: 1px solid rgba(253, 192, 3, 0.35);
  }

  .safe-download strong,
  .faq strong {
    display: block;
    margin-bottom: 6px;
    color: #3d3630;
    font-size: 14px;
  }

  .safe-download p,
  .faq p {
    margin: 0;
    color: #6f655b;
    font-size: 13px;
    line-height: 1.75;
  }

  .safe-download a {
    flex: 0 0 auto;
    padding: 10px 16px;
    border-radius: 12px;
    background: #755700;
    color: #fff;
    text-decoration: none;
    font-size: 13px;
    font-weight: 700;
  }

  .guide-steps {
    display: grid;
    gap: 14px;
  }

  article {
    display: flex;
    gap: 14px;
    padding: 16px 18px;
    border-radius: 16px;
    background: #fdfaf5;
    border: 1px solid rgba(208, 190, 150, 0.55);
  }

  .step-num {
    flex: 0 0 auto;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: #755700;
    color: #fff;
    font-size: 14px;
    font-weight: 800;
  }

  h3 {
    margin: 1px 0 6px;
    font-size: 15px;
    color: #3d3630;
  }

  article p {
    margin: 0;
    color: #6f655b;
    font-size: 13px;
    line-height: 1.75;
  }

  .faq {
    margin-top: 16px;
  }

  .faq > h3 {
    margin: 0 0 10px;
  }

  .faq-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
  }

  .faq-grid > div {
    padding: 14px 16px;
    border-radius: 14px;
    background: #fff;
    border: 1px solid #e8e2d8;
  }

  .guide-footer {
    display: flex;
    justify-content: flex-end;
    margin-top: 20px;
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

  @media (max-width: 760px) {
    .guide-modal {
      padding: 20px;
    }

    .safe-download {
      align-items: stretch;
      flex-direction: column;
    }

    .faq-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
