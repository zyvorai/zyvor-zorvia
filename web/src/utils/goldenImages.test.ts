// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from 'vitest'
import type { KrytonCapabilities, KrytonGoldenBuild, KrytonImage } from '../api/kryton'
import { canBootstrap, isGoldenBuildBusy, windowsBuilderState, windowsGoldenImages } from './goldenImages'

const caps = (goldenImages: boolean, provider = 'dockur'): KrytonCapabilities => ({
  provider, snapshots: false, networks: false, ttl: true, liveMigration: false, console: true, goldenImages,
})
const build = (over: Partial<KrytonGoldenBuild>): KrytonGoldenBuild => ({ id: 'gb-1', imageId: 'windows-11-pro', state: 'ready', ...over })

describe('windowsBuilderState', () => {
  it('reports disabled, unreachable and missing-project states', () => {
    expect(windowsBuilderState(null, null).kind).toBe('disabled')
    expect(windowsBuilderState({ enabled: true, connected: false, error: 'dial' }, null)).toEqual({ kind: 'unreachable', error: 'dial' })
    expect(windowsBuilderState({ enabled: true, connected: true, project: null }, caps(true)).kind).toBe('no-project')
  })

  it('is unavailable when the Kryton host has no golden builder', () => {
    expect(windowsBuilderState({ enabled: true, connected: true, project: 'default' }, caps(false, 'libvirt')))
      .toEqual({ kind: 'unavailable', provider: 'libvirt' })
  })

  it('is ready with a project and the builder', () => {
    expect(windowsBuilderState({ enabled: true, connected: true, project: 'default' }, caps(true)))
      .toEqual({ kind: 'ready', project: 'default' })
  })
})

describe('golden build actions', () => {
  it('allows bootstrap only for a captured build that is not already imported', () => {
    expect(canBootstrap(build({ state: 'ready' }))).toBe(true)
    expect(canBootstrap(build({ state: 'ready', bootstrapState: 'failed' }))).toBe(true)
    expect(canBootstrap(build({ state: 'capturing' }))).toBe(false)
    expect(canBootstrap(build({ state: 'ready', bootstrapState: 'running' }))).toBe(false)
    expect(canBootstrap(build({ state: 'ready', bootstrapState: 'ready' }))).toBe(false)
  })

  it('polls while installing or bootstrapping', () => {
    expect(isGoldenBuildBusy(build({ state: 'installing' }))).toBe(true)
    expect(isGoldenBuildBusy(build({ state: 'ready', bootstrapState: 'running' }))).toBe(true)
    expect(isGoldenBuildBusy(build({ state: 'failed' }))).toBe(false)
  })

  it('offers only Windows images to the builder', () => {
    const img = (id: string, os?: string) => ({ id, os } as KrytonImage)
    expect(windowsGoldenImages([img('windows-11-pro'), img('ubuntu-24.04', 'linux')]).map((i) => i.id)).toEqual(['windows-11-pro'])
  })
})
