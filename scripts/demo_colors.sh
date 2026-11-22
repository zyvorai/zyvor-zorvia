#!/bin/bash
# Copyright 2026 Zyvor AI Labs · https://zyvor.dev
# SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

# Demo script to show Zorvia themed CLI output

echo "🎨 Zorvia CLI Theme Demo"
echo ""
echo "This demo shows the themed CLI output for various commands."
echo "Note: Some commands require a Kubernetes cluster with VMs."
echo ""

# Show available templates with colors
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "1. Templates (shows OS-specific colors)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
./target/release/zorvia template templates
echo ""

# Show validation message
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "2. Validation (shows success message)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if [ -f "examples/basic-vm.yaml" ]; then
    ./target/release/zorvia template validate examples/basic-vm.yaml
else
    echo "  (No example file found - would show: ✓ Configuration is valid)"
fi
echo ""

# Show help to display all colors
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "3. Help Menu"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
./target/release/zorvia --help
echo ""

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "To see more themed output, try:"
echo "  • zorvia vm list             (requires cluster)"
echo "  • zorvia vm get <vm-name>    (requires cluster)"
echo "  • zorvia vm resources        (requires cluster)"
echo "  • zorvia vm wizard           (interactive)"
echo ""
echo "All success messages (✓) are shown in GREEN"
echo "All error messages (✗) are shown in RED"
echo "All info messages (ℹ) are shown in BLUE"
echo "VM status symbols are color-coded by state"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
