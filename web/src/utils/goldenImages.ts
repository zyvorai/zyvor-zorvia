// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import type { KrytonCapabilities, KrytonGoldenBuild, KrytonImage, KrytonStatus } from '../api/kryton'

export type WindowsBuilderState =
  | { kind: 'disabled' }
  | { kind: 'unreachable'; error?: string }
  | { kind: 'no-project' }
  | { kind: 'unavailable'; provider: string }
  | { kind: 'ready'; project: string }

/** Why the Windows golden builder can or can't be used from Zorvia. */
export function windowsBuilderState(status: KrytonStatus | null, caps: KrytonCapabilities | null): WindowsBuilderState {
  if (!status?.enabled) return { kind: 'disabled' }
  if (!status.connected) return { kind: 'unreachable', error: status.error }
  if (!caps?.goldenImages) return { kind: 'unavailable', provider: caps?.provider ?? 'unknown' }
  if (!status.project) return { kind: 'no-project' }
  return { kind: 'ready', project: status.project }
}

/** Kryton's golden installer is Windows-only; Linux images come from Zorvia imports. */
export function windowsGoldenImages(images: KrytonImage[]): KrytonImage[] {
  return images.filter((image) => image.os !== 'linux')
}

// Mirrors Kryton's GoldenBuildState and bootstrap states (internal/model/golden.go).
const BUSY_STATES = new Set(['starting', 'installing', 'sysprep', 'capturing'])

export function isGoldenBuildBusy(build: KrytonGoldenBuild): boolean {
  return BUSY_STATES.has(build.state) || build.bootstrapState === 'running'
}

/** Bootstrap imports a captured qcow2 into a CDI DataSource; Kryton rejects it before capture. */
export function canBootstrap(build: KrytonGoldenBuild): boolean {
  return build.state === 'ready' && build.bootstrapState !== 'running' && build.bootstrapState !== 'ready'
}
