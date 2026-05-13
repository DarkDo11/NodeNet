import {
  Area,
  CartesianGrid,
  ComposedChart,
  Line,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { MetricPoint, MetricsRange } from "../types";

export interface ChartSeries {
  key: keyof MetricPoint;
  name: string;
  color: string;
  type?: "line" | "area";
}

interface MetricChartProps {
  title: string;
  data: MetricPoint[];
  range: MetricsRange;
  series: ChartSeries[];
  unitFormatter: (value: number) => string;
  domain?: [number | "auto", number | "auto"];
  emptyLabel?: string;
}

interface TooltipPayload {
  name?: string;
  value?: number | null;
  color?: string;
}

interface ChartTooltipProps {
  active?: boolean;
  label?: number;
  payload?: TooltipPayload[];
  unitFormatter: (value: number) => string;
}

const formatTick = (timestamp: number, range: MetricsRange) => {
  const date = new Date(timestamp);

  if (range === "1d") {
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  if (range === "1w") {
    return date.toLocaleString([], { day: "2-digit", month: "short", hour: "2-digit" });
  }

  if (range === "1m") {
    return date.toLocaleDateString([], { day: "2-digit", month: "short" });
  }

  if (range === "1y") {
    return date.toLocaleDateString([], { month: "short", year: "numeric" });
  }

  return date.toLocaleDateString([], { month: "short", year: "numeric" });
};

const formatTooltipDate = (timestamp: number) =>
  new Date(timestamp).toLocaleString([], {
    year: "numeric",
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });

function ChartTooltip({ active, label, payload, unitFormatter }: ChartTooltipProps) {
  if (!active || typeof label !== "number" || !payload?.length) return null;

  const values = payload.filter((item) => item.value !== null && item.value !== undefined);
  if (values.length === 0) return null;

  return (
    <div className="chart-tooltip">
      <strong>{formatTooltipDate(label)}</strong>
      {values.map((item) => (
        <span key={item.name} style={{ color: item.color }}>
          {item.name}: {unitFormatter(Number(item.value))}
        </span>
      ))}
    </div>
  );
}

const gradientId = (title: string, key: string) =>
  `${title}-${key}`.replace(/[^a-zA-Z0-9_-]/g, "-");

export default function MetricChart({
  title,
  data,
  range,
  series,
  unitFormatter,
  domain = ["auto", "auto"],
  emptyLabel = "No metrics data yet",
}: MetricChartProps) {
  const hasData = data.length > 0;

  return (
    <section className="chart-panel">
      <div className="chart-header">
        <h3>{title}</h3>
        <span>{hasData ? `${data.length} pts` : "0 pts"}</span>
      </div>
      <div className="chart-body">
        {hasData ? (
          <ResponsiveContainer width="100%" height="100%">
            <ComposedChart data={data} margin={{ top: 12, right: 18, left: 0, bottom: 4 }}>
              <defs>
                {series.map((item) => (
                  <linearGradient
                    key={item.key}
                    id={gradientId(title, String(item.key))}
                    x1="0"
                    y1="0"
                    x2="0"
                    y2="1"
                  >
                    <stop offset="5%" stopColor={item.color} stopOpacity={0.28} />
                    <stop offset="95%" stopColor={item.color} stopOpacity={0} />
                  </linearGradient>
                ))}
              </defs>
              <CartesianGrid stroke="#ffffff12" vertical={false} />
              <XAxis
                dataKey="timestamp"
                type="number"
                domain={["dataMin", "dataMax"]}
                tickFormatter={(value) => formatTick(Number(value), range)}
                tick={{ fill: "#8d94a3", fontSize: 11 }}
                axisLine={false}
                tickLine={false}
                minTickGap={28}
              />
              <YAxis
                domain={domain}
                tickFormatter={(value) => unitFormatter(Number(value))}
                width={58}
                tick={{ fill: "#8d94a3", fontSize: 11 }}
                axisLine={false}
                tickLine={false}
              />
              <Tooltip
                content={<ChartTooltip unitFormatter={unitFormatter} />}
                cursor={{ stroke: "#ffffff24" }}
              />
              {series.map((item) =>
                item.type === "area" ? (
                  <Area
                    key={item.key}
                    type="monotone"
                    dataKey={item.key}
                    name={item.name}
                    stroke={item.color}
                    fill={`url(#${gradientId(title, String(item.key))})`}
                    strokeWidth={2}
                    dot={false}
                    connectNulls={false}
                    isAnimationActive={false}
                  />
                ) : (
                  <Line
                    key={item.key}
                    type="monotone"
                    dataKey={item.key}
                    name={item.name}
                    stroke={item.color}
                    strokeWidth={2}
                    dot={false}
                    connectNulls={false}
                    isAnimationActive={false}
                  />
                ),
              )}
            </ComposedChart>
          </ResponsiveContainer>
        ) : (
          <div className="chart-empty">{emptyLabel}</div>
        )}
      </div>
    </section>
  );
}
