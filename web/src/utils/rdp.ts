// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

/**
 * Build and download a minimal .rdp launcher file for the given host/port.
 * Opens directly in the user's native RDP client (mstsc on Windows,
 * Microsoft Remote Desktop on macOS, Remmina and most Linux clients) --
 * Zorvia doesn't proxy RDP itself, this just saves typing the address in.
 */
export function downloadRdpFile(host: string, port: number, username?: string, filename?: string): void {
  const lines = [`full address:s:${host}:${port}`]
  if (username) lines.push(`username:s:${username}`)
  const blob = new Blob([lines.join('\n') + '\n'], { type: 'application/rdp' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename ?? `${host.replace(/[^a-z0-9.-]/gi, '_')}-${port}.rdp`
  document.body.appendChild(a)
  a.click()
  document.body.removeChild(a)
  URL.revokeObjectURL(url)
}
