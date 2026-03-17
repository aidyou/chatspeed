<template>
  <div v-if="visible" class="status-notifier" :class="[category, { active: isRunning }]">
    <div class="notifier-content">
      <cs v-if="category === 'warning'" name="warning" size="14px" class="status-icon" />
      <cs v-else-if="isRunning" name="loading" size="14px" class="status-icon rotating" />
      <cs v-else name="info" size="14px" class="status-icon" />

      <transition name="fade-slide" mode="out-in">
        <span :key="displayMessage" class="status-message">{{ displayMessage }}</span>
      </transition>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue';
import { useI18n } from 'vue-i18n';
import { useWorkflowStore } from '@/stores/workflow';

const { locale } = useI18n();
const workflowStore = useWorkflowStore();

const visible = computed(() => workflowStore.isRunning || workflowStore.notification.message);
const isRunning = computed(() => workflowStore.isRunning);
// const visible = ref(true);
// const isRunning = ref(true);
const category = computed(() => workflowStore.notification.category || 'info');

// Funny messages repository
const funnyMessages = {
  zh: {
    thinking: [
      "水煮活鱼是煮活鱼还是煮死鱼呢，让我想想...",
      "正在思考宇宙的终极答案，顺便算算你的代码...",
      "思维火花正在碰撞，希望不要擦出火灾...",
      "逻辑电路高速运转中，闻到香味了吗？那是智慧的味道。",
      "我的 CPU 正在热身，准备给你一个惊掉下巴的方案。",
      "正在查阅《智能体修仙指南》，这一步有点玄乎...",
      "思考中... 别打扰，万一想出永动机了呢？",
      "深度思考中，目前的进度是：1 + 1 暂时等于 2。",
      "我观你代码中有真龙之气，待我推演一番...",
      "正在大脑中模拟三千个平行世界，只为找一个 Bug...",
      "思维进入量子叠加态，直到我想出来之前，方案既是完美的也是错的。",
      "正在从赛博虚空中汲取灵感...",
      "思考中，顺便在后台偷偷玩了一局扫雷。",
      "如果思考有颜色，我现在的头顶应该是五彩斑斓的黑。",
      "正在加载智慧包，当前的网速有点感人..."
    ],
    acting: [
      "我正在前往火星，那里可能有你需要的东西...",
      "正在赛博空间进行特种作战，目标：任务目标。",
      "代码正在工位上疯狂奔跑，希望它不要摔跤...",
      "正在搬运字节，这些 0 和 1 真的好沉。",
      "开始执行！现在我是这条街最靓的执行仔。",
      "正在和编译器进行友好磋商...",
      "正在穿越防火墙，这比穿越火线还刺激。",
      "行动中！我已经预感到胜利在向我们招手了。",
      "正在向服务器发送一波强势输出...",
      "代码正在疯狂生长，希望不要长成一棵歪脖子树。",
      "正在执行高难度动作，请勿模仿。",
      "我正在数字海洋里冲浪，顺便捞一下你的需求。",
      "正在加速奔跑，感觉自己快要超光速了。",
      "正在键盘上跳舞，希望能敲出优美的旋律。",
      "别担心，我办事，你放心（大概）。"
    ],
    observing: [
      "我观你骨骼清奇，是炼丹的好苗子...",
      "正在数字丛林中寻找蛛丝马迹...",
      "正在扫描数字世界的每一个角落...",
      "真相只有一个！让我再仔细瞧瞧...",
      "我正在开启天眼，洞察这一切的本质。",
      "观察中... 发现了一处有趣的数字遗迹。",
      "我的探测器正在发回信号，似乎有些不寻常。",
      "正在透过表象看本质，这一层滤镜有点厚。",
      "发现目标！它似乎想躲在代码注释里。",
      "正在进行全方位的雷达扫描...",
      "我观这代码，五行缺金，得补一下。",
      "正在分析战果，这一波不亏。",
      "正在数字星空中寻找那一颗最亮的星。",
      "观察完毕，心中的答案呼之欲出。",
      "一切都在掌控之中，至少在我的镜头里是这样。"
    ]
  },
  en: {
    thinking: [
      "Is boiled fish boiled alive or dead? Let me think...",
      "Thinking about the ultimate answer to the universe, and your code...",
      "Sparking ideas... hopefully not starting a fire.",
      "Logic circuits running at high speed. Smells like wisdom.",
      "My CPU is warming up, preparing to blow your mind.",
      "Reading 'The Agent's Guide to Immortality', this step is tricky...",
      "Thinking... Don't interrupt, I might invent a perpetual motion machine.",
      "Deep thinking... Progress: 1 + 1 temporarily equals 2.",
      "I see a powerful aura in your code, let me derive it...",
      "Simulating 3,000 parallel worlds to find one bug...",
      "My thoughts are in quantum superposition... the solution is both perfect and wrong.",
      "Drawing inspiration from the cyber void...",
      "Thinking... also secretly playing Minesweeper in the background.",
      "If thoughts had colors, I'd be thinking in 'vibrant black'.",
      "Loading wisdom pack... current speed is very nostalgic."
    ],
    acting: [
      "I'm heading to Mars, what you need might be there...",
      "Conducting special ops in cyberspace. Target acquired.",
      "Code is running wild in the office, hope it doesn't trip...",
      "Moving bytes around. These 0s and 1s are heavier than they look.",
      "Executing! I'm the coolest executor on this digital block.",
      "Engaging in friendly negotiations with the compiler...",
      "Crossing the firewall. More exciting than crossing the street.",
      "Action! I can sense victory waving at us.",
      "Sending a powerful burst of data to the server...",
      "Code is growing fast, hope it doesn't turn into a tangled vine.",
      "Performing high-difficulty maneuvers. Don't try this at home.",
      "Surfing the digital ocean to catch your requirements.",
      "Accelerating... I feel like I'm reaching light speed.",
      "Dancing on the keyboard, hoping for a beautiful melody.",
      "Don't worry, I've got this (probably)."
    ],
    observing: [
      "I see you have great potential for digital alchemy...",
      "Looking for clues in the digital jungle...",
      "Scanning every corner of the digital world...",
      "There is only one truth! Let me look closer...",
      "Opening my third eye to see the essence of everything.",
      "Observing... discovered an interesting digital ruin.",
      "Sensor feedback incoming, something seems unusual.",
      "Looking past the surface, this filter is quite thick.",
      "Target found! It seems to be hiding in the comments.",
      "Performing full-range radar scan...",
      "This code lacks 'Metal' in its five elements, needs correction.",
      "Analyzing results, this was a good move.",
      "Searching for the brightest star in the digital sky.",
      "Observation complete, the answer is imminent.",
      "Everything is under control, at least through my lens."
    ]
  }
};

const currentFunnyMessage = ref('');
const randomTimer = ref(null);

const displayMessage = computed(() => {
  // Priority 1: Direct notification from backend
  if (workflowStore.notification.message) {
    return workflowStore.notification.message;
  }
  // Priority 2: Funny message based on state
  return currentFunnyMessage.value;
});

const getMessagePool = () => {
  const lang = locale.value.startsWith('zh') ? 'zh' : 'en';
  const pool = funnyMessages[lang];

  // Map internal state to funny pool keys
  const state = workflowStore.currentWorkflow?.status?.toLowerCase() || 'thinking';
  if (state.includes('executing') || state.includes('acting')) return pool.acting;
  if (state.includes('observing') || state.includes('auditing')) return pool.observing;
  return pool.thinking;
};

const updateRandomMessage = () => {
  const pool = getMessagePool();
  const index = Math.floor(Math.random() * pool.length);
  currentFunnyMessage.value = pool[index];
};

// Auto-update message every 8 seconds when running and no direct notification
const startRandomizer = () => {
  stopRandomizer();
  updateRandomMessage();
  randomTimer.value = setInterval(() => {
    if (!workflowStore.notification.message) {
      updateRandomMessage();
    }
  }, 8000);
};

const stopRandomizer = () => {
  if (randomTimer.value) {
    clearInterval(randomTimer.value);
    randomTimer.value = null;
  }
};

// Reset notification after 10 seconds if it's not a persistent one (like compression or retrying)
watch(() => workflowStore.notification.timestamp, () => {
  if (workflowStore.notification.message && !workflowStore.isRunning) {
    setTimeout(() => {
      workflowStore.setNotification('', 'info');
    }, 10000);
  }
});

watch(isRunning, (newVal) => {
  if (newVal) {
    startRandomizer();
  } else {
    stopRandomizer();
  }
}, { immediate: true });

onMounted(() => {
  if (isRunning.value) startRandomizer();
});

onBeforeUnmount(() => {
  stopRandomizer();
});
</script>

<style lang="scss" scoped>
.status-notifier {
  margin: auto var(--cs-space);
  padding: var(--cs-space-xs) var(--cs-space);
  background: var(--cs-bg-color-overlay);
  border-bottom: 1px solid var(--cs-border-color-light);
  font-size: 12px;
  color: var(--cs-text-color-secondary);
  min-height: 32px;
  display: flex;
  align-items: center;
  overflow: hidden;
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  z-index: 10;
  backdrop-filter: blur(8px);
  transition: all 0.3s ease;
  opacity: 0;
  transform: translateY(-100%);

  &.active {
    opacity: 1;
    transform: translateY(0);
  }

  &.warning {
    color: var(--el-color-warning);
    background: var(--el-color-warning-light-9);
  }

  &.error {
    color: var(--el-color-danger);
    background: var(--el-color-danger-light-9);
  }

  .notifier-content {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
  }

  .status-icon {
    flex-shrink: 0;
  }

  .status-message {
    white-space: nowrap;
    text-overflow: ellipsis;
    overflow: hidden;
  }
}

.rotating {
  animation: rotate 2s linear infinite;
}

@keyframes rotate {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}

.fade-slide-enter-active,
.fade-slide-leave-active {
  transition: all 0.3s ease;
}

.fade-slide-enter-from {
  opacity: 0;
  transform: translateY(10px);
}

.fade-slide-leave-to {
  opacity: 0;
  transform: translateY(-10px);
}
</style>
