import {
  Area,
  AreaChart,
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { MetricPoint } from "../types";

interface MetricChartProps {
  title: string;
  data: MetricPoint[];
  variant: "traffic" | "cpu";
}

const formatBits = (value: number) => {
  if (value >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(1)} Gb`;
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)} Mb`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)} Kb`;
  return `${value.toFixed(0)} b`;
};

const trafficFormatter = (value: unknown, name: unknown) => [
  `${formatBits(Number(value))}/s`,
  name === "rxRateBps" ? "Down" : "Up",
];

export default function MetricChart({ title, data, variant }: MetricChartProps) {
  const hasData = data.length > 0;

  return (
    <section className="chart-panel">
      <div className="chart-header">
        <h3>{title}</h3>
        <span>{hasData ? `${data.length}/60` : "0/60"}</span>
      </div>
      <div className="chart-body">
        {hasData ? (
          <ResponsiveContainer width="100%" height="100%">
            {variant === "traffic" ? (
              <AreaChart data={data} margin={{ top: 12, right: 16, left: 0, bottom: 0 }}>
                <defs>
                  <linearGradient id="rxGradient" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor="#51d88a" stopOpacity={0.35} />
                    <stop offset="95%" stopColor="#51d88a" stopOpacity={0} />
                  </linearGradient>
                  <linearGradient id="txGradient" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor="#57b9ff" stopOpacity={0.3} />
                    <stop offset="95%" stopColor="#57b9ff" stopOpacity={0} />
                  </linearGradient>
                </defs>
                <CartesianGrid stroke="#ffffff12" vertical={false} />
                <XAxis dataKey="label" hide />
                <YAxis
                  tickFormatter={formatBits}
                  width={52}
                  tick={{ fill: "#8d94a3", fontSize: 11 }}
                  axisLine={false}
                  tickLine={false}
                />
                <Tooltip
                  formatter={trafficFormatter}
                  contentStyle={{
                    background: "#15171d",
                    border: "1px solid #ffffff18",
                    borderRadius: 8,
                    color: "#eef1f6",
                  }}
                  labelStyle={{ color: "#aab1c0" }}
                />
                <Area
                  type="monotone"
                  dataKey="rxRateBps"
                  stroke="#51d88a"
                  fill="url(#rxGradient)"
                  strokeWidth={2}
                  isAnimationActive={false}
                />
                <Area
                  type="monotone"
                  dataKey="txRateBps"
                  stroke="#57b9ff"
                  fill="url(#txGradient)"
                  strokeWidth={2}
                  isAnimationActive={false}
                />
              </AreaChart>
            ) : (
              <LineChart data={data} margin={{ top: 12, right: 16, left: 0, bottom: 0 }}>
                <CartesianGrid stroke="#ffffff12" vertical={false} />
                <XAxis dataKey="label" hide />
                <YAxis
                  domain={[0, 100]}
                  tickFormatter={(value) => `${value}%`}
                  width={42}
                  tick={{ fill: "#8d94a3", fontSize: 11 }}
                  axisLine={false}
                  tickLine={false}
                />
                <Tooltip
                  formatter={(value) => [`${Number(value).toFixed(1)}%`, "CPU"]}
                  contentStyle={{
                    background: "#15171d",
                    border: "1px solid #ffffff18",
                    borderRadius: 8,
                    color: "#eef1f6",
                  }}
                  labelStyle={{ color: "#aab1c0" }}
                />
                <Line
                  type="monotone"
                  dataKey="cpuPercent"
                  stroke="#ffcc66"
                  strokeWidth={2}
                  dot={false}
                  isAnimationActive={false}
                />
              </LineChart>
            )}
          </ResponsiveContainer>
        ) : (
          <div className="chart-empty">Waiting for metrics</div>
        )}
      </div>
    </section>
  );
}
