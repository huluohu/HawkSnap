import { describe, expect, it } from "vitest";
import { toLocalHover } from "./hover";

const display = { x: 0, y: 0, width: 1512, height: 982 };
describe("toLocalHover", () => {
  it("converts global frame to local rect", () => {
    expect(toLocalHover({ x: 10, y: 20, w: 100, h: 50, role: "AXButton" }, display))
      .toEqual({ x: 10, y: 20, w: 100, h: 50 });
  });
  it("drops frames outside this display", () => {
    expect(toLocalHover({ x: 2000, y: 20, w: 100, h: 50, role: "AXButton" }, display)).toBeNull();
  });
  it("returns null for null probe", () => {
    expect(toLocalHover(null, display)).toBeNull();
  });
});
