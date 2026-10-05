// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { BrowserRouter, Routes, Route, Navigate, useLocation } from 'react-router'
import { Suspense, lazy, ReactNode } from 'react'
import { ThemeProvider } from './contexts/ThemeContext'
import { ToastProvider } from './contexts/ToastContext'
import { WebSocketProvider } from './contexts/WebSocketContext'
import { AuthProvider, useAuth } from './contexts/AuthContext'
import { usePermissions } from './hooks/usePermissions'
import { PlatformInfoProvider } from './contexts/PlatformInfoContext'
import PageSkeleton from './components/PageSkeleton'
import { PageErrorBoundary } from './components/ErrorBoundary'
import ConsoleLayout from './components/ConsoleLayout'
import SignIn from './pages/SignIn'
import NotFound from './pages/NotFound'
import Home from './pages/marketing/Home'
import Product from './pages/marketing/Product'
import Platform from './pages/marketing/Platform'
import SecurityPage from './pages/marketing/Security'

const Dashboard = lazy(() => import('./pages/Dashboard'))
const Fleet = lazy(() => import('./pages/Fleet'))
const VMList = lazy(() => import('./pages/VMList'))
const VMDetails = lazy(() => import('./pages/VMDetails'))
const CreateVM = lazy(() => import('./pages/CreateVM'))
const PassthroughDevices = lazy(() => import('./pages/PassthroughDevices'))
const Console = lazy(() => import('./pages/Console'))
const Snapshots = lazy(() => import('./pages/Snapshots'))
const FavoriteVMs = lazy(() => import('./pages/FavoriteVMs'))
const KrytonWindows = lazy(() => import('./pages/KrytonWindows'))
const GoldenImages = lazy(() => import('./pages/GoldenImages'))
const Migrations = lazy(() => import('./pages/Migrations'))
const VMwareImports = lazy(() => import('./pages/VMwareImports'))
const RookStorage = lazy(() => import('./pages/RookStorage'))
const Quotas = lazy(() => import('./pages/Quotas'))
const VMCompare = lazy(() => import('./pages/VMCompare'))
const BatchImport = lazy(() => import('./pages/BatchImport'))
const MigrationReadiness = lazy(() => import('./pages/MigrationReadiness'))
const EventStream = lazy(() => import('./pages/EventStream'))
const VMHealthCheck = lazy(() => import('./pages/VMHealthCheck'))
const ServiceMap = lazy(() => import('./pages/ServiceMap'))
const Backups = lazy(() => import('./pages/Backups'))
const DiskImages = lazy(() => import('./pages/DiskImages'))
const BackupScheduler = lazy(() => import('./pages/BackupScheduler'))
const PlacementAdvisor = lazy(() => import('./pages/PlacementAdvisor'))
const HaPolicy = lazy(() => import('./pages/HaPolicy'))
const StorageVolumes = lazy(() => import('./pages/Storage'))
const NetworkPolicies = lazy(() => import('./pages/NetworkPolicies'))
const ComplianceDashboard = lazy(() => import('./pages/ComplianceDashboard'))
const SecurityDashboard = lazy(() => import('./pages/SecurityDashboard'))
const CostEstimator = lazy(() => import('./pages/CostEstimator'))
const CapacityPlanning = lazy(() => import('./pages/CapacityPlanning'))
const Zones = lazy(() => import('./pages/Zones'))
const Analytics = lazy(() => import('./pages/Analytics'))
const ResourceOptimizer = lazy(() => import('./pages/ResourceOptimizer'))
const Templates = lazy(() => import('./pages/Templates'))
const Schedules = lazy(() => import('./pages/Schedules'))
const Webhooks = lazy(() => import('./pages/Webhooks'))
const Alerts = lazy(() => import('./pages/Alerts'))
const WarmPools = lazy(() => import('./pages/WarmPools'))
const AccessControl = lazy(() => import('./pages/AccessControl'))
const Pods = lazy(() => import('./pages/Pods'))
const PodLogsPage = lazy(() => import('./pages/PodLogsPage'))
const DataBridge = lazy(() => import('./pages/DataBridge'))

function ProtectedRoute({ children }: { children: ReactNode }) {
  const { isAuthenticated, loading } = useAuth()

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-[var(--zf-canvas)]">
        <div className="flex flex-col items-center gap-3">
          <div className="w-7 h-7 border-2 border-[var(--zf-ink)] border-t-transparent rounded-full animate-spin" />
          <span className="text-sm font-medium text-[var(--zf-muted)] tracking-wide">Loading…</span>
        </div>
      </div>
    )
  }

  if (!isAuthenticated) {
    return <Navigate to="/sign-in" replace />
  }

  return <>{children}</>
}

/** First admin-gated route in the app -- everything else here only checks
 * `canWrite` (read vs. write) via usePermissions(), never `canAdmin`. The
 * backend independently enforces this too (every /api/v1/users handler
 * requires Role::Admin), so this is UX, not the actual security boundary. */
function AdminRoute({ children }: { children: ReactNode }) {
  const { canAdmin } = usePermissions()

  if (!canAdmin) {
    return (
      <div className="flex items-center justify-center h-64 text-[var(--zf-muted)]">
        <p className="text-sm">Admins only.</p>
      </div>
    )
  }

  return <>{children}</>
}

function ConsoleRoutes() {
  return (
    <ConsoleLayout>
      <PageErrorBoundary>
        <Suspense fallback={<PageSkeleton />}>
          <Routes>
            <Route index element={<Dashboard />} />
            <Route path="fleet" element={<AdminRoute><Fleet /></AdminRoute>} />
            <Route path="vms" element={<VMList />} />
            <Route path="vms/:name" element={<VMDetails />} />
            <Route path="vms/:name/console" element={<Console />} />
            <Route path="create" element={<CreateVM />} />
            <Route path="favorites" element={<FavoriteVMs />} />
            <Route path="snapshots" element={<Snapshots />} />
            <Route path="migrations" element={<Migrations />} />
            <Route path="vmware-imports" element={<AdminRoute><VMwareImports /></AdminRoute>} />
            <Route path="migrations/readiness" element={<MigrationReadiness />} />
            <Route path="events" element={<EventStream />} />
            <Route path="health-check" element={<VMHealthCheck />} />
            <Route path="service-map" element={<ServiceMap />} />
            <Route path="backups" element={<Backups />} />
            <Route path="disk-images" element={<DiskImages />} />
            <Route path="golden-images" element={<GoldenImages />} />
            <Route path="backup-scheduler" element={<BackupScheduler />} />
            <Route path="placement" element={<PlacementAdvisor />} />
            <Route path="ha-policy" element={<HaPolicy />} />
            <Route path="volumes" element={<StorageVolumes />} />
            <Route path="network-policies" element={<NetworkPolicies />} />
            <Route path="compliance" element={<ComplianceDashboard />} />
            <Route path="security" element={<SecurityDashboard />} />
            <Route path="cost-estimator" element={<CostEstimator />} />
            <Route path="capacity" element={<CapacityPlanning />} />
            <Route path="zones" element={<Zones />} />
            <Route path="analytics" element={<Analytics />} />
            <Route path="optimizer" element={<ResourceOptimizer />} />
            <Route path="templates" element={<Templates />} />
            <Route path="schedules" element={<Schedules />} />
            <Route path="webhooks" element={<Webhooks />} />
            <Route path="alerts" element={<Alerts />} />
            <Route path="warm-pools" element={<WarmPools />} />
            <Route path="storage" element={<RookStorage />} />
            <Route path="databridge" element={<DataBridge />} />
            <Route path="quotas" element={<Quotas />} />
            <Route path="compare" element={<VMCompare />} />
            <Route path="batch-import" element={<BatchImport />} />
            <Route path="windows" element={<KrytonWindows />} />
            <Route path="access-control" element={<AdminRoute><AccessControl /></AdminRoute>} />
            <Route path="devices" element={<AdminRoute><PassthroughDevices /></AdminRoute>} />
            <Route path="pods" element={<AdminRoute><Pods /></AdminRoute>} />
            <Route path="pods/:ns/:name/logs" element={<AdminRoute><PodLogsPage /></AdminRoute>} />
            <Route path="*" element={<Navigate to="/app" replace />} />
          </Routes>
        </Suspense>
      </PageErrorBoundary>
    </ConsoleLayout>
  )
}

function LegacyRedirect() {
  const { pathname } = useLocation()
  if (pathname === '/' || pathname === '') return <Navigate to="/app" replace />
  if (pathname.startsWith('/app')) return <Navigate to="/app" replace />
  return <Navigate to={`/app${pathname}`} replace />
}

function AppRoutes() {
  return (
    <Routes>
      <Route path="/" element={<Home />} />
      <Route path="/product" element={<Product />} />
      <Route path="/platform" element={<Platform />} />
      <Route path="/security" element={<SecurityPage />} />
      <Route path="/sign-in" element={<SignIn />} />
      <Route path="/login" element={<Navigate to="/sign-in" replace />} />
      <Route
        path="/app/*"
        element={
          <ProtectedRoute>
            <ConsoleRoutes />
          </ProtectedRoute>
        }
      />
      <Route path="/vms/*" element={<LegacyRedirect />} />
      <Route path="/vms" element={<Navigate to="/app/vms" replace />} />
      <Route path="/create" element={<Navigate to="/app/create" replace />} />
      <Route path="/favorites" element={<Navigate to="/app/favorites" replace />} />
      <Route path="/snapshots" element={<Navigate to="/app/snapshots" replace />} />
      <Route path="*" element={<NotFound />} />
    </Routes>
  )
}

function App() {
  return (
    <ThemeProvider>
      <AuthProvider>
        <ToastProvider>
          <WebSocketProvider>
            <PlatformInfoProvider>
              <BrowserRouter>
                <AppRoutes />
              </BrowserRouter>
            </PlatformInfoProvider>
          </WebSocketProvider>
        </ToastProvider>
      </AuthProvider>
    </ThemeProvider>
  )
}

export default App
