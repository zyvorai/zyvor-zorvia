// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import eslint from '@eslint/js'
import tseslint from 'typescript-eslint'
import reactHooks from 'eslint-plugin-react-hooks'

export default tseslint.config(
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  {
    plugins: {
      'react-hooks': reactHooks,
    },
    rules: {
      // eslint-plugin-react-hooks' 'recommended' preset is its newer React
      // Compiler ruleset (purity/immutability/set-state-in-effect/etc,
      // mostly as errors) -- this codebase's existing useEffect
      // data-fetching patterns predate that and were never written against
      // it. Declare just the two classic hook-safety rules explicitly
      // instead of spreading a preset, since the "classic" preset's name
      // isn't stable across this plugin's major versions.
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
      '@typescript-eslint/no-explicit-any': 'off',
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrorsIgnorePattern: '^_' }],
    },
  },
  {
    ignores: ['dist/**', 'e2e/**', 'playwright.config.ts', 'playwright-report/**'],
  }
)
