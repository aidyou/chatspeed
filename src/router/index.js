import { createRouter, createWebHistory } from 'vue-router'

const routes = [
  {
    path: "/",
    name: "index",
    component: () => import("@/views/Index.vue"),
  },
  {
    path: "/settings/:type?",
    name: "settings",
    component: () => import("@/views/Settings.vue"),
  },
  {
    path: "/assistant",
    name: "assistant",
    component: () => import("@/views/Assistant.vue"),
  },
  {
    path: "/toolbar",
    name: "toolbar",
    component: () => import("@/views/Toolbar.vue"),
  },
  {
    path: "/note",
    name: "note",
    component: () => import("@/views/Note.vue"),
  },
  {
    path: "/workflow",
    name: "workflow",
    component: () => import("@/views/Workflow.vue"),
  },
  {
    path: "/proxy-switcher",
    name: "proxy-switcher",
    component: () => import("@/views/ProxySwitcher.vue"),
  },
];

const router = createRouter({
  history: createWebHistory(),
  routes
})

export default router
