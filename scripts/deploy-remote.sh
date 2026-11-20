#!/usr/bin/env bash
# Copyright 2026 Zyvor AI Labs · https://zyvor.dev
# SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

# Thin wrapper — canonical script lives at deploy/remote-deploy.sh
exec "$(cd "$(dirname "$0")/.." && pwd)/deploy/remote-deploy.sh" "$@"
