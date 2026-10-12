/** Shape-preserving cubic Hermite interpolation: no overshoot between data points.
 * Call once per known segment so unknown values never get bridged. */
export function smoothPath(points: { x: number; y: number }[]): string {
  if (!points.length) return "";
  const slope = points.slice(1).map((p, i) => (p.y - points[i].y) / (p.x - points[i].x));
  const tangent = points.map((_, i) => {
    if (!i) return slope[0] ?? 0;
    if (i === points.length - 1) return slope[i - 1];
    const a = slope[i - 1],
      b = slope[i];
    if (a * b <= 0) return 0;
    // Bounded harmonic mean keeps both Bezier control points inside the interval.
    return (2 * a * b) / (a + b);
  });
  let path = `M${points[0].x},${points[0].y}`;
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1],
      b = points[i],
      dx = (b.x - a.x) / 3;
    path += ` C${a.x + dx},${a.y + dx * tangent[i - 1]} ${b.x - dx},${b.y - dx * tangent[i]} ${b.x},${b.y}`;
  }
  return path;
}
