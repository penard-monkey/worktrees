#!/bin/bash
# Runs INSIDE the Linux test VM as ~/sess.sh (run.py copies it there):
# `~/sess.sh <cmd…>` runs <cmd> inside the logged-in GNOME session. An ssh
# login has no display; the session's DISPLAY, WAYLAND_DISPLAY, D-Bus address
# and mutter's XAUTHORITY are read from gsd-xsettings' environment, which
# every GNOME session runs. xdotool, xclip, gsettings and gdbus need them.
p=$(pgrep -u "$(id -un)" -x gsd-xsettings | head -1)
[ -n "$p" ] || { echo "no GNOME session (gsd-xsettings) for $(id -un)" >&2; exit 1; }
for v in DISPLAY WAYLAND_DISPLAY XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS XDG_SESSION_TYPE XAUTHORITY; do
  val=$(tr '\0' '\n' < "/proc/$p/environ" | grep "^$v=" | cut -d= -f2-)
  [ -n "$val" ] && export "$v=$val"
done
exec "$@"
