// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import { reloadOnceForChunkError } from './utils/chunkReload'
import '@fontsource-variable/inter'
import './styles/main.css'

// Vite fires this for a failed modulepreload -- same stale-deploy cause as the
// dynamic-import errors ErrorBoundary catches, just outside a render boundary.
window.addEventListener('vite:preloadError', () => {
  reloadOnceForChunkError()
})

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
