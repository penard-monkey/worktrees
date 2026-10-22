#!/usr/bin/env python3
"""Try to bind+listen ADDR PORT once and print the outcome. Spike probe."""
import socket, sys
a, p = sys.argv[1], int(sys.argv[2])
s = socket.socket(socket.AF_INET6 if ":" in a else socket.AF_INET)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
try: s.bind((a, p)); s.listen(); print(f"bind {a}:{p} OK")
except OSError as e: print(f"bind {a}:{p} -> {e!r}")
