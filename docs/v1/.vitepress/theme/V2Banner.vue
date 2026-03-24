<script setup>
import { ref, onMounted, onUnmounted, watch } from 'vue'

const STORAGE_KEY = 'infrarust-v2-banner-dismissed'
const dismissed = ref(typeof sessionStorage !== 'undefined' && sessionStorage.getItem(STORAGE_KEY) === 'true')

const BANNER_HEIGHT = '40px'

function updateBannerHeight(show) {
  document.documentElement.style.setProperty(
    '--vp-layout-top-height',
    show ? BANNER_HEIGHT : '0px'
  )
}

onMounted(() => {
  updateBannerHeight(!dismissed.value)
})

onUnmounted(() => {
  updateBannerHeight(false)
})

function dismiss() {
  dismissed.value = true
  sessionStorage.setItem(STORAGE_KEY, 'true')
  updateBannerHeight(false)
}

function goToV2(e) {
  e.preventDefault()
  window.location.href = '/v2/'
}
</script>

<template>
  <div v-if="!dismissed" class="v2-banner">
    <div class="v2-banner-content">
      <span>
        🚀 <strong>Infrarust V2 is now in alpha!</strong>
        Check out the <a href="/v2/" @click="goToV2">new documentation</a>.
        V1 will be archived once V2 is released.
      </span>
      <button class="v2-banner-close" aria-label="Dismiss banner" @click="dismiss">✕</button>
    </div>
  </div>
</template>

<style scoped>
.v2-banner {
  background: linear-gradient(135deg, #d2691e, #cd853f);
  color: #fff;
  font-size: 0.9rem;
  line-height: 1.4;
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  z-index: 30;
  height: v-bind(BANNER_HEIGHT);
}

.v2-banner-content {
  max-width: 1200px;
  margin: 0 auto;
  padding: 0.6rem 2.5rem 0.6rem 1rem;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
  text-align: center;
}

.v2-banner a {
  color: #fff;
  font-weight: 700;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.v2-banner a:hover {
  text-decoration-thickness: 2px;
}

.v2-banner-close {
  position: absolute;
  right: 0.75rem;
  top: 50%;
  transform: translateY(-50%);
  background: none;
  border: none;
  color: #fff;
  font-size: 1rem;
  cursor: pointer;
  padding: 0.25rem 0.5rem;
  opacity: 0.8;
  line-height: 1;
}

.v2-banner-close:hover {
  opacity: 1;
}
</style>
