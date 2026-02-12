import { NavLink } from 'react-router-dom';
import { LayoutDashboard, ScrollText, Settings, type LucideIcon } from 'lucide-react';
import type { AgentStatus } from '../../types';

interface NavItem {
  to: string;
  label: string;
  icon: LucideIcon;
}

const NAV_ITEMS: NavItem[] = [
  { to: '/', label: 'Dashboard', icon: LayoutDashboard },
  { to: '/logs', label: 'Logs', icon: ScrollText },
  { to: '/settings', label: 'Settings', icon: Settings },
];

const STATUS_CONFIG: Record<AgentStatus, { label: string; color: string; badgeClass: string }> = {
  stopped: { label: 'Stopped', color: '#64748b', badgeClass: '' },
  starting: { label: 'Starting...', color: '#fb923c', badgeClass: 'badge-warning' },
  running: { label: 'Running', color: '#4ade80', badgeClass: 'badge-success' },
  stopping: { label: 'Stopping...', color: '#fb923c', badgeClass: 'badge-warning' },
  error: { label: 'Error', color: '#ef4444', badgeClass: 'badge-danger' },
};

interface SidebarProps {
  status: AgentStatus;
}

export default function Sidebar({ status }: SidebarProps) {
  const statusConfig = STATUS_CONFIG[status];

  return (
    <aside className="w-52 h-full bg-[var(--bg-sidebar)] border-r border-[var(--border-divider)] flex flex-col shrink-0">
      {/* Status indicator */}
      <div className="px-4 py-5">
        <div className="flex items-center gap-2 mb-1">
          <div
            className="w-2 h-2 rounded-full"
            style={{
              backgroundColor: statusConfig.color,
              animation: status === 'running' ? 'pulse-dot 2s ease-in-out infinite' : undefined,
            }}
          />
          <span className="text-xs font-medium text-[var(--text-secondary)]">
            {statusConfig.label}
          </span>
        </div>
      </div>

      {/* Navigation */}
      <nav className="flex-1 px-3 space-y-1">
        {NAV_ITEMS.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.to === '/'}
            className={({ isActive }) =>
              `sidebar-item ${isActive ? 'sidebar-item-active' : ''}`
            }
          >
            <item.icon size={18} />
            <span>{item.label}</span>
          </NavLink>
        ))}
      </nav>

      {/* Version */}
      <div className="px-4 py-4 border-t border-[var(--border-divider)]">
        <p className="text-[10px] text-[var(--text-dim)]">Plumise Agent v0.1.0</p>
      </div>
    </aside>
  );
}
