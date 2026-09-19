"""A local greeting program with no third-party dependencies."""


def greet(name):
    return f"Hello, {name.strip() or 'world'}!"


if __name__ == "__main__":
    import sys

    print(greet(" ".join(sys.argv[1:])))
