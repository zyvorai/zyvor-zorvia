// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import type { LucideIcon } from 'lucide-react'
import {
  Home,
  Server,
  Star,
  Camera,
  Plus,
  MonitorCog,
  ArrowRightLeft,
  Database,
  ShieldCheck,
  Gauge,
  GitCompare,
  UploadCloud,
  ListChecks,
  Radio,
  HeartPulse,
  Network,
  Save,
  Disc,
  Clock,
  Scale,
  ShieldAlert,
  HardDrive,
  Lock,
  ClipboardList,
  ShieldQuestion,
  DollarSign,
  MapPin,
  BarChart3,
  Lightbulb,
  Layers,
  CalendarClock,
  Cpu,
  Webhook,
  Bell,
  Layers3,
  Boxes,
  DatabaseZap,
} from 'lucide-react'

export interface NavItem {
  label: string
  path: string
  icon: LucideIcon
  /** Short mega-menu blurb (Netra-style). */
  blurb?: string
  /** Hidden unless usePermissions().canAdmin is true. */
  adminOnly?: boolean
}

export interface NavSection {
  label: string
  items: NavItem[]
}

export interface NavGroup {
  name: string
  compact: string
  barIcon: LucideIcon
  items?: NavItem[]
  sections?: NavSection[]
}

/** Top-bar entry: direct link or mega-menu group. */
export type TopNavEntry =
  | { kind: 'link'; item: NavItem }
  | { kind: 'mega'; label: string; items: NavItem[] }

export function flattenNavGroup(g: NavGroup): NavItem[] {
  if (g.items?.length) return g.items
  if (g.sections?.length) return g.sections.flatMap((s) => s.items)
  return []
}

export function navDropdownSections(group: NavGroup): NavSection[] {
  if (group.items?.length) return [{ label: '', items: group.items }]
  return group.sections ?? []
}

function dedupeNavPaths(items: NavItem[]): NavItem[] {
  const seen = new Set<string>()
  return items.filter((item) => {
    if (seen.has(item.path)) return false
    seen.add(item.path)
    return true
  })
}

/** Direct top-bar links (always visible). */
export const TOP_DIRECT_LINKS: NavItem[] = [
  { label: 'Dashboard', path: '/app', icon: Home, blurb: 'Cluster overview and recent activity.' },
  { label: 'Virtual Machines', path: '/app/vms', icon: Server, blurb: 'List, start, stop, and manage VMs.' },
  { label: 'Pods', path: '/app/pods', icon: Boxes, blurb: 'Logs and shell for every pod.', adminOnly: true },
]

/** Mega-menu groups — Netra-style top nav (no sidebar). */
export const TOP_MEGA_GROUPS: { label: string; items: NavItem[] }[] = [
  {
    label: 'Compute',
    items: [
          { label: 'Create VM', path: '/app/create', icon: Plus, blurb: 'Launch a new VM.' },
      { label: 'Favorites', path: '/app/favorites', icon: Star, blurb: 'Pinned VMs.' },
      { label: 'Passthrough devices', path: '/app/devices', icon: Cpu, blurb: 'GPUs, SR-IOV and host devices.', adminOnly: true },
      { label: 'Templates', path: '/app/templates', icon: Layers, blurb: 'Reusable blueprints.' },
      { label: 'Compare', path: '/app/compare', icon: GitCompare, blurb: 'Side-by-side specs.' },
      { label: 'Batch Import', path: '/app/batch-import', icon: UploadCloud, blurb: 'Import many at once.' },
      { label: 'Windows', path: '/app/windows', icon: MonitorCog, blurb: 'Kryton Windows guests.' },
    ],
  },
  {
    label: 'Storage',
    items: [
      { label: 'Volumes', path: '/app/volumes', icon: HardDrive, blurb: 'Persistent volume claims.' },
      { label: 'Storage', path: '/app/storage', icon: Database, blurb: 'Rook-Ceph pools and status.' },
      { label: 'Disk Images', path: '/app/disk-images', icon: Disc, blurb: 'Cloud images and ISOs.' },
      { label: 'Golden Images', path: '/app/golden-images', icon: Layers, blurb: 'Windows builds via Kryton, Linux imports and captures.' },
      { label: 'Snapshots', path: '/app/snapshots', icon: Camera, blurb: 'Point-in-time VM snapshots.' },
      { label: 'Backups', path: '/app/backups', icon: Save, blurb: 'Backup jobs and restores.' },
      { label: 'Backup Scheduler', path: '/app/backup-scheduler', icon: Clock, blurb: 'Recurring backup policies.' },
      { label: 'DataBridge', path: '/app/databridge', icon: DatabaseZap, blurb: 'Cloud-to-edge database migration.' },
    ],
  },
  {
    label: 'Networking',
    items: [
      { label: 'Network Policies', path: '/app/network-policies', icon: Lock, blurb: 'Ingress and egress rules.' },
      { label: 'Service Map', path: '/app/service-map', icon: Network, blurb: 'Observed service topology.' },
      { label: 'Zones', path: '/app/zones', icon: MapPin, blurb: 'Placement and failure domains.' },
    ],
  },
  {
    label: 'Ops',
    items: [
      { label: 'Fleet', path: '/app/fleet', icon: Network, blurb: 'Multi-cluster inventory and node readiness.', adminOnly: true },
      { label: 'Migrations', path: '/app/migrations', icon: ArrowRightLeft, blurb: 'Live and cold migrations.' },
      { label: 'VMware Imports', path: '/app/vmware-imports', icon: UploadCloud, blurb: 'Readiness, network mapping, and import waves.', adminOnly: true },
      {
        label: 'Migration Readiness',
        path: '/app/migrations/readiness',
        icon: ListChecks,
        blurb: 'Pre-flight migration checks.',
      },
      { label: 'Health Check', path: '/app/health-check', icon: HeartPulse, blurb: 'Guest and agent health.' },
      { label: 'HA Policy', path: '/app/ha-policy', icon: ShieldAlert, blurb: 'High-availability policies.' },
      { label: 'Placement Advisor', path: '/app/placement', icon: Scale, blurb: 'Where to schedule next.' },
      { label: 'Warm Pools', path: '/app/warm-pools', icon: Layers3, blurb: 'Pre-warmed capacity pools.' },
    ],
  },
  {
    label: 'Automation',
    items: [
      { label: 'Event Stream', path: '/app/events', icon: Radio, blurb: 'Live cluster event feed.' },
      { label: 'Schedules', path: '/app/schedules', icon: CalendarClock, blurb: 'Power and maintenance windows.' },
      { label: 'Alerts', path: '/app/alerts', icon: Bell, blurb: 'Alert rules and history.' },
      { label: 'Webhooks', path: '/app/webhooks', icon: Webhook, blurb: 'Outbound event delivery.' },
    ],
  },
  {
    label: 'Secure',
    items: [
      {
        label: 'Access Control',
        path: '/app/access-control',
        icon: ShieldCheck,
        blurb: 'Users, roles, and MFA.',
        adminOnly: true,
      },
      { label: 'Compliance', path: '/app/compliance', icon: ClipboardList, blurb: 'Compliance posture board.' },
      { label: 'Security', path: '/app/security', icon: ShieldQuestion, blurb: 'Security findings and risk.' },
    ],
  },
  {
    label: 'Insights',
    items: [
      { label: 'Capacity', path: '/app/capacity', icon: Cpu, blurb: 'Headroom and forecasts.' },
      { label: 'Analytics', path: '/app/analytics', icon: BarChart3, blurb: 'Usage trends over time.' },
      { label: 'Optimizer', path: '/app/optimizer', icon: Lightbulb, blurb: 'Rightsizing suggestions.' },
      { label: 'Cost Estimator', path: '/app/cost-estimator', icon: DollarSign, blurb: 'Rough cost models.' },
      { label: 'Quotas', path: '/app/quotas', icon: Gauge, blurb: 'Namespace resource limits.' },
    ],
  },
]

export const TOP_NAV: TopNavEntry[] = [
  ...TOP_DIRECT_LINKS.map((item) => ({ kind: 'link' as const, item })),
  ...TOP_MEGA_GROUPS.map((g) => ({ kind: 'mega' as const, label: g.label, items: g.items })),
]

/** @deprecated Prefer TOP_NAV — kept for command palette / breadcrumb helpers. */
export const TOP_BAR_QUICK_LINKS: NavItem[] = []

/** Sidebar-era groups — still used by command palette flattening. */
export const NAV_GROUPS: NavGroup[] = [
  {
    name: 'Core',
    compact: 'Core',
    barIcon: Home,
    sections: [
      { label: 'Compute', items: [...TOP_DIRECT_LINKS, ...TOP_MEGA_GROUPS[0].items] },
      { label: 'Storage', items: TOP_MEGA_GROUPS[1].items },
      { label: 'Networking', items: TOP_MEGA_GROUPS[2].items },
      { label: 'Ops & Resilience', items: TOP_MEGA_GROUPS[3].items },
      { label: 'Automation & Alerts', items: TOP_MEGA_GROUPS[4].items },
      { label: 'Security & Compliance', items: TOP_MEGA_GROUPS[5].items },
      { label: 'Insights & Admin', items: TOP_MEGA_GROUPS[6].items },
    ],
  },
]

export const ALL_NAV_ITEMS: NavItem[] = dedupeNavPaths([
  ...TOP_DIRECT_LINKS,
  ...TOP_MEGA_GROUPS.flatMap((g) => g.items),
])

export const PAGE_TITLE_BY_PATH: Record<string, string> = Object.fromEntries(
  ALL_NAV_ITEMS.map((item) => [item.path, item.label]),
)

export const routeLabels: Record<string, string> = {
  ...PAGE_TITLE_BY_PATH,
  '/app/create': 'Create VM',
}
