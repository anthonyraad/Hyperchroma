#!/usr/bin/env python3
"""Point Rust's WaitOnAddress import at kernelbase.dll.

Rust 1.78+ imports WaitOnAddress from api-ms-win-core-synch-l1-2-0.dll. That
name is an API set, not a file in System32. Cubase's scanner calls
SetDllDirectory on the plugin folder before LoadLibrary, does not resolve the
API set, and drops the plugin. kernelbase.dll is a real system DLL that
exports WaitOnAddress, WakeByAddressAll, and WakeByAddressSingle, and it is
already loaded in the scanner.

KERNEL32.dll is the wrong stand-in: on current Windows the export is only a
forwarder, and Wine's kernel32 does not export it at all.
"""

import sys
from pathlib import Path

OLD_NAMES = (
    b"api-ms-win-core-synch-l1-2-0.dll",
    b"KERNEL32.dll" + b"\0" * (32 - len(b"KERNEL32.dll")),
)
NEW = b"kernelbase.dll"


def main() -> None:
    path = Path(sys.argv[1])
    data = path.read_bytes()
    if b"kernelbase.dll" in data and all(name not in data for name in OLD_NAMES):
        print(f"{path} already imports from kernelbase.dll")
        return
    count = 0
    for old in OLD_NAMES:
        found = data.count(old)
        if not found:
            continue
        if len(NEW) > len(old):
            sys.exit("replacement DLL name is longer than the original")
        padded = NEW + b"\0" * (len(old) - len(NEW))
        data = data.replace(old, padded)
        count += found
    if count == 0:
        sys.exit(f"{path} does not import the sync API set")
    path.write_bytes(data)
    print(f"retargeted {count} import(s) in {path}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(f"usage: {sys.argv[0]} <plugin.dll>")
    main()
