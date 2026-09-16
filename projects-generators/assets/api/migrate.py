import os
from pathlib import Path

import psycopg


if os.getenv("FAIL_MIGRATION") == "1":
    raise SystemExit("Injected migration failure")

with psycopg.connect(os.environ["DATABASE_URL"], connect_timeout=3) as connection:
    connection.execute(Path(__file__).with_name("schema.sql").read_text())
print("Greetings schema ready", flush=True)
