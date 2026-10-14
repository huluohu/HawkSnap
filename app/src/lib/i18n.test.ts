import { afterEach, describe, expect, it, vi } from "vitest";
import {
  t,
  setLanguage,
  getLanguage,
  __setDictionariesForTests,
} from "./i18n";
import zhDict from "../locales/zh-CN.json";
import enDict from "../locales/en-US.json";

// 每个 case 自恢复词典内建态；语言态由各 case 显式 setLanguage 前置，互不依赖。
afterEach(() => {
  __setDictionariesForTests();
  vi.restoreAllMocks();
});

// 覆盖 window.navigator.language（jsdom：getter 在原型上，实例自有属性即遮蔽）。
function stubNavigatorLanguage(lang: string) {
  Object.defineProperty(window.navigator, "language", {
    value: lang,
    configurable: true,
  });
}

describe("i18n", () => {
  it("t_returns_zh_for_known_key", () => {
    setLanguage("zh-CN");
    expect(t("settings.title")).toBe("设置");
    expect(t("overlay.copy")).toBe("复制");
  });

  it("t_returns_en_for_known_key_after_switch", () => {
    setLanguage("en-US");
    expect(t("settings.title")).toBe("Settings");
  });

  it("t_falls_back_to_zh_then_key", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    setLanguage("en-US");
    // en 词典缺 key（zh 有）：回退 zh-CN。
    __setDictionariesForTests({ "only.in.zh": "只有中文" }, { "some.other": "Other" });
    expect(t("only.in.zh")).toBe("只有中文");
    // 两份词典都缺：回退 key 本身 + console.warn（dev 告警）。
    expect(t("totally.missing")).toBe("totally.missing");
    expect(console.warn).toHaveBeenCalled();
  });

  it("t_interpolates_vars", () => {
    setLanguage("zh-CN");
    expect(
      t("banner.hotkeyError", { kind: "截图", message: "被占用" }),
    ).toBe("截图 热键注册失败：被占用");
    expect(t("settings.quality", { n: 42 })).toBe("质量 42");
    // 缺参：占位符原样保留（不吞不炸）。
    expect(t("settings.quality")).toBe("质量 {n}");
  });

  it("language_resolution_follows_system_navigator", () => {
    stubNavigatorLanguage("zh-CN");
    setLanguage("system");
    expect(getLanguage()).toBe("zh-CN");
    stubNavigatorLanguage("en-GB");
    setLanguage("system");
    expect(getLanguage()).toBe("en-US");
  });

  it("language_resolution_explicit_override_wins", () => {
    stubNavigatorLanguage("en-US");
    setLanguage("zh-CN");
    expect(getLanguage()).toBe("zh-CN");
    setLanguage("system");
    expect(getLanguage()).toBe("en-US");
  });

  // G6 补网：动态 key（模板字面量 t(`hotkey.kind.${kind}`)）检查器扫不到，
  // 用单测断言三个 kind key 双语同时在盘。
  it("dynamic_hotkey_kind_keys_exist_in_both_dictionaries", () => {
    for (const kind of ["capture", "pin", "record"] as const) {
      expect(zhDict[`hotkey.kind.${kind}`]).toBeTruthy();
      expect(enDict[`hotkey.kind.${kind}`]).toBeTruthy();
    }
  });
});
