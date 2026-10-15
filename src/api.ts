import { invoke } from "@tauri-apps/api/core";

export type NodeKind = NodeDto["kind"];
export interface Detected {
  kind: "obj" | "arr";
  line: number;
  charLen: number;
  depth: number;
  preview: string;
  loadText: string;
}
export interface DocInfo {
  size: number; rootId: number; rootKind: NodeKind;
  errorLine: number | null; errorCol: number | null; errorMsg: string | null;
  detections: Detected[];
}
export interface NodeDto {
  id: number; key: string | null;
  kind: "obj" | "arr" | "str" | "num" | "bool" | "null";
  len: number; literal: string | null; hasChildren: boolean;
}
export interface Locate { startLine: number; endLine: number; startCol: number; endCol: number; }
export interface FormatOut {
  text: string; size: number; rootId: number; rootKind: NodeKind;
  errorLine: number | null; errorCol: number | null; errorMsg: string | null;
  alreadyLoaded: boolean;
}
export interface TableInfo { id: number; path: string; columns: string[]; rowCount: number; }
export interface Cell { col: string; text: string; nested: boolean; }
export interface Row { index: number; nodeId: number; cells: Cell[]; }
export interface PathValue { path: string; value: string; }

export const api = {
  loadText: (text: string) => invoke<DocInfo>("load_text", { text }),
  getChildren: (nodeId: number, offset = 0, limit = 500) =>
    invoke<NodeDto[]>("get_children", { nodeId, offset, limit }),
  locate: (nodeId: number) => invoke<Locate>("locate", { nodeId }),
  locateCell: (rowNodeId: number, col: string) =>
    invoke<Locate>("locate_cell", { rowNodeId, col }),
  nodeAtLine: (line: number) => invoke<number | null>("node_at_line", { line }),
  nodeAncestors: (nodeId: number) => invoke<number[]>("node_ancestors", { nodeId }),
  formatText: (indent: number) =>
    invoke<FormatOut>("format_text", { indent }),
  minifyText: () => invoke<FormatOut>("minify_text"),
  tableCreate: (nodeId: number) => invoke<TableInfo>("table_create", { nodeId }),
  locateCellNode: (rowNodeId: number, col: string) =>
    invoke<number | null>("locate_cell_node", { rowNodeId, col }),
  tableReopen: (path: string, values: string[], col: string | null) =>
    invoke<{ info: TableInfo; hits: number[] }>("table_reopen", { path, values, col }),
  getRows: (tableId: number, offset: number, limit: number) =>
    invoke<Row[]>("get_rows", { tableId, offset, limit }),
  getRowsByIndices: (tableId: number, indices: number[]) =>
    invoke<Row[]>("get_rows_by_indices", { tableId, indices }),
  tableFilter: (tableId: number, values: string[], cols: string[] | null) =>
    invoke<number[]>("table_filter", { tableId, values, cols }),
  tableExport: (tableId: number, path: string, values: string[], cols: string[] | null) =>
    invoke<string>("table_export", { tableId, path, values, cols }),
  rowTranspose: (tableId: number, row: number) =>
    invoke<PathValue[]>("row_transpose", { tableId, row }),
  nodeText: (nodeId: number) => invoke<string>("node_text", { nodeId }),
};

/** 多值切分：逗号 / 空格 / Tab / 换行 */
export const splitTokens = (input: string): string[] =>
  input.split(/[,\s]+/).map(s => s.trim()).filter(Boolean);
