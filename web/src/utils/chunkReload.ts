// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

/**
 * A tab that stays open across a deploy still holds the old index.html, which
 * points at content-hashed chunk filenames the server no longer serves once the
 * new build lands -- any further `import()` for a route not yet visited 404s.
 * Reloading fetches the current index.html and fixes it, so treat this one error
 * shape as self-healing rather than a real crash.
 */
const CHUNK_LOAD_ERROR_RE =
  /failed to fetch dynamically imported module|error loading dynamically imported module|importing a module script failed/i

export function isChunkLoadError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error ?? '')
  return CHUNK_LOAD_ERROR_RE.test(message)
}

const RELOAD_FLAG_KEY = 'zf-chunk-reload-attempted'

/**
 * Reload once per tab session to recover from a stale-chunk failure. Guarded by
 * sessionStorage (not cleared) so a persistent failure -- a real outage, not a
 * stale deploy -- surfaces as a normal error instead of reload-looping the tab.
 */
export function reloadOnceForChunkError(): boolean {
  if (window.sessionStorage.getItem(RELOAD_FLAG_KEY)) {
    return false
  }
  window.sessionStorage.setItem(RELOAD_FLAG_KEY, '1')
  window.location.reload()
  return true
}
