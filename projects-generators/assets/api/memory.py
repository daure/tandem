import json
from pathlib import Path
from threading import Lock

message = None
lock = Lock()


def health():
    pass


def request(method, path, value):
    global message
    if path != "/greeting" or method not in {"GET", "PUT"}:
        return 404, {"error": "not found"}
    with lock:
        if method == "PUT":
            message = value
        default = json.loads(Path(__file__).with_name("config.json").read_text())["greeting"]
        return 200, {"message": message if message is not None else default}
