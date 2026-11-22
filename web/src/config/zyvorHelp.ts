// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { ZYVOR_COPY, ZYVOR_URL } from '../components/ZyvorBrand'

export { ZYVOR_URL, ZYVOR_COPY }

export const ZYVOR_HELP = {
  platform: ZYVOR_URL,
  docs: 'https://zyvor.dev/docs',
  docsIntro: 'https://zyvor.dev/docs/intro',
  products: 'https://zyvor.dev/docs/products',
  contact: 'https://zyvor.dev/contact',
  demo: 'https://zyvor.dev/demo',
  suite: 'https://zyvor.dev/docs/intro#suite-product-guides',
  sales: 'mailto:sales@zyvor.dev',
  info: 'mailto:info@zyvor.dev',
} as const

export type ProductHelpMeta = {
  name: string
  tagline: string
  version: string
  productUrl: string
}

/** Primary product identity — Zorvia control plane. */
export const ZORVIA_HELP: ProductHelpMeta = {
  name: 'Zorvia',
  tagline: 'KubeVirt VMs for private cloud — one control plane',
  version: '0.2.0',
  productUrl: 'https://zyvor.dev',
}

/** Technical daemon / service name. */
export const ZORVIA_DAEMON = 'zorvia'

/** @deprecated Use ZORVIA_HELP — kept so existing imports keep working. */
export const ZYVOR_FABRIC_HELP = ZORVIA_HELP

/** @deprecated Use ZORVIA_DAEMON — kept so existing imports keep working. */
export const ZYVOR_FABRIC_DAEMON = ZORVIA_DAEMON
