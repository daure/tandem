import os

import psycopg


def connect():
    return psycopg.connect(os.environ["DATABASE_URL"], connect_timeout=3)


def health():
    with connect() as connection:
        connection.execute("SELECT message FROM greetings WHERE id = 1").fetchone()


def request(method, path, value):
    if path != "/greeting" or method not in {"GET", "PUT"}:
        return 404, {"error": "not found"}
    with connect() as connection:
        if method == "PUT":
            connection.execute("UPDATE greetings SET message = %s WHERE id = 1", (value,))
        row = connection.execute("SELECT message FROM greetings WHERE id = 1").fetchone()
        return 200, {"message": row[0]}
