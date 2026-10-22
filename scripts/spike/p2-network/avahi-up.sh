#!/bin/bash
# Start the system bus and avahi-daemon inside a throwaway container.
sed -i 's/^rlimit-nproc=.*/#&/' /etc/avahi/avahi-daemon.conf
mkdir -p /run/dbus; rm -f /run/dbus/pid
dbus-daemon --system --fork
avahi-daemon -D || { echo "avahi-daemon failed"; exit 1; }
sleep 1; avahi-daemon -c && echo "avahi up on $(hostname)"
