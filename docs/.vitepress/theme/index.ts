import { h } from 'vue'
import type { Theme } from 'vitepress'
import DefaultTheme from 'vitepress/theme'
import './fonts.css'
import './style.css'
import HomeLayout from './HomeLayout.vue'
import LicenseLayout from './components/LicenseLayout.vue'

export default {
    extends: DefaultTheme,
    Layout: () => {
        return h(DefaultTheme.Layout, null, {})
    },
    enhanceApp({ app, router, siteData }) {
        app.component('HomeLayout', HomeLayout)
        app.component('LicenseLayout', LicenseLayout)
    }
} satisfies Theme
