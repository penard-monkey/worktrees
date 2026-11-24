#!/bin/bash
# Runs INSIDE the Linux test VM as ~/launch-app.sh (run.py copies it there):
# starts the worktrees AppImage inside the logged-in GNOME session. A bare
# launch over ssh has no display and panics "Failed to initialize GTK".
p=$(pgrep -u "$(id -un)" -x gsd-xsettings | head -1)
[ -n "$p" ] || { echo "no GNOME session (gsd-xsettings) for $(id -un)" >&2; exit 1; }
for v in DISPLAY WAYLAND_DISPLAY XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS XDG_SESSION_TYPE XAUTHORITY; do
  val=$(tr '\0' '\n' < "/proc/$p/environ" | grep "^$v=" | cut -d= -f2-)
  [ -n "$val" ] && export "$v=$val"
done
cd "$HOME" || exit 1
setsid nohup "$HOME/Applications/worktrees.AppImage" >/tmp/wt-app.log 2>&1 < /dev/null &
echo launched
