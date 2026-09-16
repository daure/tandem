import json
import os
import uuid

import redis


def connect():
    return redis.Redis.from_url(os.environ["REDIS_URL"], decode_responses=True,
                               socket_connect_timeout=3, socket_timeout=5)


def health():
    with connect() as client:
        client.ping()
        if not client.exists("worker:alive"):
            raise RuntimeError("worker heartbeat missing")


def request(method, path, value):
    with connect() as client:
        if method == "POST" and path == "/jobs":
            identifier = uuid.uuid4().hex
            job = {"id": identifier, "status": "queued", "message": value}
            with client.pipeline() as transaction:
                transaction.set(f"job:{identifier}", json.dumps(job))
                transaction.rpush("pending", identifier)
                transaction.execute()
            return 202, job
        if method == "GET" and path.startswith("/jobs/"):
            job = client.get(f"job:{path.removeprefix('/jobs/')}")
            if job:
                return 200, json.loads(job)
        return 404, {"error": "not found"}
