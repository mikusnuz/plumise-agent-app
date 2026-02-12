interface GaugeRingProps {
  value: number; // 0-100
  label: string;
  detail: string;
  color: string;
  size?: number;
}

export default function GaugeRing({ value, label, detail, color, size = 100 }: GaugeRingProps) {
  const strokeWidth = 8;
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const offset = circumference - (Math.min(value, 100) / 100) * circumference;

  return (
    <div className="flex flex-col items-center gap-2">
      <div className="relative" style={{ width: size, height: size }}>
        <svg width={size} height={size} className="-rotate-90">
          <circle
            className="gauge-ring"
            cx={size / 2}
            cy={size / 2}
            r={radius}
            strokeWidth={strokeWidth}
            stroke="var(--border-divider)"
          />
          <circle
            className="gauge-ring"
            cx={size / 2}
            cy={size / 2}
            r={radius}
            strokeWidth={strokeWidth}
            stroke={color}
            strokeDasharray={circumference}
            strokeDashoffset={offset}
            style={{ transition: 'stroke-dashoffset 0.6s ease' }}
          />
        </svg>
        <div className="absolute inset-0 flex items-center justify-center">
          <span className="text-lg font-bold text-[var(--text-primary)]">
            {Math.round(value)}%
          </span>
        </div>
      </div>
      <div className="text-center">
        <div className="text-xs font-medium text-[var(--text-secondary)]">{label}</div>
        <div className="text-[10px] text-[var(--text-dim)]">{detail}</div>
      </div>
    </div>
  );
}
