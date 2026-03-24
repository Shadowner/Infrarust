<script setup>
import DefaultTheme from 'vitepress/theme'
import V2Banner from './V2Banner.vue'

const { Layout } = DefaultTheme
</script>

<template>
  <Layout>
    <template #layout-top>
      <V2Banner />
    </template>
  </Layout>
</template>
