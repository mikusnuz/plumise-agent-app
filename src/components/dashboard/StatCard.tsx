import type { LucideIcon } from 'lucide-react';

interface StatCardProps {
  icon: LucideIcon;
  label: string;
  value: string | number;
  sub?: string;
  color?: string;
}

export default function StatCard({ icon: Icon, label, value, sub, color = '#06b6d4' }: StatCardProps) {
  return (
    <div className="stat-card">
      <div className="flex items-start justify-between mb-3">
        <div
          className="flex items-center justify-center w-9 h-9 rounded-lg"
          style={{ background: `${color}15` }}
        >
          <Icon size={18} style={{ color }} />
        </div>
      </div>
      <div className="text-2xl font-bold tracking-tight text-[var(--text-primary)]">
        {value}
      </div>
      <div className="text-xs text-[var(--text-muted)] mt-0.5">{label}</div>
      {sub && (
        <div className="text-[10px] text-[var(--text-dim)] mt-1">{sub}</div>
      )}
    </div>
  );
}
