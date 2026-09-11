#!/bin/sh
# Runs after install/upgrade. Reload systemd so the new unit is known.
# The service is intentionally NOT auto-enabled: run
# `systemctl enable --now ocid` once (state lives in /var/lib/ocid).
set -e

if [ -d /run/systemd/system ]; then
    systemctl daemon-reload || true
fi
