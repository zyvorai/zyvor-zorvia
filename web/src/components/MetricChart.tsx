// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer } from 'recharts'

interface MetricChartProps {
  title: string
  data: Array<{ time: string; value: number }>
  color?: string
}

export default function MetricChart({ title, data, color = '#3b82f6' }: MetricChartProps) {
  return (
    <div className="bg-[var(--zf-canvas)] rounded-lg p-6 border border-[var(--zf-hairline)]">
      <h3 className="text-lg font-semibold text-[var(--zf-ink)] mb-4">{title}</h3>
      <ResponsiveContainer width="100%" height={200}>
        <LineChart data={data}>
          <CartesianGrid strokeDasharray="3 3" stroke="var(--zf-hairline)" />
          <XAxis dataKey="time" stroke="var(--zf-muted)" />
          <YAxis stroke="var(--zf-muted)" />
          <Tooltip
            contentStyle={{
              backgroundColor: 'var(--zf-surface)',
              border: '1px solid var(--zf-hairline)',
              borderRadius: '0.5rem',
              color: 'var(--zf-ink)',
              fontFamily: 'SF Pro Text, -apple-system, BlinkMacSystemFont, Helvetica Neue, Helvetica, Arial, sans-serif',
            }}
          />
          <Line type="monotone" dataKey="value" stroke={color === '#3b82f6' ? 'var(--zf-link)' : color} strokeWidth={2} dot={false} />
        </LineChart>
      </ResponsiveContainer>
    </div>
  )
}
