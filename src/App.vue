<template>
  <div class="app" :data-theme="theme" @dragover.prevent @drop.prevent="onDropFile($event)">
    <!-- abstract：不渲染额外 DOM。naive 组件（输入框/表格/下拉/树）统一跟随明暗主题，
         覆盖色值见 naive-theme.ts（与 styles.css 的 CSS 变量同源） -->
    <NConfigProvider abstract :theme="naiveTheme" :theme-overrides="naiveThemeOverrides">
      <Toolbar
        :has-doc="hasDoc"
        :theme="theme"
        @loaded="loadTextInner($event)"
        @cleared="onCleared"
        @toggle-theme="theme = theme === 'light' ? 'dark' : 'light'"
      />
      <div v-if="errorMsg" class="error-bar">{{ errorMsg }}</div>
      <div class="main">
        <div id="treePane" ref="treePane" class="pane">
          <JsonTree ref="treeRef" :root-id="rootId" />
          <!-- 空态提示：布局常驻，未加载文档时树区给出操作引导 -->
          <div v-if="!hasDoc" class="tree-empty-hint">{{ t("emptyHint") }}</div>
          <div v-if="panelBusy" class="panel-mask" role="status" aria-live="polite">
            <div class="mask-spinner"></div>
            <div class="mask-text">{{ t("processingHint") }}</div>
          </div>
        </div>
        <div
          class="splitter-v"
          title="拖拽调整分栏宽度 · 双击复位"
          @mousedown="splitVDown"
          @dblclick="resetSplit"
        ></div>
        <div id="rawPane" class="pane">
          <RawView ref="rawRef" :text="rawText" :theme="theme" :on-edit="onRawEdit" @loaded="onRawLoaded" />
          <div v-if="panelBusy" class="panel-mask" role="status" aria-live="polite">
            <div class="mask-spinner"></div>
            <div class="mask-text">{{ t("processingHint") }}</div>
          </div>
        </div>
      </div>
      <TableView ref="tableRef" :has-doc="hasDoc" />
      <div class="statusbar">
        <span v-if="hasDoc" class="ok">{{ errorMsg ? t("statusInvalid") : t("statusValid") }}</span>
        <span v-else>{{ t("statusWaiting") }}</span>
        <span>{{ fmtSize(docInfo?.size ?? 0) }}</span>
        <span>UTF-8</span>
        <span>{{ t("parseMs", { ms: parseMs.toFixed(0) }) }}</span>
        <span v-if="detect" class="detect-entry"
              :title="t('detectEntry', { n: detect.candidates.length })"
              @click="detectShow = true">{{ t("detectEntry", { n: detect.candidates.length }) }}</span>
        <span v-if="detect" class="detect-entry" @click="restoreOriginal">
          {{ t("detectOriginal", { size: fmtSize(detect.originalText.length) }) }}</span>
      </div>
      <DetectOverlay :show="detectShow" :candidates="detect?.candidates ?? []"
                     :current-idx="detect?.currentIdx ?? null"
                     @close="detectShow = false" @load="loadDetect" />
    </NConfigProvider>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, provide, ref } from "vue";
import { NConfigProvider, darkTheme } from "naive-ui";
import { api, type Detected, type DocInfo, type FormatOut, type Locate } from "./api";
import { t } from "./i18n";
import { naiveOverrides } from "./naive-theme";
import Toolbar from "./components/Toolbar.vue";
import JsonTree from "./components/JsonTree.vue";
import RawView from "./components/RawView.vue";
import TableView from "./components/TableView.vue";
import DetectOverlay from "./components/DetectOverlay.vue";

const docInfo = ref<DocInfo | null>(null);
const rootId = ref(0);
const hasDoc = ref(false);
const errorMsg = ref<string | null>(null);
const theme = ref<"light" | "dark">("dark");
const rawText = ref("");
const parseMs = ref(0);

// 探测会话：原始全文 + 候选 + 已载入候选下标（null = 未载入过）。
// 生命周期：整篇进入失败时建立/重建；载入片段/切回原文保留；新文档成功载入/清空时丢弃
const detect = ref<{ originalText: string; candidates: Detected[]; currentIdx: number | null } | null>(null);
const detectShow = ref(false);

// naive 全局主题：darkTheme + 与 CSS 变量同源的 overrides；theme 同时注入给
// TableView/JsonTree 的 discrete message（脱离组件树的弹层需自行带主题）
const naiveTheme = computed(() => (theme.value === "dark" ? darkTheme : null));
const naiveThemeOverrides = computed(() => naiveOverrides(theme.value));
provide("theme", theme);

// 处理中蒙版：busyCount > 0 时树/原文双面板盖蒙版。用计数器而非布尔是防御性的：
// 当前各路径不会嵌套 begin/end，但未来若有嵌套操作（如格式化内嵌刷新改为带蒙版），
// 计数器保证蒙版不被内层提前揭掉。begin/end 必须成对，end 有下限钳制
const busyCount = ref(0);
const panelBusy = computed(() => busyCount.value > 0);
provide("panelBusy", panelBusy);
function busyEnd() {
  busyCount.value = Math.max(0, busyCount.value - 1);
}
provide("panelBusyCtl", {
  begin: () => { busyCount.value++; },
  end: busyEnd,
});

// 组件占位（Task 7-9 实现），方法调用全部走可选链容错
const treePane = ref<HTMLElement>();
const treeRef = ref<{ reload?: (rootId: number, rootKind?: DocInfo["rootKind"]) => void; selectNode?: (nodeId: number) => void; clear?: () => void }>();
const rawRef = ref<{
  load?: (text: string) => void;
  markError?: (line: number | null, col: number | null) => void;
  locate?: (l: Locate) => void;
  clearMarks?: () => void;
}>();
const tableRef = ref<{
  create?: (nodeId: number, path: string) => void;
  onDocReplaced?: () => void;
  onDocCleared?: () => void;
}>();

async function loadTextInner(text: string, opts?: { skipEditor?: boolean; keepDetect?: boolean; autoDetect?: boolean }) {
  // 蒙版触发矩阵：拖拽/打开（无 skipEditor）与粘贴（autoDetect）✅；打字（skipEditor）❌ 防闪烁
  const masked = !opts?.skipEditor || !!opts?.autoDetect;
  if (masked) busyCount.value++;
  const t0 = performance.now();
  try {
    const info = await api.loadText(text);
    parseMs.value = performance.now() - t0;
    if (info.errorMsg) {
      errorMsg.value = `第 ${info.errorLine} 行第 ${info.errorCol} 列：${info.errorMsg}`;
      rawRef.value?.markError?.(info.errorLine, info.errorCol);
      // 探测分流：失败文本变化 → 重建会话；自动弹浮层 = 整篇进入（拖拽/打开/粘贴）；
      // 编辑器打字失败只静默更新入口，不打断输入
      if (info.detections.length) {
        if (!detect.value || detect.value.originalText !== text) {
          detect.value = { originalText: text, candidates: info.detections, currentIdx: null };
        }
        detectShow.value = !opts?.skipEditor || !!opts?.autoDetect;
      }
      return;
    }
    errorMsg.value = null;
    // 成功载入：除非是片段载入/切回原文（keepDetect），否则探测会话随之作废
    if (!opts?.keepDetect) {
      detect.value = null;
      detectShow.value = false;
    }
    docInfo.value = info;
    rootId.value = info.rootId;
    hasDoc.value = true;
    if (!opts?.skipEditor) {
      rawText.value = text;
    }
    // 首次加载/清空后再加载时组件可能尚未挂载，等一帧确保 treeRef/rawRef 就绪
    await nextTick();
    if (!opts?.skipEditor) {
      rawRef.value?.load?.(text); // 编辑路径跳过：编辑器已显示用户输入内容
    }
    treeRef.value?.reload?.(info.rootId, info.rootKind);
    // 文档被整体替换（新加载/格式化/压缩/编辑重解析）：面板可见且有表格 → 按路径重挂并重放过滤；
    // 隐藏/无表时仅标记数据失效（恢复显示时再重验），防止用失效 tableId 调后端命令
    tableRef.value?.onDocReplaced?.();
  } catch (e) {
    // 命令拒绝（后端 Err）：提示但不清空已有文档
    errorMsg.value = String(e);
  } finally {
    if (masked) busyEnd();
  }
}

// 浮层点击候选：替换文档为片段载入文本（保留探测会话，浮层里标记 ✓）
function loadDetect(idx: number) {
  const d = detect.value;
  if (!d) return;
  d.currentIdx = idx;
  detectShow.value = false;
  loadTextInner(d.candidates[idx].loadText, { keepDetect: true });
}

// 大小格式化：≤1024KB 用 KB（1 位小数），超过用 MB（2 位小数）
function fmtSize(bytes: number): string {
  return bytes > 1024 * 1024
    ? `${(bytes / 1024 / 1024).toFixed(2)} MB`
    : `${(bytes / 1024).toFixed(1)} KB`;
}

// 状态栏「↩ 原始文本」：切回完整原文（非法），错误分支会自动重弹浮层（闭环）
function restoreOriginal() {
  const d = detect.value;
  if (!d) return;
  detectShow.value = false;
  d.currentIdx = null; // 没有任何候选处于载入态，浮层 ✓ 标记清除
  // 错误分支不会回写编辑器：切回原文须显式回显 + 镜像 rawText（供格式化/压缩取当前文本）
  rawText.value = d.originalText;
  rawRef.value?.load?.(d.originalText);
  loadTextInner(d.originalText, { keepDetect: true });
}

// ── 原文编辑：立即清除过期标记，防抖 500ms 重解析（skipEditor 不回写编辑器）──
// rawText 在编辑路径同步镜像编辑器内容：格式化/压缩前刷新（flushPendingEdit）需要
// 编辑器的精确当前文本；RawView 没有 props.text watcher，更新它不会重载编辑器/重置光标。
let editTimer: ReturnType<typeof setTimeout> | undefined;

function onRawEdit(text: string, paste = false) {
  rawText.value = text;
  rawRef.value?.clearMarks?.(); // 立即清除残留的行束/错误标记
  clearTimeout(editTimer);
  // 粘贴整段失败也自动弹浮层（autoDetect），与拖拽/打开同一待遇；打字只更新入口
  editTimer = setTimeout(() => loadTextInner(text, { skipEditor: true, autoDetect: paste }), 500);
}

// 格式化/压缩结果回写（RawView doFormat/doMinify 的 emit("loaded")）：
// alreadyLoaded = 后端 format_text/minify_text 已载入新文档并建好 span 树，
// 前端跳过第二次 load_text（省 108ms parse + 12.2MB IPC），直接用返回的根信息刷新
function onRawLoaded(out: FormatOut) {
  if (out.alreadyLoaded) applyPreloaded(out);
  else loadTextInner(out.text, { keepDetect: true });
}

// 后端已载入新文档：跳过 load_text，直接同步前端（探测会话保留 = 原 keepDetect 语义）
async function applyPreloaded(out: FormatOut) {
  errorMsg.value = null;
  docInfo.value = {
    size: out.size, rootId: out.rootId, rootKind: out.rootKind,
    errorLine: null, errorCol: null, errorMsg: null,
    detections: [], // 格式化产物必合法，无探测候选
  };
  rootId.value = out.rootId;
  hasDoc.value = true;
  rawText.value = out.text;
  try {
    // 确保组件 ref 就绪（与 loadTextInner 同款时序）
    await nextTick();
    rawRef.value?.load?.(out.text);
    treeRef.value?.reload?.(out.rootId, out.rootKind);
    tableRef.value?.onDocReplaced?.();
  } finally {
    // 成功/异常路径都收尾蒙版（异常安全：防 nextTick 后任一步抛出导致蒙版永挂）
    busyEnd();
  }
}

// 格式化/压缩前刷新（RawView 在调 formatText/minifyText 前 await 注入的此函数）：
// 取消挂起的编辑防抖，把编辑器当前文本同步给后端并重建树/面板（skipEditor 不回写编辑器）。
// 不变量：点击 格式化/压缩 的任意时刻（哪怕在 500ms 防抖窗口内），后端拿到的都是
// 编辑器的精确当前文本 → 格式化跑在新文本上，结果覆盖编辑器，任何交错下都不丢键。
// 返回 false = 刷新失败（编辑器当前文本非法，后端保留旧文档）：RawView 须中止格式化，
// 否则 formatText 会对旧文档执行并用结果覆盖编辑器，同样丢键。
// keepDetect：编辑当前片段不丢弃探测会话（原始全文/候选保留，可随时切回）
async function flushPendingEdit(): Promise<boolean> {
  clearTimeout(editTimer);
  await loadTextInner(rawText.value, { skipEditor: true, keepDetect: true });
  return !errorMsg.value;
}
provide("beforeFormat", flushPendingEdit);

// 工具栏「清空」：不发后端命令，仅重置前端状态（布局常驻，编辑器/树回到空态）
function onCleared() {
  clearTimeout(editTimer); // 取消挂起的编辑重解析：清空后不应再把刚输入的文本发给后端
  hasDoc.value = false;
  docInfo.value = null;
  rawText.value = "";
  errorMsg.value = null;
  rootId.value = 0;
  parseMs.value = 0;
  rawRef.value?.load?.(""); // 清空编辑器内容（不触发编辑管线）
  treeRef.value?.clear?.(); // 清掉树数据与选中态，回到空态提示
  tableRef.value?.onDocCleared?.(); // 清空：lastPath 随文档失效，清空表格数据并隐藏面板（回细栏引导态）
  detect.value = null; // 清空：探测会话一并丢弃
  detectShow.value = false;
}

async function onDropFile(e: DragEvent) {
  const file = e.dataTransfer?.files?.[0];
  if (!file) return;
  await loadTextInner(await file.text());
}

function locateTo(l: Locate) { rawRef.value?.locate?.(l); }
provide("locateTo", locateTo);

function treeSelect(nodeId: number) { treeRef.value?.selectNode?.(nodeId); }
provide("treeSelect", treeSelect);

function openTable(nodeId: number, path: string) { tableRef.value?.create?.(nodeId, path); }
provide("openTable", openTable);

// 竖直分隔线拖拽（与原型 docs/prototypes/json-tool-ui.html 相同逻辑）
function splitVDown(e: MouseEvent) {
  e.preventDefault();
  const pane = treePane.value;
  const main = pane?.parentElement;
  if (!pane || !main) return;
  const move = (ev: MouseEvent) => {
    const rect = main.getBoundingClientRect();
    const pct = Math.min(80, Math.max(15, ((ev.clientX - rect.left) / rect.width) * 100));
    pane.style.flex = `0 0 ${pct}%`;
  };
  const up = () => {
    document.removeEventListener("mousemove", move);
    document.removeEventListener("mouseup", up);
    document.body.classList.remove("col-resizing");
    document.body.style.userSelect = "";
  };
  document.addEventListener("mousemove", move);
  document.addEventListener("mouseup", up);
  document.body.classList.add("col-resizing");
  document.body.style.userSelect = "none";
}

function resetSplit() {
  if (treePane.value) treePane.value.style.flex = "";
  // 复位后回到 CSS 固定默认宽 28%（内联 flex 清空即生效）
}

// 启动防白屏：窗口以 visible:false 创建，首帧渲染完成后再显示
// （tauri.conf.json windows[0].visible = false；backgroundColor 兜底过渡色）
onMounted(async () => {
  await nextTick();
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().show();
  } catch { /* 浏览器环境（无 Tauri IPC）下忽略 */ }
});
</script>
