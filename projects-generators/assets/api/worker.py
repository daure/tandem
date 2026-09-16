import json
import os
import time

from store import connect


if os.getenv("FAIL_WORKER") == "1":
    raise SystemExit("Injected worker failure")

with connect() as client:
    while True:
        client.set("worker:alive", "ready", ex=5)
        item = client.blpop("pending", timeout=1)
        if item is None:
            continue
        identifier = item[1]
        job = json.loads(client.get(f"job:{identifier}"))
        time.sleep(0.5)
        job.update(status="done", result=f"Delivered: {job['message']}")
        client.set(f"job:{identifier}", json.dumps(job))
        print(f"Delivered {identifier}", flush=True)
