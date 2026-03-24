<template>
  <section class="stats-bar">
    <div class="stats-inner">
      <div class="stat-item">
        <span class="stat-icon">&#9889;</span>
        <span class="stat-text">Rust-powered</span>
      </div>
      <span class="stat-divider" aria-hidden="true"></span>
      <div class="stat-item">
        <span class="stat-icon">&#128202;</span>
        <span class="stat-text">OpenTelemetry native</span>
      </div>
      <span class="stat-divider" aria-hidden="true"></span>
      <div class="stat-item">
        <span class="stat-icon">&#128256;</span>
        <span class="stat-text">1 port &#8594; &#8734; servers</span>
      </div>
    </div>
  </section>
</template>

<style scoped>
.stats-bar {
  width: 100%;
  background: var(--ir-bg-dark);
  border-top: 1px solid var(--ir-border);
  border-bottom: 1px solid var(--ir-border);
  padding: 20px 24px;
}

.stats-inner {
  max-width: 1200px;
  margin: 0 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 32px;
}

.stat-item {
  display: flex;
  align-items: center;
  gap: 8px;
}

.stat-icon {
  font-size: 18px;
}

.stat-text {
  font-family: var(--ir-font-sans);
  font-weight: 600;
  font-size: 15px;
  color: rgba(240, 237, 232, 0.7);
}

.stat-divider {
  width: 1px;
  height: 20px;
  background: var(--ir-border);
}

@media (max-width: 767px) {
  .stats-inner {
    flex-direction: column;
    gap: 14px;
  }

  .stat-divider {
    display: none;
  }
}
</style>
