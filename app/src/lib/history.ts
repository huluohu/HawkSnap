// 历史网格（M0）前端侧纯逻辑：ShotRow 契约与展示格式化。与 Rust history::ShotRow
// （serde camelCase）逐字段对齐；窗口/UI 状态留在 App.tsx，此文件保持纯函数可单测。
export interface ShotRow {
  id: number;
  createdAt: string;
  path: string;
  thumbPath: string;
  w: number;
  h: number;
  app: string;
  /** 录制时长（秒）。M3 继承项①新增：截图为 null，仅录制行有值。 */
  duration?: number | null;
}

/**
 * RFC3339 created_at → 本地化「日期 时间」串（网格瓦片标题）。解析失败原样返回
 * （历史库内容是数据不是文案，手改坏不炸 UI）。lang 决定 Intl.Locale：zh-CN / en-US。
 */
export function formatShotDate(iso: string, lang: "zh-CN" | "en-US"): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(lang, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}
