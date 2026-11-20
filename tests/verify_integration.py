#!/usr/bin/env python3
# Copyright 2026 Zyvor AI Labs · https://zyvor.dev
# SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
required = [
    ROOT/'src/kryton/mod.rs', ROOT/'src/kryton/client.rs', ROOT/'src/kryton/models.rs',
    ROOT/'src/api/http_server/web/kryton_handlers.rs', ROOT/'web/src/api/kryton.ts',
    ROOT/'web/src/pages/KrytonWindows.tsx', ROOT/'web/src/api/__tests__/kryton.test.ts',
    ROOT/'docs/KRYTON_INTEGRATION.md'
]
for path in required:
    assert path.exists() and path.stat().st_size > 50, f'missing/empty {path}'

client = (ROOT/'src/kryton/client.rs').read_text()
handlers = (ROOT/'src/api/http_server/web/kryton_handlers.rs').read_text()
models = (ROOT/'src/kryton/models.rs').read_text()
frontend = (ROOT/'web/src/api/kryton.ts').read_text()

envs = {'KRYTON_URL','KRYTON_TOKEN','KRYTON_PROJECT','KRYTON_TIMEOUT_SECS','KRYTON_TLS_INSECURE'}
assert envs <= set(re.findall(r'KRYTON_[A-Z_]+', client)), 'missing env configuration'

upstream_paths = {
    '/api/v1/capabilities','/api/v1/doctor','/api/v1/images','/api/v1/summary','/api/v1/machines'
}
for p in upstream_paths:
    assert p in client, f'missing upstream path {p}'

adapter_paths = [
    '/api/v1/kryton/status','/api/v1/kryton/images','/api/v1/kryton/summary','/api/v1/kryton/machines'
]
for p in adapter_paths:
    assert p in frontend, f'missing frontend adapter path {p}'

for field in ['memoryMiB','sizeGiB','providerRef','ipAddresses','rdpHost','rdpPort','createdAt','updatedAt']:
    # serde rename_all covers most fields; fixture verifies exact JSON spellings.
    assert field in models, f'missing Kryton contract field/fixture {field}'

assert 'bearer_auth(token)' in client, 'Kryton token must be server-side bearer auth'
assert 'KRYTON_TOKEN' not in frontend, 'browser code must not know Kryton token'
assert 'KRYTON_TOKEN' not in handlers, 'handlers should use configured client, not raw token plumbing'
assert 'KRYTON_UNREACHABLE' in handlers and 'BAD_GATEWAY' in handlers

print('PASS: Zorvia/Kryton integration static contract checks')


apply_script = (ROOT/'integration/apply.py').read_text() if (ROOT/'integration/apply.py').exists() else ''
if apply_script:
    assert "deploy/k8s.yaml" in apply_script and "deploy/k3s-zorvia-web.yaml" in apply_script
    assert ".github/workflows/ci.yml" in apply_script
    assert "npm run typecheck" in apply_script and "npm run lint" in apply_script

status_handlers = (ROOT/'src/api/http_server/web/kryton_handlers.rs').read_text()
assert '"baseUrl"' not in status_handlers, 'do not expose internal Kryton URL to browser status payload'
assert 'Result<Snapshot, Error>' in client, 'restore snapshot should match Kryton Snapshot response contract'
