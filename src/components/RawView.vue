<template>
  <div class="pane-head">
    <span class="tag">{{ t("rawTag") }}</span>
    <span class="spacer"></span>
    <button class="btn" :disabled="docEmpty || panelBusy" :title="t('formatTitle')" @click="doFormat">{{ t("format") }}</button>
    <button class="btn" :disabled="docEmpty || panelBusy" title="压缩为单行" @click="doMinify">{{ t("minify") }}</button>
    <span class="raw-search">
      <span class="raw-search-box">
        <input
          ref="searchEl"
          v-model="searchText"
          class="raw-search-input mono"
          :class="{ 'has-clear': searchText }"
          type="text"
          :placeholder="t('searchPlaceholder')"
          spellcheck="false"
          @input="onSearchInput"
          @keydown.enter.prevent="(e: KeyboardEvent) => onSearchKey(!e.shiftKey)"
          @keydown.esc.prevent="clearSearch(true)"
        />
        <button v-if="searchText" class="search-clear" :title="t('clearSearchTitle')"
                tabindex="-1" @mousedown.prevent @click="clearSearch()">✕</button>
      </span>
      <span class="search-count mono">{{ searchInfo }}</span>
    </span>
  </div>
  <div class="pane-body" ref="host"></div>
</template>

<script setup lang="ts">
import { computed, inject, onBeforeUnmount, onMounted, ref, watch, type Ref } from "vue";
import { createDiscreteApi, darkTheme } from "naive-ui";
import { EditorView, lineNumbers, Decoration, keymap, type DecorationSet } from "@codemirror/view";
import { EditorState, StateEffect, StateField, Annotation, Transaction, type Range } from "@codemirror/state";
import { indentWithTab } from "@codemirror/commands";
import { json } from "@codemirror/lang-json";
import { oneDark } from "@codemirror/theme-one-dark";
import { api, type FormatOut, type Locate } from "../api";
import { t } from "../i18n";

const props = defineProps<{ text: string; theme: "light" | "dark"; onEdit?: (text: string, paste?: boolean) => void }>();
const emit = defineEmits<{ (e: "loaded", out: FormatOut): void }>();
const treeSelect = inject<(nodeId: number) => void>("treeSelect")!;
// 格式化/压缩前刷新：App 注入的异步函数（取消挂起的编辑防抖并把编辑器当前文本同步给后端）。
// 必须在 formatText/minifyText 之前 await——这两个命令读的是后端文档，
// 不刷新的话 500ms 编辑防抖窗口内的最后几次键入会丢（格式化跑在旧文本上）。
const beforeFormat = inject<() => Promise<boolean>>("beforeFormat", async () => true);

// 处理中蒙版：格式化/压缩全程盖双面板蒙版；成功路径由 App.applyPreloaded 收尾
const panelBusy = inject<Ref<boolean>>("panelBusy", ref(false));
const busyCtl = inject<{ begin: () => void; end: () => void }>("panelBusyCtl", { begin: () => {}, end: () => {} });

// 独立 message 实例（App 内未包 NMessageProvider，用 discrete api 不依赖祖先）
// 仅格式化/压缩失败时提示（此时后端保留旧文档，App 错误条不会触发）
const { message } = createDiscreteApi(["message"], {
  configProviderProps: computed(() => ({
    theme: props.theme === "dark" ? darkTheme : undefined,
  })),
});

// 格式化：固定缩进 2 空格，span 保真（只重排空白，token 原样保留，见 §4.11）
// 注意：不能用 props.text 做禁用判定——编辑路径（skipEditor）不回写 App.rawText，
// 粘贴/输入后 props.text 仍为空，按钮会永久禁用。改用编辑器实际文档状态。
const docEmpty = ref(true);

async function doFormat() {
  busyCtl.begin();
  try {
    if (!(await beforeFormat())) { busyCtl.end(); return; } // 刷新失败（当前文本非法）：中止，防旧文档覆盖编辑器丢键
    const out = await api.formatText(2);
    if (out.errorMsg) {
      busyCtl.end();
      message.error(t("errAt", { line: out.errorLine, col: out.errorCol, msg: out.errorMsg }));
      return;
    }
    emit("loaded", out); // 成功：蒙版由 App.applyPreloaded 末尾揭掉
  } catch (e) {
    busyCtl.end();
    message.error(t("formatFail", { e: String(e) }));
  }
}

async function doMinify() {
  busyCtl.begin();
  try {
    if (!(await beforeFormat())) { busyCtl.end(); return; }
    const out = await api.minifyText();
    if (out.errorMsg) {
      busyCtl.end();
      message.error(t("errAt", { line: out.errorLine, col: out.errorCol, msg: out.errorMsg }));
      return;
    }
    emit("loaded", out);
  } catch (e) {
    busyCtl.end();
    message.error(t("minifyFail", { e: String(e) }));
  }
}

const host = ref<HTMLDivElement>();
let view: EditorView | null = null;

const setBeam = StateEffect.define<number | null>();   // 行号；null = 清除
const setError = StateEffect.define<number | null>();  // 文档偏移；null = 清除

// 事务注解：true = 程序化加载（load），不视为用户编辑
const ExternalText = Annotation.define<boolean>();

// ── 原文搜索（手写扫描 + StateField 装饰，避免新增 @codemirror/search 依赖）──
const setHits = StateEffect.define<{ from: number; to: number }[] | null>(); // 全部命中；null = 清除
const setCur = StateEffect.define<number>();                                 // 当前命中下标

const beamMark = Decoration.line({ class: "cm-beam" });
const errMark = Decoration.mark({ class: "cm-err" });
const hitMark = Decoration.mark({ class: "cm-hit" });
const curHitMark = Decoration.mark({ class: "cm-hit cm-cur" });

interface SearchState { hits: { from: number; to: number }[]; cur: number }

const searchField = StateField.define<{ deco: DecorationSet; s: SearchState }>({
  create: () => ({ deco: Decoration.none, s: { hits: [], cur: 0 } }),
  update(v, tr) {
    if (tr.docChanged) return { deco: Decoration.none, s: { hits: [], cur: 0 } }; // doc 变化 = 清空
    let { hits, cur } = v.s;
    let touched = false;
    for (const e of tr.effects) {
      if (e.is(setHits)) { hits = e.value ?? []; cur = 0; touched = true; }
      else if (e.is(setCur)) { cur = e.value; touched = true; }
    }
    if (!touched) return v;
    const deco = hits.length
      ? Decoration.set(hits.map((h, i) => (i === cur ? curHitMark : hitMark).range(h.from, h.to)), true)
      : Decoration.none;
    return { deco, s: { hits, cur } };
  },
});
const searchDecorations = EditorView.decorations.compute([searchField], s => s.field(searchField).deco);

const beamField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    for (const e of tr.effects) if (e.is(setBeam)) {
      if (e.value === null) return Decoration.none;
      const l = tr.state.doc.line(e.value);
      return Decoration.set([beamMark.range(l.from, l.from)]);
    }
    return deco;
  },
});
const errField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    for (const e of tr.effects) if (e.is(setError)) {
      if (e.value === null) return Decoration.none;
      // 保证标记至少 1 字符：文档末尾出错时 pos 可能等于 doc.length（空标记会抛 RangeError）
      const len = tr.state.doc.length;
      const from = Math.min(Math.max(0, e.value), Math.max(0, len - 1));
      return Decoration.set([errMark.range(from, from + 1)]);
    }
    return deco;
  },
});
const allDecorations = EditorView.decorations.compute([beamField, errField], s => {
  const parts: Range<Decoration>[] = [];
  s.field(beamField).between(0, 1e12, (f, t, v) => { parts.push(v.range(f, t)); });
  s.field(errField).between(0, 1e12, (f, t, v) => { parts.push(v.range(f, t)); });
  return parts.length ? Decoration.set(parts, true) : Decoration.none;
});

// ── 搜索框状态（Vue 侧持有命中列表，CM 侧经 effect 同步装饰）──
const searchEl = ref<HTMLInputElement>();
const searchText = ref("");
const searchInfo = ref("");                 // "n/m"；空 = 未搜索
const matches = ref<{ from: number; to: number }[]>([]);
const curIdx = ref(0);
let searchTimer: ReturnType<typeof setTimeout> | undefined;
let searchedFor = "";                       // 当前命中列表对应的关键词（Enter 判断是否需重搜）

function findMatches(): { from: number; to: number }[] {
  if (!view) return [];
  const q = searchText.value.toLowerCase();
  if (!q) return [];
  const text = view.state.doc.toString().toLowerCase();
  const out: { from: number; to: number }[] = [];
  let idx = text.indexOf(q);
  while (idx !== -1) {
    out.push({ from: idx, to: idx + q.length });
    idx = text.indexOf(q, idx + q.length); // 允许自交叠（如 "aa" 命中 "aaa" 两次）
  }
  return out;
}

function applySearch(scroll: boolean) {
  if (!view) return;
  const hits = findMatches();
  searchedFor = searchText.value;
  matches.value = hits;
  curIdx.value = 0;
  view.dispatch({ effects: setHits.of(hits.length ? hits : []) });
  searchInfo.value = hits.length ? `1/${hits.length}` : "0/0";
  if (scroll && hits.length) scrollToMatch(0);
}

function scrollToMatch(i: number) {
  if (!view) return;
  const h = matches.value[i];
  if (h) view.dispatch({ effects: EditorView.scrollIntoView(h.from, { y: "center" }) });
}

function onSearchInput() {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    if (!searchText.value) clearSearch();
    else applySearch(true);
  }, 200);
}

function onSearchKey(next: boolean) {
  if (!searchText.value) return;
  // 输入后未等防抖就按 Enter：先用当前文本搜索（与防抖结果可能不同步）
  if (searchText.value !== searchedFor) {
    clearTimeout(searchTimer);
    applySearch(true);
    return;
  }
  const n = matches.value.length;
  if (!n || !view) return;
  curIdx.value = ((curIdx.value + (next ? 1 : -1)) % n + n) % n;
  view.dispatch({ effects: setCur.of(curIdx.value) });
  searchInfo.value = `${curIdx.value + 1}/${n}`;
  scrollToMatch(curIdx.value);
}

function clearSearch(focusBack = false) {
  clearTimeout(searchTimer);
  searchText.value = "";
  searchedFor = "";
  searchInfo.value = "";
  matches.value = [];
  curIdx.value = 0;
  view?.dispatch({ effects: setHits.of(null) });
  if (focusBack) view?.focus(); // 用户按 Esc 清空搜索后焦点还给编辑器；程序化加载不抢焦点
}

function build(dark: boolean, docLen: number) {
  return [
    lineNumbers(),
    // 巨型文档（如 9.5MB 单行）跳过 lezer JSON 语法解析：实测占加载 ~187ms，
    // 单行场景高亮几乎不可见；编辑/格式化能力不受影响
    ...(docLen <= 2_000_000 ? [json()] : []),
    keymap.of([indentWithTab]),
    beamField, errField, allDecorations, searchField, searchDecorations,
    ...(dark ? [oneDark] : []),
    // 用户编辑回调：doc 变化且未标记为程序化加载时触发；
    // 粘贴事务（userEvent=input.paste）单独标记——探测功能据此区分「粘贴整段」与打字
    EditorView.updateListener.of(u => {
      if (u.docChanged) docEmpty.value = u.state.doc.length === 0;
      if (u.docChanged && !u.transactions.some(tr => tr.annotation(ExternalText) === true)) {
        const paste = u.transactions.some(tr => tr.annotation(Transaction.userEvent) === "input.paste");
        props.onEdit?.(u.state.doc.toString(), paste);
      }
    }),
    EditorView.domEventHandlers({
      mousedown: (e, v) => {
        const pos = v.posAtCoords(e);
        if (pos == null) return false;
        const lineNo = v.state.doc.lineAt(pos).number;
        api.nodeAtLine(lineNo).then(id => { if (id !== null) treeSelect(id); });
        return false;
      },
      // 文件拖放：阻止 CM/浏览器把文件内容当文本插入编辑器（否则编辑管线会把
      // 「旧文档 + 文件内容」的拼接文本发给后端重解析报错），preventDefault 后
      // 放行冒泡给根 .app 的 onDropFile 统一加载
      drop: (e) => {
        if (e.dataTransfer && [...e.dataTransfer.types].includes("Files")) {
          e.preventDefault();
          return true;
        }
        return false; // 编辑器内文本拖放仍由 CM 处理
      },
    }),
  ];
}

function createView(doc: string, dark: boolean) {
  view?.destroy();
  view = new EditorView({
    parent: host.value!,
    state: EditorState.create({ doc, extensions: build(dark, doc.length) }),
  });
}

onMounted(() => {
  createView(props.text, props.theme === "dark");
  docEmpty.value = view!.state.doc.length === 0;
  // Ctrl+F 全局聚焦原文搜索框（搜索框内再按不递归：input 元素上 keydown 先被其 esc/enter 处理，
  // 这里 preventDefault 阻止浏览器默认查找对话框即可）
  window.addEventListener("keydown", onGlobalKey);
});

// 主题切换：销毁重建（同一文档文本，装饰随新 State 清空）；恢复当前搜索高亮
watch(() => props.theme, t => {
  if (!view) return;
  createView(view.state.doc.toString(), t === "dark");
  if (searchText.value) applySearch(false);
});

onBeforeUnmount(() => {
  view?.destroy();
  view = null;
  window.removeEventListener("keydown", onGlobalKey);
});

// Ctrl+F → 聚焦搜索框；Esc 清空搜索后焦点交还编辑器
function onGlobalKey(e: KeyboardEvent) {
  if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === "f") {
    e.preventDefault();
    searchEl.value?.focus();
  }
}

defineExpose({
  // 程序化加载：带 ExternalText 注解，不触发 onEdit；同时清除行束/错误标记
  load(text: string) {
    docEmpty.value = text.length === 0;
    view?.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text },
                     annotations: ExternalText.of(true),
                     effects: [setBeam.of(null), setError.of(null)] });
    clearSearch(); // 新文档：清空搜索框与全部高亮
  },
  // 清除行束与错误标记（编辑开始时调用，防止残留过期标记）
  clearMarks() {
    view?.dispatch({ effects: [setBeam.of(null), setError.of(null)] });
  },
  locate(l: Locate) {
    if (!view) return;
    if (l.startLine < 1 || l.startLine > view.state.doc.lines) return;
    view.dispatch({ effects: setBeam.of(l.startLine) });
    const line = view.state.doc.line(l.startLine);
    view.dispatch({ effects: EditorView.scrollIntoView(line.from, { y: "center" }) });
  },
  markError(line: number | null, col: number | null) {
    if (!view || line == null || col == null) return;
    if (line < 1 || line > view.state.doc.lines) return;
    const l = view.state.doc.line(line);
    view.dispatch({ effects: setError.of(Math.min(l.to, l.from + Math.max(0, col - 1))) });
  },
});
</script>

<style scoped>
/* ✕ 清空按钮：绝对定位在输入框右缘，不挤占输入区；仅在有输入时渲染 */
.raw-search-box { position: relative; display: inline-flex; }
.raw-search-input.has-clear { padding-right: 22px; }
.search-clear {
  position: absolute; right: 4px; top: 50%; transform: translateY(-50%);
  width: 16px; height: 16px; padding: 0; border: none; border-radius: 3px;
  background: none; color: var(--fg-dim); font-size: 11px; line-height: 1;
  display: flex; align-items: center; justify-content: center;
  cursor: pointer; user-select: none;
}
.search-clear:hover { color: var(--accent); }
</style>
