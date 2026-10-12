import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import UsageTrend from "../src/features/usage/UsageTrend";
import { smoothPath } from "../src/features/usage/smoothPath";
import type { UsagePoint } from "../src/contracts/pricing";
describe("趋势曲线", () => {
  it("平滑控制点不越过原始区间，平台和峰谷不产生假波动", () => {
    const points = [255, 255, 50, 230, 210].map((y, x) => ({ x: x * 100, y }));
    const path = smoothPath(points);
    const cubics = [
      ...path.matchAll(/C([\d.-]+),([\d.-]+) ([\d.-]+),([\d.-]+) ([\d.-]+),([\d.-]+)/g),
    ];
    expect(cubics).toHaveLength(4);
    cubics.forEach((c, i) => {
      const min = Math.min(points[i].y, points[i + 1].y);
      const max = Math.max(points[i].y, points[i + 1].y);
      for (const y of [Number(c[2]), Number(c[4])]) {
        expect(y).toBeGreaterThanOrEqual(min);
        expect(y).toBeLessThanOrEqual(max);
      }
      expect(Number(c[5])).toBe(points[i + 1].x);
      expect(Number(c[6])).toBe(points[i + 1].y);
    });
    expect(smoothPath([])).toBe("");
    expect(smoothPath([{ x: 0, y: 0 }])).toBe("M0,0");
  });
  it("输入、输出、缓存颜色独立，未知输入保留断点，零仍是数据点", () => {
    const series: UsagePoint[] = ["10", null, "0", "5"].map((n, i) => ({
      at: `2026-10-12T0${i}:00:00Z`,
      currentCost: [],
      tokens: {
        inputFresh: n,
        inputCacheRead: "100",
        inputCacheWrite: "0",
        inputTotal: null,
        outputTotal: "2",
        total: null,
      },
    }));
    const html = renderToStaticMarkup(<UsageTrend series={series} hourly />);
    const groups = [
      ...html.matchAll(
        /<g data-series="([^"]+)">([\s\S]*?)(?=<g data-series=|<text[^>]* y="285")/g,
      ),
    ];
    const group = (name: string) => groups.find((g) => g[1] === name)![2];
    const input = group("inputFresh");
    expect([...input.matchAll(/class="trend-line"/g)]).toHaveLength(2);
    expect([...input.matchAll(/<circle/g)]).toHaveLength(3);
    const colors = ["inputFresh", "outputTotal", "inputCacheRead"].map(
      (name) => group(name).match(/stroke="([^"]+)"/)![1],
    );
    expect(new Set(colors).size).toBe(3);
    expect(group("outputTotal")).toContain(" C");
  });
});
