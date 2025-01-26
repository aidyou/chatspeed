import { createApp } from "vue";
import { createPinia } from 'pinia'

import App from "./App.vue";
import router from './router'
import i18n from './i18n'

import '@/components/icon/chatspeed.css'
import cs from '@/components/icon/index.vue'
import logo from '@/components/icon/logo.vue'

import { registerDirective } from '@/libs/directive'


const app = createApp(App)
app.use(createPinia())
app.use(router)
app.use(i18n)
app.component('cs', cs)
app.component('logo', logo)

registerDirective(app)
app.mount("#app");

