// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import type { ITerminalOptions } from '@xterm/xterm'

/** macOS Terminal.app "Pro" profile: black canvas, Terminal.app ANSI palette. */
export const MAC_TERMINAL_THEME: NonNullable<ITerminalOptions['theme']> = {
  background: '#000000',
  foreground: '#f2f2f2',
  cursor: '#f2f2f2',
  cursorAccent: '#000000',
  selectionBackground: 'rgba(10, 132, 255, 0.4)',
  black: '#000000',
  red: '#c23621',
  green: '#25bc24',
  yellow: '#adad27',
  blue: '#492ee1',
  magenta: '#d338d3',
  cyan: '#33bbc8',
  white: '#cbcccd',
  brightBlack: '#818383',
  brightRed: '#fc391f',
  brightGreen: '#31e722',
  brightYellow: '#eaec23',
  brightBlue: '#5833ff',
  brightMagenta: '#f935f8',
  brightCyan: '#14f0f0',
  brightWhite: '#e9ebeb',
}

export const MAC_TERMINAL_FONT = "'SF Mono', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace"

export const MAC_TERMINAL_OPTIONS: ITerminalOptions = {
  theme: MAC_TERMINAL_THEME,
  fontFamily: MAC_TERMINAL_FONT,
  fontSize: 12.5,
  lineHeight: 1.2,
  letterSpacing: 0,
  macOptionIsMeta: true,
  allowProposedApi: true,
}

/** Hex versions of the bright Terminal.app palette for DOM-rendered terminal views. */
export const TERM_HEX = {
  fg: '#f2f2f2',
  dim: '#818383',
  red: '#fc391f',
  green: '#31e722',
  yellow: '#eaec23',
  blue: '#5ac8fa',
  magenta: '#f935f8',
  cyan: '#14f0f0',
  orange: '#ff9f0a',
} as const

export interface TermSegment {
  text: string
  color?: string
}

const YAML_SCALAR = /^(-?\d+(\.\d+)?([eE][+-]?\d+)?|true|false|null|~)$/

function yamlScalar(value: string): TermSegment[] {
  if (!value) return []
  const comment = value.match(/^(.*?)(\s+#.*)$/)
  const body = comment ? comment[1] : value
  const segs: TermSegment[] = []
  const trimmed = body.trim()
  if (/^["']/.test(trimmed)) segs.push({ text: body, color: TERM_HEX.green })
  else if (YAML_SCALAR.test(trimmed)) {
    const isNum = /^-?\d/.test(trimmed)
    segs.push({ text: body, color: isNum ? TERM_HEX.magenta : TERM_HEX.yellow })
  } else if (/^[|>][-+]?$/.test(trimmed)) segs.push({ text: body, color: TERM_HEX.dim })
  else segs.push({ text: body, color: TERM_HEX.fg })
  if (comment) segs.push({ text: comment[2], color: TERM_HEX.dim })
  return segs
}

/** Colored segments for one YAML line (keys cyan, strings green, numbers magenta, literals yellow). */
export function yamlLineSegments(line: string): TermSegment[] {
  if (/^\s*#/.test(line)) return [{ text: line, color: TERM_HEX.dim }]
  if (/^---|^\.\.\./.test(line)) return [{ text: line, color: TERM_HEX.dim }]
  const m = line.match(/^(\s*)(- )?([^\s:#"'][^:#]*?|"[^"]*"|'[^']*')(:)(\s+.*|$)/)
  if (m) {
    const [, indent, dash, key, colon, rest] = m
    const segs: TermSegment[] = [{ text: indent }]
    if (dash) segs.push({ text: dash, color: TERM_HEX.dim })
    segs.push({ text: key, color: TERM_HEX.cyan }, { text: colon, color: TERM_HEX.dim })
    const lead = rest.match(/^\s*/)?.[0] ?? ''
    if (lead) segs.push({ text: lead })
    segs.push(...yamlScalar(rest.slice(lead.length)))
    return segs
  }
  const item = line.match(/^(\s*)(- )(.*)$/)
  if (item) return [{ text: item[1] }, { text: item[2], color: TERM_HEX.dim }, ...yamlScalar(item[3])]
  const lead = line.match(/^\s*/)?.[0] ?? ''
  return [...(lead ? [{ text: lead }] : []), ...yamlScalar(line.slice(lead.length))]
}

const ESC = '\x1b['
const RESET = `${ESC}0m`
const paint = (code: string, s: string) => `${ESC}${code}m${s}${RESET}`

const DIM = '90'
const KEY = '36'
const STR = '32'
const NUM = '35'
const LIT = '33'
const METHOD = '1;34'

function levelCode(level: string): string {
  const l = level.toLowerCase()
  if (/^(fatal|panic|crit|critical|error|err|e)$/.test(l)) return '1;31'
  if (/^(warn|warning|w)$/.test(l)) return '1;33'
  if (/^(info|notice|i)$/.test(l)) return '1;36'
  return DIM
}

function statusCode(code: string): string {
  const c = code[0]
  if (c === '2') return '32'
  if (c === '3') return '36'
  if (c === '4') return '33'
  return '1;31'
}

function colorizeJson(line: string): string {
  return line.replace(
    /("(?:\\.|[^"\\])*")(\s*:)?|\b(true|false|null)\b|(-?\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b)/g,
    (m, str: string | undefined, colon: string | undefined, lit: string | undefined, num: string | undefined) => {
      if (str !== undefined) {
        if (colon) return paint(KEY, str) + colon
        if (/^"(fatal|panic|error|err|warn|warning|info|debug|trace)"$/i.test(str)) {
          return paint(levelCode(str.slice(1, -1)), str)
        }
        return paint(STR, str)
      }
      if (lit !== undefined) return paint(LIT, lit)
      if (num !== undefined) return paint(NUM, num)
      return m
    },
  )
}

const LEVEL_WORDS = 'FATAL|PANIC|CRITICAL|CRIT|ERROR|ERR|WARNING|WARN|INFO|NOTICE|DEBUG|TRACE'

function colorizeText(line: string): string {
  const http = /\b(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\b|HTTP\/\d/.test(line)
  const re = new RegExp(
    [
      `\\b(?<lk>level|lvl|severity)=(?<lq>"?)(?<lv>[A-Za-z]+)\\k<lq>`,
      `\\b(?<level>${LEVEL_WORDS})\\b`,
      `\\b(?<key>[A-Za-z_][\\w.-]*)=`,
      `(?<str>"(?:\\\\.|[^"\\\\])*")`,
      `\\b(?<method>GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\\b`,
      ...(http ? ['(?<=\\s)(?<status>[1-5]\\d{2})(?=\\s|$)'] : []),
    ].join('|'),
    'g',
  )
  return line.replace(re, (...args) => {
    const m = args[0] as string
    const g = args[args.length - 1] as Record<string, string | undefined>
    if (g.lk !== undefined && g.lv !== undefined) {
      return `${paint(KEY, g.lk)}=${g.lq}${paint(levelCode(g.lv), g.lv)}${g.lq}`
    }
    if (g.level !== undefined) return paint(levelCode(g.level), g.level)
    if (g.key !== undefined) return paint(KEY, g.key) + '='
    if (g.str !== undefined) {
      const req = /^"(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS) /.exec(g.str)
      if (req) return `"${paint(METHOD, req[1])}${paint(STR, g.str.slice(req[0].length - 1))}`
      return paint(STR, g.str)
    }
    if (g.method !== undefined) return paint(METHOD, g.method)
    if (g.status !== undefined) return paint(statusCode(g.status), g.status)
    return m
  })
}

/**
 * Terminal.app-style colors for plain log lines. Lines that already carry
 * ANSI escapes are passed through untouched.
 */
export function colorizeLogLine(line: string): string {
  if (line.includes('\x1b[')) return line
  let prefix = ''
  let rest = line

  const ts = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?)(\s+)/.exec(rest)
  if (ts) {
    prefix += paint(DIM, ts[1]) + ts[2]
    rest = rest.slice(ts[0].length)
  }

  const klog = /^([IWEF])(\d{4} \d{2}:\d{2}:\d{2}\.\d+)(\s+)/.exec(rest)
  if (klog) {
    prefix += paint(levelCode(klog[1]), klog[1]) + paint(DIM, klog[2]) + klog[3]
    rest = rest.slice(klog[0].length)
  }

  const trimmed = rest.trimStart()
  const body = trimmed.startsWith('{') || trimmed.startsWith('[') ? colorizeJson(rest) : colorizeText(rest)
  return prefix + body
}
