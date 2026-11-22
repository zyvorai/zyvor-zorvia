// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { describe, expect, it } from 'vitest'
import { colorizeLogLine, TERM_HEX, yamlLineSegments } from './terminalTheme'

describe('yamlLineSegments', () => {
  const join = (line: string) => yamlLineSegments(line).map((s) => s.text).join('')
  const colorOf = (line: string, text: string) => yamlLineSegments(line).find((s) => s.text === text)?.color

  it('never changes the visible text', () => {
    for (const l of [
      'apiVersion: v1',
      '  name: zorvia-api # comment',
      '  - name: web',
      '    - --flag',
      '  replicas: 3',
      '  ready: true',
      '  msg: "quoted: value"',
      '  script: |',
      '---',
      '# top',
      '',
    ]) {
      expect(join(l)).toBe(l)
    }
  })

  it('colors keys, strings, numbers and literals', () => {
    expect(colorOf('  name: web', 'name')).toBe(TERM_HEX.cyan)
    expect(colorOf('  replicas: 3', '3')).toBe(TERM_HEX.magenta)
    expect(colorOf('  ready: true', 'true')).toBe(TERM_HEX.yellow)
    expect(colorOf('  msg: "a: b"', '"a: b"')).toBe(TERM_HEX.green)
    expect(colorOf('  name: web # why', ' # why')).toBe(TERM_HEX.dim)
  })

  it('handles list items with and without keys', () => {
    expect(colorOf('  - name: web', '- ')).toBe(TERM_HEX.dim)
    expect(colorOf('  - name: web', 'name')).toBe(TERM_HEX.cyan)
    expect(colorOf('    - --flag', '--flag')).toBe(TERM_HEX.fg)
  })
})

const ESC = '\x1b['
const paint = (code: string, s: string) => `${ESC}${code}m${s}${ESC}0m`
const SGR = new RegExp(`${String.fromCharCode(27)}\\[[0-9;]*m`, 'g')
const strip = (s: string) => s.replace(SGR, '')

describe('colorizeLogLine', () => {
  it('never changes the visible text', () => {
    const lines = [
      '2026-09-27T01:25:45.903Z INFO zorvia::api: listening on 0.0.0.0:5151',
      'I0927 01:25:45.123456       1 leader.go:42] acquired lease',
      '{"level":"error","msg":"boom","code":500,"ok":false,"extra":null}',
      'time="2026" level=warn msg="disk low" free=12',
      '10.0.0.1 - - "GET /api/v1/health HTTP/1.1" 200 12',
      'plain text with no structure',
      '',
    ]
    for (const l of lines) expect(strip(colorizeLogLine(l))).toBe(l)
  })

  it('passes through lines that already carry ANSI', () => {
    const line = `${ESC}32mgreen${ESC}0m already colored`
    expect(colorizeLogLine(line)).toBe(line)
  })

  it('dims a leading RFC 3339 timestamp', () => {
    const out = colorizeLogLine('2026-09-27T01:25:45.903+02:00 hello')
    expect(out.startsWith(paint('90', '2026-09-27T01:25:45.903+02:00'))).toBe(true)
  })

  it('colors uppercase levels by severity', () => {
    expect(colorizeLogLine('ERROR failed')).toContain(paint('1;31', 'ERROR'))
    expect(colorizeLogLine('WARN slow')).toContain(paint('1;33', 'WARN'))
    expect(colorizeLogLine('INFO ok')).toContain(paint('1;36', 'INFO'))
    expect(colorizeLogLine('DEBUG x')).toContain(paint('90', 'DEBUG'))
  })

  it('colors klog headers by their level letter', () => {
    const out = colorizeLogLine('E0927 01:25:45.123456 1 x.go:1] bad')
    expect(out.startsWith(paint('1;31', 'E') + paint('90', '0927 01:25:45.123456'))).toBe(true)
  })

  it('colors logfmt level= values and keys', () => {
    const out = colorizeLogLine('level=error msg="x"')
    expect(out).toContain(`${paint('36', 'level')}=${paint('1;31', 'error')}`)
    expect(out).toContain(`${paint('36', 'msg')}=`)
    expect(out).toContain(paint('32', '"x"'))
  })

  it('keeps quotes around quoted level values', () => {
    expect(colorizeLogLine('level="warn"')).toContain(`${paint('36', 'level')}="${paint('1;33', 'warn')}"`)
  })

  it('tokenizes JSON lines', () => {
    const out = colorizeLogLine('{"msg":"hi","n":-1.5e3,"ok":true,"level":"warn"}')
    expect(out).toContain(paint('36', '"msg"') + ':')
    expect(out).toContain(paint('32', '"hi"'))
    expect(out).toContain(paint('35', '-1.5e3'))
    expect(out).toContain(paint('33', 'true'))
    expect(out).toContain(paint('1;33', '"warn"'))
  })

  it('colors HTTP methods and status codes on HTTP lines', () => {
    const out = colorizeLogLine('"GET /x HTTP/1.1" 200 5 then 404 and 503')
    expect(out).toContain(paint('1;34', 'GET'))
    expect(out).toContain(paint('32', '200'))
    expect(out).toContain(paint('33', '404'))
    expect(out).toContain(paint('1;31', '503'))
  })

  it('highlights the method inside a quoted access-log request line', () => {
    const out = colorizeLogLine('1.2.3.4 "POST /login HTTP/2" 302')
    expect(out).toContain(`"${paint('1;34', 'POST')}${paint('32', ' /login HTTP/2"')}`)
    expect(out).toContain(paint('36', '302'))
  })

  it('leaves bare numbers alone on non-HTTP lines', () => {
    expect(colorizeLogLine('processed 200 items')).toBe('processed 200 items')
  })
})
