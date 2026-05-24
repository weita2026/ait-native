def create_app():
    from .app import create_app as _create_app

    return _create_app()


def main():
    from .app import main as _main

    return _main()


__all__ = ["create_app", "main"]
