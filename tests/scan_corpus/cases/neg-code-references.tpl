import os

password = os.environ["DB_PASSWORD"]
token = os.getenv("API_TOKEN")
secret = get_secret("app/db")


def check(password: str) -> bool:
    return password != ""
