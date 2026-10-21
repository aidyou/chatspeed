import { createApp } from "vue";
import { createPinia } from 'pinia'

import App from "./App.vue";
import router from './router'
import i18n from './i18n'

import '@/components/icon/chatspeed.css'
import cs from '@/components/icon/Icon.vue'
import logo from '@/components/icon/Logo.vue'
import avatar from '@/components/common/Avatar.vue'

// Element Plus MessageBox styles
import 'element-plus/es/components/message-box/style/css'
import 'element-plus/es/components/overlay/style/css'

import { registerDirective } from '@/libs/directive'
import { setUserErrorFallback } from '@/libs/error'

// Store error fields are rendered outside a component, so they need the same
// locale-aware fallback the Tauri wrapper uses for command errors.
setUserErrorFallback(() => i18n.global.t('common.errorFallback'))

const app = createApp(App)
app.use(createPinia())
app.use(router)
app.use(i18n)
app.component('cs', cs)
app.component('logo', logo)
app.component('avatar', avatar)

registerDirective(app)
app.mount("#app");
