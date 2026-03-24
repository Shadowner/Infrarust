<script setup>
import SiteNavbar from './components/SiteNavbar.vue'
import HeroSection from './components/HeroSection.vue'
import StatsBar from './components/StatsBar.vue'
import FeaturesSection from './components/FeaturesSection.vue'
import CodeSection from './components/CodeSection.vue'
import HowItWorks from './components/HowItWorks.vue'
import GitHubCTA from './components/GitHubCTA.vue'
import SiteFooter from './components/SiteFooter.vue'
</script>

<template>
  <div class="home-layout">
    <SiteNavbar />
    <HeroSection />
    <StatsBar />
    <FeaturesSection />
    <CodeSection />
    <HowItWorks />
    <GitHubCTA />
    <SiteFooter />
  </div>
</template>

<style scoped>
.home-layout {
  font-family: var(--ir-font-sans);
  color: var(--ir-text);
  background: var(--ir-bg);
  overflow-x: hidden;
  min-height: 100vh;
}
</style>
