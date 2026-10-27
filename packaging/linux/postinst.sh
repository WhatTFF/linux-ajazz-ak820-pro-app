#!/bin/sh
# Shared by the .deb and .rpm post-install scriptlets.
#
# 1. Reload udev so the new hidraw rule applies to a keyboard that is already
#    plugged in (otherwise the user has to replug it).
# 2. Tell the user how to turn on clock autosync. The systemd units are
#    installed but deliberately not enabled: they talk to a USB device, which
#    is the user's decision, and enabling user units from a root scriptlet is
#    not reliable anyway.

if command -v udevadm >/dev/null 2>&1; then
  udevadm control --reload >/dev/null 2>&1 || true
  udevadm trigger --subsystem-match=hidraw --action=change >/dev/null 2>&1 || true
fi

cat <<'EOF'
AK820 Studio installed.

To keep the keyboard clock in sync automatically, run as your own user:
  systemctl --user daemon-reload
  systemctl --user enable --now ak820-clock.timer
  systemctl --user enable ak820-resume.service

Then unplug and replug the keyboard so the new permissions take effect.
EOF

exit 0
