// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { formatUserError } from './apiError'

type ToastLike = {
  error: (message: string, duration?: number) => string
}

/** Show a toast with a sanitized API/daemon error message.
 *
 * Also logs to the console: a real (still-unconfirmed) bug can make
 * error-type toasts render nothing visible in some sessions -- this way a
 * failure is never *completely* silent even if that happens again. */
export function toastFailure(toast: ToastLike, label: string, e: unknown): void {
  const message = `${label}: ${formatUserError(e)}`
  console.error(message, e)
  toast.error(message)
}
