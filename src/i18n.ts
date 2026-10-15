// 轻量 i18n：模块级 locale 单例 + t() 取词。
// t() 内部读取 locale.value——在模板/render/computed 中调用时自动建立响应依赖，
// 切换语言即全量刷新。文案 key 以 zh 字典为准（编译期校验 en 无缺漏）。
// 注意：后端（Rust）返回的错误文案不在前端 i18n 范围，保持原文。
import { ref } from "vue";

export type Locale = "zh" | "en";
export const locale = ref<Locale>("zh"); // 默认中文

const zh = {
  open: "打开...",
  clear: "清空",

  treeTag: "🌳 JSON 树",
  expandAll: "展开全部",
  collapseAll: "收缩全部",
  warnPartExpanded: "节点过多，仅展开已加载部分",
  warnPartExpandedHint: "节点过多，仅展开已加载部分（可用搜索/表格查看深层内容）",

  emptyHint: "拖拽json文本文件到窗口，或在右侧原文粘贴",
  statusWaiting: "待加载",
  statusValid: "✓ JSON 合法",
  statusInvalid: "✕ JSON 非法",
  parseMs: "解析 {ms}ms",

  rawTag: "📄 原文",
  format: "格式化",
  formatTitle: "缩进 2 空格 · 只重排空白，token 原样保留",
  minify: "压缩",
  searchPlaceholder: "搜索原文，Enter 下一个",
  clearSearchTitle: "清空搜索",
  errAt: "第 {line} 行第 {col} 列：{msg}",
  formatFail: "格式化失败：{e}",
  minifyFail: "压缩失败：{e}",
  processingHint: "处理中…",

  tableTag: "📊 表格",
  tipRightClickTree: "右键树节点转为表格",
  tipLoadFirst: "加载 JSON 后右键节点转为表格",
  restoringTip: "恢复中…",
  staleTip: "数据已过期，点击恢复",
  hiddenTip: "已隐藏，点击恢复",
  tableTitle: "表格 [{n}]",
  colChip: "列: {col} ✕",
  filterPlaceholder: "多值过滤：alice, 30 或换行分隔",
  filtering: "过滤中…",
  rowsTotal: "共 {n} 行",
  hitInfo: "命中 {hit} / {total} 行 · {n} 个过滤值{scope}",
  loadedRows: "已载前 {m} 行",
  scopeCol: " · 仅「{col}」列",
  colsBtn: "展示列",
  colsTitle: "列显示：勾选要展示的列",
  colsSearch: "搜索列名",
  colsAll: "全选",
  colsInvert: "反选",
  colsOk: "确定",
  exportExcel: "⬆ 导出 Excel",
  hideBtn: "⬇ 隐藏",
  backBtn: "← 返回",
  loadMore: "加载更多（{a} / {b}）",
  dragHeightTitle: "拖拽调整表格高度 · 双击复位",
  clickRestoreTitle: "点击恢复表格面板",
  clearFilterTitle: "清空过滤",
  colChipTitle: "点击取消列过滤",
  backTitle: "返回（转置视图回主表格 / 下钻回上一层）",

  menuCopyKey: "📋 复制Key",
  menuCopyValue: "📋 复制Value",
  menuCopyPath: "📋 复制路径",
  menuCopyRowJson: "📋 复制行JSON",
  menuTranspose: "📐 行转列",
  menuToTable: "📊 转为表格",
  colMenuFilterOnly: "🔎 只过滤「{col}」列",
  colMenuClearFilter: "✕ 取消列过滤",

  msgExported: "已导出 {n} 行",
  msgNoFields: "第 {n} 行没有可展平的字段",
  transposeTitle: "第 {n} 行 · {k} 个字段",
  msgCopiedIntersection: "已复制 {c} 列 × {r} 行交集（TSV）",
  msgCopiedRows: "已复制 {r} 行（TSV）",
  msgCopiedCols: "已复制 {c} 列 × {r} 行（TSV）",

  detectTitle: "⚡ 检测到 {n} 个 JSON 片段",
  detectSubtitle: "文本不是合法 JSON —— 点击片段载入",
  detectLoad: "载入 →",
  detectLoaded: "✓ 已载入",
  detectMeta: "第 {line} 行 · {len} 字符 · 嵌套 {depth} 层",
  kindObj: "对象",
  kindArr: "数组",
  detectEntry: "⚡ 探测结果 ({n})",
  detectOriginal: "↩ 原始文本 ({size})",
};

const en: Record<keyof typeof zh, string> = {
  open: "Open...",
  clear: "Clear",

  treeTag: "🌳 JSON Tree",
  expandAll: "Expand all",
  collapseAll: "Collapse all",
  warnPartExpanded: "Too many nodes — expanded loaded part only",
  warnPartExpandedHint: "Too many nodes — expanded loaded part only (use search/table for deeper content)",

  emptyHint: "Drop a JSON text file here, or paste JSON in the editor on the right",
  statusWaiting: "Waiting",
  statusValid: "✓ JSON valid",
  statusInvalid: "✕ Invalid JSON",
  parseMs: "Parse {ms}ms",

  rawTag: "📄 Raw",
  format: "Format",
  formatTitle: "Indent 2 spaces · whitespace only, tokens kept verbatim",
  minify: "Minify",
  searchPlaceholder: "Search raw text; Enter for next",
  clearSearchTitle: "Clear search",
  errAt: "Line {line}, col {col}: {msg}",
  formatFail: "Format failed: {e}",
  minifyFail: "Minify failed: {e}",
  processingHint: "Processing…",

  tableTag: "📊 Table",
  tipRightClickTree: "Right-click a tree node to create a table",
  tipLoadFirst: "Load JSON, then right-click a node to create a table",
  restoringTip: "Restoring…",
  staleTip: "Data outdated — click to restore",
  hiddenTip: "Hidden — click to restore",
  tableTitle: "Table [{n}]",
  colChip: "Col: {col} ✕",
  filterPlaceholder: "Multi-value filter: alice, 30 or one per line",
  filtering: "Filtering…",
  rowsTotal: "{n} rows",
  hitInfo: "Hit {hit} / {total} rows · {n} filter values{scope}",
  loadedRows: "loaded first {m}",
  scopeCol: " · column \"{col}\" only",
  colsBtn: "Show Cols",
  colsTitle: "Column visibility: check columns to show",
  colsSearch: "Search columns",
  colsAll: "Select all",
  colsInvert: "Invert",
  colsOk: "OK",
  exportExcel: "⬆ Export Excel",
  hideBtn: "⬇ Hide",
  backBtn: "← Back",
  loadMore: "Load more ({a} / {b})",
  dragHeightTitle: "Drag to resize table height · double-click to reset",
  clickRestoreTitle: "Click to restore table panel",
  clearFilterTitle: "Clear filter",
  colChipTitle: "Click to clear column filter",
  backTitle: "Back (transpose → main table / drill-down → upper level)",

  menuCopyKey: "📋 Copy Key",
  menuCopyValue: "📋 Copy Value",
  menuCopyPath: "📋 Copy Path",
  menuCopyRowJson: "📋 Copy Row JSON",
  menuTranspose: "📐 Row → Columns",
  menuToTable: "📊 To Table",
  colMenuFilterOnly: "🔎 Filter column \"{col}\" only",
  colMenuClearFilter: "✕ Clear column filter",

  msgExported: "Exported {n} rows",
  msgNoFields: "Row {n} has no flattenable fields",
  transposeTitle: "Row {n} · {k} fields",
  msgCopiedIntersection: "Copied {c} cols × {r} rows intersection (TSV)",
  msgCopiedRows: "Copied {r} rows (TSV)",
  msgCopiedCols: "Copied {c} cols × {r} rows (TSV)",

  detectTitle: "⚡ Detected {n} JSON fragments",
  detectSubtitle: "Text is not valid JSON — click a fragment to load it",
  detectLoad: "Load →",
  detectLoaded: "✓ Loaded",
  detectMeta: "Line {line} · {len} chars · depth {depth}",
  kindObj: "obj",
  kindArr: "arr",
  detectEntry: "⚡ Detect results ({n})",
  detectOriginal: "↩ Original text ({size})",
};

const dicts: Record<Locale, Record<keyof typeof zh, string>> = { zh, en };

export function t(
  key: keyof typeof zh,
  params?: Record<string, string | number | null | undefined>,
): string {
  let s: string = dicts[locale.value][key];
  if (params) {
    for (const [k, v] of Object.entries(params)) s = s.replaceAll(`{${k}}`, String(v));
  }
  return s;
}
