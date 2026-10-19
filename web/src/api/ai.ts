// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: Apache-2.0

import { apiPost } from './client'
import { API_BASE_URL } from './config'

export interface AiDiagnosis {
  vm: string
  model: string
  /** Markdown from the model. */
  diagnosis: string
  /** The exact redacted evidence that was sent to the provider. */
  evidence_sent: unknown
}

export async function diagnoseVm(name: string): Promise<AiDiagnosis> {
  return apiPost<AiDiagnosis>(`${API_BASE_URL}/vms/${encodeURIComponent(name)}/ai/diagnose`)
}
