#!/bin/sh
# Runs after removal. Reload systemd; drop node state only on deb purge
# (rpm uninstall keeps /var/lib/ocid — remove it by hand if desired).
set -e

if [ -d /run/systemd/system ]; then
    systemctl daemon-reload || true
fi

if [ "${1:-}" = "purge" ]; then
    rm -rf /var/lib/ocid
fi
