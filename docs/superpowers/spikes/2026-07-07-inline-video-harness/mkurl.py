#!/usr/bin/env python3
"""Build spike.html URL for given channel names (reads url_<name>.txt)."""
import base64
import json
import os
import sys

here = os.path.dirname(os.path.abspath(__file__))
streams = []
for name in sys.argv[1:]:
    with open(os.path.join(here, f"url_{name}.txt")) as f:
        streams.append({"name": name, "url": f.read().strip()})
b64 = base64.b64encode(json.dumps(streams).encode()).decode()
print(f"http://127.0.0.1:8807/spike.html?streams={b64}")
