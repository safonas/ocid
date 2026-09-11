#!/bin/sh
# Runs before removal. Stop the service; state in /var/lib/ocid is kept
# (deleted only on deb purge, see postremove).
set -e

if [ -d /run/systemd/system ]; then
    systemctl stop ocid || true
fi
