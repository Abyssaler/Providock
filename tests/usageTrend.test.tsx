import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import UsageTrend, {
  formatAxisTokens,
  formatTooltipTime,
  tooltipRows,
} from "../src/features/usage/UsageTrend";
import { smoothPath } from "../src/features/usage/smoothPath";
import type { UsagePoint } from "../src/contracts/pricing";
describe("趋势曲线", () => {
  it("多币种提供独立选择，提示按所选币种读取，缺币种不补零", () => {
    const point: UsagePoint = {
      at: "2026-10-12T08:00:00Z",
      currentCost: [
        { currency: "CNY", amount: "7" },
        { currency: "USD", amount: "1" },
      ],
      tokens: {
        inputFresh: null,
        inputCacheRead: null,
        inputCacheWrite: null,
        inputTotal: null,
        outputTotal: null,
        total: null,
      },
    };
    const html = renderToStaticMarkup(<UsageTrend series={[point]} hourly />);
    expect(html).toContain('aria-label="趋势成本币种"');
    expect(html).toContain('role="combobox"');
    expect(html).toContain('title="CNY"');
    expect(tooltipRows(point, "CNY")[0].text).toBe("¥7.00");
    expect(tooltipRows(point, "USD")[0].text).toBe("$1.00");
    expect(
      tooltipRows({ ...point, currentCost: [{ currency: "CNY", amount: "7" }] }, "USD"),
    ).toEqual([]);
  });
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
        /<g data-series="([^"]+)">([\s\S]*?)(?=<g data-series=|<text[^>]* y="330")/g,
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
    // 未知与无成本在数据表中显示「未提供」，绝不补零
    expect(html).toContain("未提供");
    // 每个 Token 序列都有渐变填充定义（0.18 → 0），成本线没有填充
    expect([...html.matchAll(/<linearGradient id="fill-/g)]).toHaveLength(4);
    expect(html).toContain('stop-opacity="0.18"');
    expect(html).not.toContain('id="fill-cost"');
  });
  it("成本线为虚线并使用独立的右轴刻度", () => {
    const series: UsagePoint[] = [1, 2, 3].map((n, i) => ({
      at: `2026-10-1${i}T08:00:00Z`,
      currentCost: [{ currency: "CNY", amount: String(n * 0.5) }],
      tokens: {
        inputFresh: "1000",
        inputCacheRead: null,
        inputCacheWrite: null,
        inputTotal: null,
        outputTotal: null,
        total: null,
      },
    }));
    const html = renderToStaticMarkup(<UsageTrend series={series} hourly={false} />);
    expect(html).toContain('data-series="cost"');
    expect(html).toContain('stroke="#ff7088"');
    expect(html).toContain('stroke-dasharray="4 4"');
    const cost = html.match(
      /<g data-series="cost">([\s\S]*?)(?=<g data-series=|<text[^>]* y="330")/,
    )![1];
    expect(cost).toContain('fill="none"');
    expect(cost).not.toContain("url(#fill-");
    // 右轴 ¥ 刻度与左轴 Token 刻度（千分位/紧凑）互相独立
    expect(html).toContain("¥1.50");
    expect(html).toContain("¥0.00");
    expect(html).toContain(">1000</text>");
    expect(html).toContain("估算成本（CNY）");
  });
  it("轴刻度与提示格式：紧凑中文刻度、含年份标题、千分位 Token、两位小数成本", () => {
    expect(formatAxisTokens(0)).toBe("0");
    expect(formatAxisTokens(15000)).toBe("1.5万");
    expect(formatAxisTokens(120000000)).toBe("1.2亿");
    expect(formatTooltipTime("2026-10-12T08:30:00Z")).toContain("2026");
    const point: UsagePoint = {
      at: "2026-10-12T08:30:00Z",
      currentCost: [{ currency: "CNY", amount: "0.123456789" }],
      tokens: {
        inputFresh: "12345",
        inputCacheRead: null,
        inputCacheWrite: null,
        inputTotal: null,
        outputTotal: "678",
        total: null,
      },
    };
    const rows = tooltipRows(point, "CNY");
    expect(rows.map((r) => r.key)).toEqual(["inputFresh", "outputTotal", "cost"]);
    expect(rows[0].text).toContain("12,345");
    expect(rows[2].text).toBe("¥0.12");
    // 断档 bucket：全部未知 → 没有任何行，不伪造 0
    const gap = tooltipRows(
      {
        at: point.at,
        currentCost: [],
        tokens: {
          inputFresh: null,
          inputCacheRead: null,
          inputCacheWrite: null,
          inputTotal: null,
          outputTotal: null,
          total: null,
        },
      },
      "CNY",
    );
    expect(gap).toEqual([]);
  });
});
