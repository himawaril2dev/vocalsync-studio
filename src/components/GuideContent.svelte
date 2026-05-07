<script lang="ts">
  import { t } from "../i18n";
</script>

<div class="guide-content">
  <h2>{$t("startupGuide.title")}</h2>
  <p class="guide-intro">{$t("startupGuide.subtitle")}</p>

  <div class="guide-steps">
    <article>
      <span class="step-num">1</span>
      <div>
        <h3>{$t("startupGuide.step1.title")}</h3>
        <p>{$t("startupGuide.step1.body")}</p>
      </div>
    </article>
    <article>
      <span class="step-num">2</span>
      <div>
        <h3>{$t("startupGuide.step2.title")}</h3>
        <p>{$t("startupGuide.step2.body")}</p>
      </div>
    </article>
    <article>
      <span class="step-num">3</span>
      <div>
        <h3>{$t("startupGuide.step3.title")}</h3>
        <p>{$t("startupGuide.step3.body")}</p>
      </div>
    </article>
    <article>
      <span class="step-num">4</span>
      <div>
        <h3>{$t("startupGuide.step4.title")}</h3>
        <p>{$t("startupGuide.step4.body")}</p>
      </div>
    </article>
    <article>
      <span class="step-num">5</span>
      <div>
        <h3>{$t("startupGuide.step5.title")}</h3>
        <p>{$t("startupGuide.step5.body")}</p>
      </div>
    </article>
  </div>

  <div class="guide-note">
    <strong>{$t("startupGuide.tip.title")}</strong>
    <p>{$t("startupGuide.tip.body")}</p>
  </div>
</div>

<style>
  .guide-content {
    color: #3d3630;
  }

  h2 {
    margin: 0;
    font-size: 20px;
    color: #3d3630;
  }

  .guide-intro {
    margin: 8px 0 18px;
    color: #6f655b;
    font-size: 14px;
    line-height: 1.8;
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

  p {
    margin: 0;
    color: #6f655b;
    font-size: 13px;
    line-height: 1.75;
  }

  .guide-note {
    margin-top: 16px;
    padding: 16px 18px;
    border-radius: 16px;
    background: rgba(253, 192, 3, 0.14);
    border: 1px solid rgba(253, 192, 3, 0.35);
  }

  .guide-note strong {
    display: block;
    margin-bottom: 6px;
    font-size: 14px;
    color: #3d3630;
  }
</style>
